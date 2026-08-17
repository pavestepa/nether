use std::collections::{HashMap, HashSet};

use nether_ast::{
    BinaryOp, Block, EnumDecl, Expr, ExprKind, FnDecl, Ident, ImplBlock, TraitDecl, Item,
    Literal, Module, NodeId, Path, Pattern, SelfParam, Stmt, Symbol, TemplatePart, StructDecl,
    StructDeclKind, TypeAliasDecl, TypeExpr, UnaryOp,
};
use nether_diagnostics::{Diagnostic, Span};
use nether_resolver::{DefId, DefKind, LocalId, Resolution, ResolvedNames};

use crate::sig::{EnumSig, FnSig, GenericBound, MethodSet, ReceiverDomain, Signatures, TypeShape};
use crate::ty::{PrimitiveKind, Type};

mod call;
mod casing;
mod checker;
mod construct;
mod control;
mod declarations;
mod entry;
mod expr;
mod traits;
mod layout;
mod method;

use casing::validate_alias_casing;
use checker::Checker;
use declarations::*;
use entry::{
    collect_generic_bindings, contextualize_unknowns, describe_type, prefer_concrete_type,
    substitute_generic,
};
use traits::*;
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
    type_decls: HashMap<DefId, &'a StructDecl>,
    enum_decls: HashMap<DefId, &'a EnumDecl>,
    trait_decls: HashMap<DefId, &'a TraitDecl>,
    type_aliases: HashMap<DefId, &'a TypeAliasDecl>,
}

fn index_decls<'a>(module: &'a Module, resolved: &ResolvedNames) -> DeclIndex<'a> {
    let mut idx = DeclIndex {
        type_decls: HashMap::new(),
        enum_decls: HashMap::new(),
        trait_decls: HashMap::new(),
        type_aliases: HashMap::new(),
    };
    for item in &module.items {
        match item {
            Item::Struct(t) => {
                if let Some(id) = resolved.definitions.lookup_in(t.span.file, &t.name.name) {
                    idx.type_decls.insert(id, t);
                }
            }
            Item::Enum(e) => {
                if let Some(id) = resolved.definitions.lookup_in(e.span.file, &e.name.name) {
                    idx.enum_decls.insert(id, e);
                }
            }
            Item::Trait(i) => {
                if let Some(id) = resolved.definitions.lookup_in(i.span.file, &i.name.name) {
                    idx.trait_decls.insert(id, i);
                }
            }
            Item::TypeAlias(a) => {
                if let Some(id) = resolved.definitions.lookup_in(a.span.file, &a.name.name) {
                    idx.type_aliases.insert(id, a);
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
/// than re-deriving name lookups itself. Public entry point — always
/// starts a fresh alias-cycle guard (see [`lower_type_expr_inner`]).
fn lower_type_expr(
    ty: &TypeExpr,
    resolved: &ResolvedNames,
    decls: &DeclIndex,
    diags: &mut Vec<Diagnostic>,
) -> Type {
    lower_type_expr_inner(ty, resolved, decls, diags, &mut HashSet::new())
}

/// `visiting` tracks which [`TypeAliasDecl`]s are currently being
/// substituted through, so a self-referential alias (`type A = B; type B
/// = A;`) produces a diagnostic in [`lower_named_type`] instead of
/// overflowing the stack — language-spec §4.3.
fn lower_type_expr_inner(
    ty: &TypeExpr,
    resolved: &ResolvedNames,
    decls: &DeclIndex,
    diags: &mut Vec<Diagnostic>,
    visiting: &mut HashSet<DefId>,
) -> Type {
    match ty {
        TypeExpr::Named { path, generics, .. } => {
            lower_named_type(path, generics, resolved, decls, diags, visiting)
        }
        TypeExpr::Tuple(elems, _) => Type::Tuple(
            elems
                .iter()
                .map(|e| lower_type_expr_inner(e, resolved, decls, diags, visiting))
                .collect(),
        ),
        TypeExpr::Array(inner, _) => Type::Array(Box::new(lower_type_expr_inner(
            inner, resolved, decls, diags, visiting,
        ))),
        TypeExpr::Weak(inner, span) => {
            let inner_ty = lower_type_expr_inner(inner, resolved, decls, diags, visiting);
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
        TypeExpr::Unique(inner, _) => Type::Unique(Box::new(lower_type_expr_inner(
            inner, resolved, decls, diags, visiting,
        ))),
        TypeExpr::Ref(inner, _) => Type::Ref(Box::new(lower_type_expr_inner(
            inner, resolved, decls, diags, visiting,
        ))),
        TypeExpr::MutRef(inner, _) => Type::MutRef(Box::new(lower_type_expr_inner(
            inner, resolved, decls, diags, visiting,
        ))),
        TypeExpr::Function { params, ret, .. } => Type::Function(
            params
                .iter()
                .map(|p| lower_type_expr_inner(p, resolved, decls, diags, visiting))
                .collect(),
            Box::new(lower_type_expr_inner(ret, resolved, decls, diags, visiting)),
        ),
    }
}

fn lower_named_type(
    path: &Path,
    generics: &[TypeExpr],
    resolved: &ResolvedNames,
    decls: &DeclIndex,
    diags: &mut Vec<Diagnostic>,
    visiting: &mut HashSet<DefId>,
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
                    Type::Array(Box::new(lower_type_expr_inner(
                        &generics[0],
                        resolved,
                        decls,
                        diags,
                        visiting,
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
                    // (`stdlib/option.nr`/`result.nt`), so they always
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
                        .map(|g| lower_type_expr_inner(g, resolved, decls, diags, visiting))
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
                        .map(|generic| {
                            lower_type_expr_inner(generic, resolved, decls, diags, visiting)
                        })
                        .collect();
                    match &decl.kind {
                        StructDeclKind::TupleStruct(_) => Type::TupleStruct(id, args),
                        _ => Type::Struct(id, args),
                    }
                }
                DefKind::Trait => {
                    diags.push(
                        Diagnostic::error(format!(
                            "`{name}` is a trait and cannot be used as a value type"
                        ))
                        .with_label(path.span, "use it as a generic bound instead"),
                    );
                    Type::Error
                }
                DefKind::TypeAlias => {
                    if !visiting.insert(id) {
                        diags.push(
                            Diagnostic::error(format!(
                                "type alias `{name}` is defined in terms of itself"
                            ))
                            .with_label(path.span, "cyclic alias"),
                        );
                        return Type::Error;
                    }
                    let result = match decls.type_aliases.get(&id) {
                        Some(alias_decl) => {
                            lower_type_expr_inner(&alias_decl.ty, resolved, decls, diags, visiting)
                        }
                        None => Type::Error,
                    };
                    visiting.remove(&id);
                    result
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
            Diagnostic::error("a generic bound must name a trait")
                .with_label(ty.span(), "not a trait"),
        );
        return None;
    };
    let id = type_expr_def_id(ty, resolved)?;
    if resolved.definitions.get(id).kind != DefKind::Trait {
        diags.push(
            Diagnostic::error(format!(
                "`{}` is not a trait",
                resolved.definitions.get(id).name
            ))
            .with_label(*span, "used as a bound here"),
        );
        return None;
    }
    let expected = decls
        .trait_decls
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
        trait_id: id,
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
