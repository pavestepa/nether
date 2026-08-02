use std::collections::{HashMap, HashSet};

use nether_ast::{
    BinaryOp, Block, EnumDecl, Expr, ExprKind, FnDecl, Ident, ImplBlock, InterfaceDecl, Item,
    Literal, Module, NodeId, Path, Pattern, SelfParam, Stmt, Symbol, TemplatePart, TypeDecl,
    TypeDeclKind, TypeExpr, UnaryOp,
};
use nether_diagnostics::{Diagnostic, Span};
use nether_resolver::{DefId, DefKind, Resolution, ResolvedNames};

use crate::sig::{EnumSig, FnSig, GenericBound, MethodSet, Signatures, TypeShape};
use crate::ty::{PrimitiveKind, Type};

mod call;
mod checker;
mod construct;
mod control;
mod declarations;
mod entry;
mod expr;
mod interfaces;
mod layout;
mod method;

use checker::Checker;
use declarations::*;
use entry::{
    collect_generic_bindings, contextualize_unknowns, describe_type, prefer_concrete_type,
    substitute_generic,
};
use interfaces::*;
use layout::*;

pub use entry::check;

/// Every result of one [`check`] call.
pub struct TypedTables {
    /// Keyed by every [`Expr`]'s own `id` — see `nether_ast::Expr` docs.
    pub expr_types: HashMap<NodeId, Type>,
    /// Keyed by a binding site's `id` — a [`FnDecl`]'s (for `self`), a
    /// [`nether_ast::Param`]'s, a [`nether_ast::LetStmt`]'s, or a
    /// [`Pattern::Binding`]'s — recording that local's final type, the
    /// same way [`nether_resolver::ResolvedNames::locals`] records its
    /// identity. `hir` cross-references this against `resolver`'s
    /// `locals` map to look up a local's type by `LocalId` when lowering
    /// a leftover path segment (language-spec §10) whose base is a local.
    pub local_types: HashMap<NodeId, Type>,
    /// Fully resolved generic arguments for each direct generic call,
    /// ordered like the callee signature's generic parameter list.
    pub call_generic_args: HashMap<NodeId, Vec<Type>>,
    pub signatures: Signatures,
}

/// A quick DefId → declaration-node index, built once at the start of
/// [`check`] by looking each top-level name up through
/// [`ResolvedNames::definitions`] — this sidesteps any need for this
/// crate's construction order to match `resolver`'s (see this crate's
/// module docs for why that would otherwise be a fragile coupling).
struct DeclIndex<'a> {
    type_decls: HashMap<DefId, &'a TypeDecl>,
    enum_decls: HashMap<DefId, &'a EnumDecl>,
    interface_decls: HashMap<DefId, &'a InterfaceDecl>,
}

fn index_decls<'a>(module: &'a Module, resolved: &ResolvedNames) -> DeclIndex<'a> {
    let mut idx = DeclIndex {
        type_decls: HashMap::new(),
        enum_decls: HashMap::new(),
        interface_decls: HashMap::new(),
    };
    for item in &module.items {
        match item {
            Item::Type(t) => {
                if let Some(id) = resolved.definitions.lookup_in(t.span.file, &t.name.name) {
                    idx.type_decls.insert(id, t);
                }
            }
            Item::Enum(e) => {
                if let Some(id) = resolved.definitions.lookup_in(e.span.file, &e.name.name) {
                    idx.enum_decls.insert(id, e);
                }
            }
            Item::Interface(i) => {
                if let Some(id) = resolved.definitions.lookup_in(i.span.file, &i.name.name) {
                    idx.interface_decls.insert(id, i);
                }
            }
            _ => {}
        }
    }
    idx
}

// ---------------------------------------------------------------------
// TypeExpr -> Type
// ---------------------------------------------------------------------

/// Converts a syntactic [`TypeExpr`] into a structured [`Type`], reading
/// the [`Resolution`] `resolver` already computed for its [`Path`] rather
/// than re-deriving name lookups itself.
fn lower_type_expr(
    ty: &TypeExpr,
    resolved: &ResolvedNames,
    decls: &DeclIndex,
    diags: &mut Vec<Diagnostic>,
) -> Type {
    match ty {
        TypeExpr::Named { path, generics, .. } => {
            lower_named_type(path, generics, resolved, decls, diags)
        }
        TypeExpr::Tuple(elems, _) => Type::Tuple(
            elems
                .iter()
                .map(|e| lower_type_expr(e, resolved, decls, diags))
                .collect(),
        ),
        TypeExpr::Array(inner, _) => {
            Type::Array(Box::new(lower_type_expr(inner, resolved, decls, diags)))
        }
        TypeExpr::Weak(inner, span) => {
            let inner_ty = lower_type_expr(inner, resolved, decls, diags);
            if !inner_ty.is_error()
                && crate::alloc::alloc_kind(&inner_ty, &resolved.definitions)
                    != crate::alloc::AllocKind::Heap
            {
                diags.push(
                    Diagnostic::error("`weak` can only wrap a heap-allocated type")
                        .with_label(*span, "this type is not heap-allocated"),
                );
            }
            Type::Weak(Box::new(inner_ty))
        }
        TypeExpr::Function { params, ret, .. } => Type::Function(
            params
                .iter()
                .map(|p| lower_type_expr(p, resolved, decls, diags))
                .collect(),
            Box::new(lower_type_expr(ret, resolved, decls, diags)),
        ),
    }
}

fn lower_named_type(
    path: &Path,
    generics: &[TypeExpr],
    resolved: &ResolvedNames,
    decls: &DeclIndex,
    diags: &mut Vec<Diagnostic>,
) -> Type {
    let Some(res) = resolved.path_res.get(&path.id) else {
        return Type::Error;
    };
    match res.base {
        Resolution::GenericParam => Type::Generic(path.segments[0].name.clone()),
        Resolution::Error => Type::Error,
        Resolution::Def(id) => {
            let def = resolved.definitions.get(id);
            let name = def.name.as_str();
            if name == "String" {
                return Type::String;
            }
            if name == "Array" {
                return if generics.len() == 1 {
                    Type::Array(Box::new(lower_type_expr(
                        &generics[0],
                        resolved,
                        decls,
                        diags,
                    )))
                } else {
                    diags.push(
                        Diagnostic::error("`Array` takes exactly one type argument")
                            .with_label(path.span, "here"),
                    );
                    Type::Error
                };
            }
            if let Some(prim) = PrimitiveKind::from_name(name) {
                return Type::Primitive(prim);
            }
            match def.kind {
                DefKind::Enum => {
                    // `Option`/`Result` are ordinary prelude `enum`s now
                    // (`stdlib/option.nt`/`result.nt`), so they always
                    // have a `decls.enum_decls` entry like any other enum
                    // — no builtin-arity fallback needed.
                    let expected = decls
                        .enum_decls
                        .get(&id)
                        .map(|decl| decl.generics.len())
                        .unwrap_or(0);
                    if generics.len() != expected {
                        diags.push(
                            Diagnostic::error(format!(
                                "`{name}` takes {expected} type argument(s), found {}",
                                generics.len()
                            ))
                            .with_label(path.span, "here"),
                        );
                        return Type::Error;
                    }
                    let args = generics
                        .iter()
                        .map(|g| lower_type_expr(g, resolved, decls, diags))
                        .collect();
                    Type::Enum(id, args)
                }
                DefKind::Type => {
                    let Some(decl) = decls.type_decls.get(&id) else {
                        if generics.is_empty() {
                            return Type::Struct(id, Vec::new());
                        }
                        diags.push(
                            Diagnostic::error(format!("`{name}` does not take type arguments"))
                                .with_label(path.span, "here"),
                        );
                        return Type::Error;
                    };
                    if generics.len() != decl.generics.len() {
                        diags.push(
                            Diagnostic::error(format!(
                                "`{name}` takes {} type argument(s), found {}",
                                decl.generics.len(),
                                generics.len()
                            ))
                            .with_label(path.span, "here"),
                        );
                        return Type::Error;
                    }
                    let args = generics
                        .iter()
                        .map(|generic| lower_type_expr(generic, resolved, decls, diags))
                        .collect();
                    match &decl.kind {
                        TypeDeclKind::TupleStruct(_) => Type::TupleStruct(id, args),
                        _ => Type::Struct(id, args),
                    }
                }
                DefKind::Interface => {
                    diags.push(
                        Diagnostic::error(format!(
                            "`{name}` is an interface and cannot be used as a value type"
                        ))
                        .with_label(path.span, "use it as a generic bound instead"),
                    );
                    Type::Error
                }
                DefKind::Fn | DefKind::Primitive | DefKind::Imported => Type::Error,
            }
        }
        // Local/EnumVariant/StaticMember never arise for a type-position
        // path (`resolver::resolve_type_path` only ever produces
        // GenericParam/Def/Error for these).
        _ => Type::Error,
    }
}

fn type_expr_def_id(ty: &TypeExpr, resolved: &ResolvedNames) -> Option<DefId> {
    if let TypeExpr::Named { path, .. } = ty {
        if let Some(res) = resolved.path_res.get(&path.id) {
            if let Resolution::Def(id) = res.base {
                return Some(id);
            }
        }
    }
    None
}

fn lower_generic_bound(
    ty: &TypeExpr,
    resolved: &ResolvedNames,
    decls: &DeclIndex,
    diags: &mut Vec<Diagnostic>,
) -> Option<GenericBound> {
    let TypeExpr::Named {
        path,
        generics,
        span,
    } = ty
    else {
        diags.push(
            Diagnostic::error("a generic bound must name an interface")
                .with_label(ty.span(), "not an interface"),
        );
        return None;
    };
    let id = type_expr_def_id(ty, resolved)?;
    if resolved.definitions.get(id).kind != DefKind::Interface {
        diags.push(
            Diagnostic::error(format!(
                "`{}` is not an interface",
                resolved.definitions.get(id).name
            ))
            .with_label(*span, "used as a bound here"),
        );
        return None;
    }
    let expected = decls
        .interface_decls
        .get(&id)
        .map(|decl| decl.generics.len())
        .unwrap_or_else(|| usize::from(resolved.definitions.get(id).name.as_str() == "Into"));
    if generics.len() != expected {
        diags.push(
            Diagnostic::error(format!(
                "`{}` takes {expected} type argument(s), found {}",
                resolved.definitions.get(id).name,
                generics.len()
            ))
            .with_label(path.span, "here"),
        );
        return None;
    }
    Some(GenericBound {
        interface: id,
        args: generics
            .iter()
            .map(|arg| lower_type_expr(arg, resolved, decls, diags))
            .collect(),
    })
}

// ---------------------------------------------------------------------
// Signature collection
// ---------------------------------------------------------------------

fn build_fn_sig(
    f: &FnDecl,
    resolved: &ResolvedNames,
    decls: &DeclIndex,
    diags: &mut Vec<Diagnostic>,
) -> FnSig {
    let generics = f
        .generics
        .iter()
        .map(|g| {
            (
                g.name.name.clone(),
                g.bound
                    .as_ref()
                    .and_then(|bound| lower_generic_bound(bound, resolved, decls, diags)),
            )
        })
        .collect();
    let params = f
        .params
        .iter()
        .map(|p| crate::sig::ParamSig {
            name: p.name.name.clone(),
            mutable: p.mutable,
            variadic: p.variadic,
            ty: lower_type_expr(&p.ty, resolved, decls, diags),
        })
        .collect();
    let ret = f
        .ret
        .as_ref()
        .map(|r| lower_type_expr(r, resolved, decls, diags))
        .unwrap_or_else(Type::unit);
    FnSig {
        self_param: f.self_param,
        params,
        ret,
        generics,
    }
}
