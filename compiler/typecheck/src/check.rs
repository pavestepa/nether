use std::collections::{HashMap, HashSet};

use nether_ast::{
    BinaryOp, Block, EnumDecl, Expr, ExprKind, FnDecl, Ident, ImplBlock, Item, Literal, Module,
    NodeId, Path, Pattern, SelfParam, Stmt, StructDecl, StructDeclKind, Symbol, TemplatePart,
    TraitDecl, TypeAliasDecl, TypeExpr, UnaryOp,
};
use nether_diagnostics::{Diagnostic, Span};
use nether_resolver::{DefId, DefKind, LocalId, Resolution, ResolvedNames};

use crate::sig::{
    AssociatedConstSig, AssociatedTypeSig, EnumSig, FnSig, GenericBound, MethodSet, ReceiverDomain,
    ReturnOrigin, Signatures, TraitAssociatedConstSig, TraitAssociatedTypeSig, TypeShape,
};
use crate::ty::{PrimitiveKind, Type};

mod call;
mod casing;
mod checker;
mod construct;
mod control;
mod declarations;
mod entry;
mod expr;
mod layout;
mod method;
mod traits;

use casing::validate_alias_casing;
use checker::{BorrowOrigin, Checker, ClosureOrigin};
use declarations::*;
pub(crate) use entry::substitute_generic;
use entry::{
    collect_generic_bindings, contextualize_unknowns, describe_type, prefer_concrete_type,
};
use layout::*;
use traits::*;

pub(super) fn call_result_type(sig: &FnSig, output: Type) -> Type {
    if sig.is_async {
        Type::Task(Box::new(output))
    } else {
        output
    }
}

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
    /// Concrete source type for each implicit `value -> any/some Trait` pack.
    pub existential_coercions: HashMap<NodeId, Type>,
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
        TypeExpr::Const(value, _) => Type::Const(*value),
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
        TypeExpr::FixedArray {
            element,
            length,
            span,
        } => {
            let element = lower_type_expr_inner(element, resolved, decls, diags, visiting);
            let length = lower_type_expr_inner(length, resolved, decls, diags, visiting);
            if !matches!(length, Type::Const(_) | Type::Generic(_) | Type::Error) {
                diags.push(
                    Diagnostic::error("fixed-array length must be a compile-time integer")
                        .with_label(*span, "invalid fixed-array length"),
                );
                Type::Error
            } else {
                Type::FixedArray(Box::new(element), Box::new(length))
            }
        }
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
        TypeExpr::Any(trait_ty, span) => {
            lower_existential_type(trait_ty, *span, false, resolved, decls, diags, visiting)
        }
        TypeExpr::Some(trait_ty, span) => {
            lower_existential_type(trait_ty, *span, true, resolved, decls, diags, visiting)
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

fn lower_existential_type(
    syntax: &TypeExpr,
    span: Span,
    opaque: bool,
    resolved: &ResolvedNames,
    decls: &DeclIndex,
    diags: &mut Vec<Diagnostic>,
    visiting: &mut HashSet<DefId>,
) -> Type {
    let TypeExpr::Named { path, generics, .. } = syntax else {
        diags.push(
            Diagnostic::error("`any`/`some` must be followed by a trait name")
                .with_label(span, "not a trait"),
        );
        return Type::Error;
    };
    let Some(resolution) = resolved.path_res.get(&path.id) else {
        return Type::Error;
    };
    let Resolution::Def(id) = resolution.base else {
        diags.push(
            Diagnostic::error("`any`/`some` must be followed by a trait name")
                .with_label(span, "not a trait"),
        );
        return Type::Error;
    };
    if resolved.definitions.get(id).kind != DefKind::Trait {
        diags.push(
            Diagnostic::error("`any`/`some` must be followed by a trait name")
                .with_label(span, "this is not a trait"),
        );
        return Type::Error;
    }
    fn validate_existential_safety(
        trait_id: DefId,
        resolved: &ResolvedNames,
        decls: &DeclIndex,
        diags: &mut Vec<Diagnostic>,
        visiting: &mut HashSet<DefId>,
    ) {
        if !visiting.insert(trait_id) {
            return;
        }
        let Some(declaration) = decls.trait_decls.get(&trait_id).copied() else {
            return;
        };
        for method in &declaration.methods {
            if matches!(
                method.self_param,
                Some(
                    nether_ast::SelfParam::Owned
                        | nether_ast::SelfParam::OwnedRef
                        | nether_ast::SelfParam::OwnedMutRef
                )
            ) {
                diags.push(
                    Diagnostic::error(format!(
                        "trait `{}` is not existential-safe: owned-domain receivers cannot be stored in a reusable witness package",
                        declaration.name.name
                    ))
                    .with_label(
                        method.name.span,
                        "owned-domain receivers cannot be stored in a reusable witness package",
                    ),
                );
            }
            if method.self_param.is_none() {
                diags.push(
                    Diagnostic::error(format!(
                        "trait `{}` is not existential-safe",
                        declaration.name.name
                    ))
                    .with_label(
                        method.name.span,
                        "static methods cannot be dispatched through `any`/`some`",
                    ),
                );
            }
            if !method.generics.is_empty() {
                diags.push(
                    Diagnostic::error(format!(
                        "trait `{}` is not existential-safe",
                        declaration.name.name
                    ))
                    .with_label(
                        method.name.span,
                        "generic methods cannot appear in an existential witness table",
                    ),
                );
            }
        }
        for associated in &declaration.associated_types {
            if associated.value.is_none() {
                diags.push(
                    Diagnostic::error(format!(
                        "trait `{}` is not existential-safe",
                        declaration.name.name
                    ))
                    .with_label(
                        associated.name.span,
                        "an existential associated type needs a default binding",
                    ),
                );
            }
        }
        for parent in &declaration.parents {
            if let Some(parent_id) = type_expr_def_id(parent, resolved) {
                validate_existential_safety(parent_id, resolved, decls, diags, visiting);
            }
        }
    }
    validate_existential_safety(id, resolved, decls, diags, &mut HashSet::new());
    let expected = decls
        .trait_decls
        .get(&id)
        .map(|decl| decl.generics.len())
        .unwrap_or(0);
    if generics.len() != expected {
        diags.push(
            Diagnostic::error(format!(
                "trait `{}` takes {expected} type argument(s), found {}",
                resolved.definitions.get(id).name,
                generics.len()
            ))
            .with_label(span, "wrong number of trait arguments"),
        );
        return Type::Error;
    }
    let args = generics
        .iter()
        .map(|arg| lower_type_expr_inner(arg, resolved, decls, diags, visiting))
        .collect();
    if opaque {
        Type::Some(id, args)
    } else {
        Type::Any(id, args)
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
        Resolution::GenericParam | Resolution::ConstParam => {
            let owner = Type::Generic(path.segments[0].name.clone());
            match path.segments.get(1) {
                Some(member) if path.segments.len() == 2 && generics.is_empty() => {
                    Type::Associated(Box::new(owner), member.name.clone())
                }
                Some(_) => {
                    diags.push(
                        Diagnostic::error("an associated type projection has exactly two segments")
                            .with_label(path.span, "invalid projection"),
                    );
                    Type::Error
                }
                None => owner,
            }
        }
        Resolution::Error => Type::Error,
        Resolution::Def(id) => {
            let def = resolved.definitions.get(id);
            let name = def.name.as_str();
            if res.consumed == 1 && path.segments.len() == 2 {
                if !generics.is_empty() {
                    diags.push(
                        Diagnostic::error("generic arguments belong on the projection owner")
                            .with_label(path.span, "write the owner type before `.Associated`"),
                    );
                    return Type::Error;
                }
                let owner = match def.kind {
                    DefKind::Type => match decls.type_decls.get(&id).map(|decl| &decl.kind) {
                        Some(StructDeclKind::TupleStruct(_)) => Type::TupleStruct(id, Vec::new()),
                        _ => Type::Struct(id, Vec::new()),
                    },
                    DefKind::Enum => Type::Enum(id, Vec::new()),
                    _ => {
                        diags.push(
                            Diagnostic::error("only a concrete type can own an associated type")
                                .with_label(path.span, "invalid associated type owner"),
                        );
                        return Type::Error;
                    }
                };
                return Type::Associated(Box::new(owner), path.segments[1].name.clone());
            }
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
        // Local/EnumVariant/StaticMember/StaticConst never arise for a type-position
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
    let mut const_params: HashMap<Symbol, Type> = f
        .generics
        .iter()
        .filter_map(|generic| {
            let syntax = generic.const_ty.as_ref()?;
            let ty = lower_type_expr(syntax, resolved, decls, diags);
            if !matches!(ty, Type::Primitive(kind) if kind.is_integer()) {
                diags.push(
                    Diagnostic::error("a const generic parameter must have an integer type")
                        .with_label(syntax.span(), "not an integer type"),
                );
            }
            Some((generic.name.name.clone(), ty))
        })
        .collect();
    let mut generics: Vec<(Symbol, Vec<GenericBound>)> = f
        .generics
        .iter()
        .map(|g| {
            (
                g.name.name.clone(),
                g.bounds
                    .iter()
                    .filter_map(|bound| lower_generic_bound(bound, resolved, decls, diags))
                    .collect(),
            )
        })
        .collect();
    if f.params.last().is_some_and(|param| param.variadic) {
        let name = crate::sig::variadic_len_param();
        generics.push((name.clone(), Vec::new()));
        const_params.insert(name, Type::Primitive(PrimitiveKind::Usize));
    }
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
        visibility: f.visibility,
        file: f.span.file,
        self_param: f.self_param,
        is_async: f.is_async,
        params,
        ret,
        generics,
        const_params,
        return_origins: Vec::new(),
    }
}
