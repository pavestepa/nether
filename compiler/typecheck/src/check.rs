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
    let Some(id) = type_expr_def_id(ty, resolved) else {
        return None;
    };
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

fn build_type_shapes(
    decls: &DeclIndex,
    resolved: &ResolvedNames,
    sigs: &mut Signatures,
    diags: &mut Vec<Diagnostic>,
) {
    for (&id, t) in &decls.type_decls {
        sigs.type_generics
            .insert(id, t.generics.iter().map(|g| g.name.name.clone()).collect());
        sigs.generic_type_bounds.insert(
            id,
            t.generics
                .iter()
                .map(|generic| {
                    generic
                        .bound
                        .as_ref()
                        .and_then(|bound| lower_generic_bound(bound, resolved, decls, diags))
                })
                .collect(),
        );
        let shape = match &t.kind {
            TypeDeclKind::Struct(fields) => TypeShape::Struct(
                fields
                    .iter()
                    .map(|f| {
                        (
                            f.name.name.clone(),
                            lower_type_expr(&f.ty, resolved, decls, diags),
                        )
                    })
                    .collect(),
            ),
            TypeDeclKind::TupleStruct(tys) => TypeShape::TupleStruct(
                tys.iter()
                    .map(|ty| lower_type_expr(ty, resolved, decls, diags))
                    .collect(),
            ),
            TypeDeclKind::Unit => TypeShape::Unit,
        };
        sigs.type_shapes.insert(id, shape);
    }
}

fn build_enum_sigs(
    module: &Module,
    resolved: &ResolvedNames,
    decls: &DeclIndex,
    sigs: &mut Signatures,
    diags: &mut Vec<Diagnostic>,
) {
    // `Option`/`Result` need no seeding here — they're ordinary `enum`
    // items in the bundled prelude (`stdlib/option.nt`/`result.nt`), so
    // the loop below already covers them exactly like any user-declared
    // generic enum.
    for item in &module.items {
        let Item::Enum(e) = item else { continue };
        let Some(id) = resolved.definitions.lookup_in(e.span.file, &e.name.name) else {
            continue;
        };
        let generics = e.generics.iter().map(|g| g.name.name.clone()).collect();
        sigs.generic_type_bounds.insert(
            id,
            e.generics
                .iter()
                .map(|generic| {
                    generic
                        .bound
                        .as_ref()
                        .and_then(|bound| lower_generic_bound(bound, resolved, decls, diags))
                })
                .collect(),
        );
        let variants = e
            .variants
            .iter()
            .map(|v| {
                (
                    v.name.name.clone(),
                    v.payload
                        .iter()
                        .map(|t| lower_type_expr(t, resolved, decls, diags))
                        .collect(),
                )
            })
            .collect();
        sigs.enum_sigs.insert(id, EnumSig { generics, variants });
    }
}

fn build_fn_sigs(
    module: &Module,
    resolved: &ResolvedNames,
    decls: &DeclIndex,
    sigs: &mut Signatures,
    diags: &mut Vec<Diagnostic>,
) {
    for item in &module.items {
        if let Item::Fn(f) = item {
            if let Some(id) = resolved.definitions.lookup_in(f.span.file, &f.name.name) {
                let sig = build_fn_sig(f, resolved, decls, diags);
                sigs.fns.insert(id, sig);
            }
        }
    }
}

#[derive(Clone)]
struct InterfaceDefault {
    source: DefId,
    /// Source-interface generic parameters expressed in the current
    /// interface's generic parameters.
    subst: HashMap<Symbol, Type>,
}

#[derive(Clone)]
struct InterfaceMethod {
    sig: FnSig,
    default: Option<InterfaceDefault>,
    ambiguous_default: bool,
}

type InterfaceMethodTable = HashMap<DefId, HashMap<Symbol, InterfaceMethod>>;

fn specialize_fn_sig(sig: &FnSig, subst: &HashMap<Symbol, Type>) -> FnSig {
    FnSig {
        self_param: sig.self_param,
        params: sig
            .params
            .iter()
            .map(|param| crate::sig::ParamSig {
                name: param.name.clone(),
                mutable: param.mutable,
                variadic: param.variadic,
                ty: substitute_generic(&param.ty, subst),
            })
            .collect(),
        ret: substitute_generic(&sig.ret, subst),
        generics: sig
            .generics
            .iter()
            .map(|(name, bound)| {
                (
                    name.clone(),
                    bound.as_ref().map(|bound| GenericBound {
                        interface: bound.interface,
                        args: bound
                            .args
                            .iter()
                            .map(|arg| substitute_generic(arg, subst))
                            .collect(),
                    }),
                )
            })
            .collect(),
    }
}

fn method_signatures_match(actual: &FnSig, required: &FnSig) -> bool {
    actual.self_param == required.self_param
        && actual.params.len() == required.params.len()
        && actual.generics.len() == required.generics.len()
        && actual
            .params
            .iter()
            .zip(&required.params)
            .all(|(actual, required)| {
                actual.mutable == required.mutable && actual.ty == required.ty
            })
        && actual.ret == required.ret
}

fn owner_generic_params(
    owner: DefId,
    decls: &DeclIndex,
    resolved: &ResolvedNames,
    diags: &mut Vec<Diagnostic>,
) -> Vec<(Symbol, Option<GenericBound>)> {
    let generics = decls
        .type_decls
        .get(&owner)
        .map(|decl| decl.generics.as_slice())
        .or_else(|| {
            decls
                .enum_decls
                .get(&owner)
                .map(|decl| decl.generics.as_slice())
        })
        .unwrap_or(&[]);
    generics
        .iter()
        .map(|generic| {
            (
                generic.name.name.clone(),
                generic
                    .bound
                    .as_ref()
                    .and_then(|bound| lower_generic_bound(bound, resolved, decls, diags)),
            )
        })
        .collect()
}

/// The number of type arguments `owner` takes — a declared type's or
/// enum's own `generics.len()` (`Option`/`Result` included: ordinary
/// prelude `enum`s, not builtins, so they always have a declaration to
/// read here), `0` for anything else (a primitive, an interface, ...).
fn owner_arity(owner: DefId, decls: &DeclIndex) -> usize {
    decls
        .type_decls
        .get(&owner)
        .map(|decl| decl.generics.len())
        .or_else(|| decls.enum_decls.get(&owner).map(|decl| decl.generics.len()))
        .unwrap_or(0)
}

/// Owner generics for `impl<T, ...> Target<...> { ... }` — the explicit
/// Rust-like form ([`nether_ast::ImplBlock::generics`] non-empty). Unlike
/// the implicit form (whose scope is read straight off the target's own
/// declaration, see [`owner_generic_params`]), this form is the only way
/// to bind a name for a builtin owner such as `Option`/`Result` that has
/// no declaration to read parameters from.
///
/// `target_args` is deliberately restricted to a bare permutation of the
/// impl's own declared generic names, in the owner's positional order —
/// pure renaming, not specialization. `impl Option<i32>` (a concrete
/// argument) or `impl<T, U> Option<T>` (an unused impl parameter) are
/// rejected; only `impl<T> Option<T>` is accepted here (MVP scope —
/// concrete/specialized impls are a separate, not-yet-supported feature).
fn explicit_impl_owner_generics(
    block: &ImplBlock,
    owner: DefId,
    decls: &DeclIndex,
    resolved: &ResolvedNames,
    diags: &mut Vec<Diagnostic>,
) -> Vec<(Symbol, Option<GenericBound>)> {
    let owner_name = resolved.definitions.get(owner).name.clone();
    let expected = owner_arity(owner, decls);

    let fallback = |diags: &mut Vec<Diagnostic>| {
        block
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
            .collect::<Vec<_>>()
    };

    if block.target_args.len() != expected {
        diags.push(
            Diagnostic::error(format!(
                "`{owner_name}` takes {expected} type argument(s), found {}",
                block.target_args.len()
            ))
            .with_label(block.span, "in this `impl` block"),
        );
        return fallback(diags);
    }

    let mut used = HashSet::new();
    let mut ordered_names = Vec::with_capacity(block.target_args.len());
    let mut ok = true;
    for arg in &block.target_args {
        let named = match arg {
            TypeExpr::Named {
                path, generics, ..
            } if generics.is_empty() && path.segments.len() == 1 => block
                .generics
                .iter()
                .find(|g| g.name.name == path.segments[0].name)
                .map(|g| g.name.name.clone()),
            _ => None,
        };
        match named {
            Some(name) if used.insert(name.clone()) => ordered_names.push(name),
            _ => {
                ok = false;
                diags.push(
                    Diagnostic::error(
                        "an explicit `impl<...> Target<...>` argument list must name each of \
                         the impl's own generic parameters exactly once (no concrete types, no \
                         repeats)",
                    )
                    .with_label(arg.span(), "not one of this impl's generic parameters"),
                );
            }
        }
    }
    if ok && ordered_names.len() != block.generics.len() {
        ok = false;
        diags.push(
            Diagnostic::error(format!(
                "every one of this `impl`'s generic parameters must appear in `{owner_name}<...>`"
            ))
            .with_label(block.span, "in this `impl` block"),
        );
    }
    if !ok {
        return fallback(diags);
    }

    ordered_names
        .into_iter()
        .map(|name| {
            let bound = block
                .generics
                .iter()
                .find(|g| g.name.name == name)
                .and_then(|g| g.bound.as_ref())
                .and_then(|bound| lower_generic_bound(bound, resolved, decls, diags));
            (name, bound)
        })
        .collect()
}

/// Dispatches to the implicit, explicit-generic, or concrete-specialization
/// form depending on what `block` wrote — the one place `build_impl_methods`
/// needs to look up an impl block's generic scope. A concrete
/// specialization (`impl Option<i32> { ... }`, [`impl_specialization_args`])
/// has no generic parameters at all in scope for its own methods: every
/// field the `self` receiver could mention is already a concrete type.
fn owner_generics_for_impl(
    block: &ImplBlock,
    owner: DefId,
    decls: &DeclIndex,
    resolved: &ResolvedNames,
    diags: &mut Vec<Diagnostic>,
) -> Vec<(Symbol, Option<GenericBound>)> {
    if !block.generics.is_empty() {
        explicit_impl_owner_generics(block, owner, decls, resolved, diags)
    } else if block.target_args.is_empty() {
        owner_generic_params(owner, decls, resolved, diags)
    } else {
        Vec::new()
    }
}

/// Whether `block` is a concrete specialization — `impl Option<i32> {
/// ... }`, no `impl<...>` generics of its own but a fully concrete
/// `target_args` list (necessarily concrete: with no generics pushed into
/// scope for this block, `resolver` could only have resolved each
/// `target_args` entry to an actual declared type, never a generic
/// parameter — see `resolver::resolve_impl_block`'s explicit-form
/// branch). `None` for every other block shape, including on an arity
/// mismatch (already diagnosed here; the caller falls back to treating it
/// as an ordinary non-specialized block).
fn impl_specialization_args(
    block: &ImplBlock,
    owner: DefId,
    decls: &DeclIndex,
    resolved: &ResolvedNames,
    diags: &mut Vec<Diagnostic>,
) -> Option<Vec<Type>> {
    if !block.generics.is_empty() || block.target_args.is_empty() {
        return None;
    }
    let owner_name = resolved.definitions.get(owner).name.clone();
    let expected = owner_arity(owner, decls);
    if block.target_args.len() != expected {
        diags.push(
            Diagnostic::error(format!(
                "`{owner_name}` takes {expected} type argument(s), found {}",
                block.target_args.len()
            ))
            .with_label(block.span, "in this `impl` block"),
        );
        return None;
    }
    Some(
        block
            .target_args
            .iter()
            .map(|arg| lower_type_expr(arg, resolved, decls, diags))
            .collect(),
    )
}

/// Whether a concrete specialization's method signature (`concrete_sig`,
/// declared with no generics of its own beyond the method's own extra
/// ones) matches the generic impl's signature for the same method name,
/// once the generic impl's own owner parameters are substituted with
/// `concrete_args`. Specialization may only override a method's body, not
/// its externally observable type — required so that a still-generic
/// caller (which can only ever see the generic signature, since it
/// doesn't know which concrete override will apply until monomorphized)
/// never disagrees with what actually runs.
fn specialization_matches_generic(
    owner: DefId,
    decls: &DeclIndex,
    generic_sig: &FnSig,
    concrete_args: &[Type],
    concrete_sig: &FnSig,
) -> bool {
    let owner_arity = owner_arity(owner, decls);
    if generic_sig.self_param != concrete_sig.self_param
        || generic_sig.params.len() != concrete_sig.params.len()
        || generic_sig.generics.len() != owner_arity + concrete_sig.generics.len()
    {
        return false;
    }
    let subst: HashMap<Symbol, Type> = generic_sig.generics[..owner_arity]
        .iter()
        .map(|(name, _)| name.clone())
        .zip(concrete_args.iter().cloned())
        .collect();
    let expected = specialize_fn_sig(generic_sig, &subst);
    expected
        .params
        .iter()
        .zip(&concrete_sig.params)
        .all(|(expected, actual)| expected.mutable == actual.mutable && expected.ty == actual.ty)
        && expected.ret == concrete_sig.ret
}

fn build_interface_method_table(
    decls: &DeclIndex,
    resolved: &ResolvedNames,
    sigs: &mut Signatures,
    diags: &mut Vec<Diagnostic>,
) -> InterfaceMethodTable {
    let mut table = HashMap::new();
    for (&id, iface) in &decls.interface_decls {
        sigs.interface_generics.insert(
            id,
            iface
                .generics
                .iter()
                .map(|generic| generic.name.name.clone())
                .collect(),
        );
        let parents = iface
            .parents
            .iter()
            .filter_map(|parent| lower_generic_bound(parent, resolved, decls, diags))
            .collect();
        sigs.interface_parents.insert(id, parents);
    }

    fn build_one(
        id: DefId,
        decls: &DeclIndex,
        resolved: &ResolvedNames,
        sigs: &Signatures,
        diags: &mut Vec<Diagnostic>,
        visiting: &mut HashSet<DefId>,
        table: &mut InterfaceMethodTable,
    ) {
        if table.contains_key(&id) {
            return;
        }
        let Some(iface) = decls.interface_decls.get(&id).copied() else {
            return;
        };
        if !visiting.insert(id) {
            diags.push(
                Diagnostic::error(format!(
                    "interface inheritance cycle involving `{}`",
                    resolved.definitions.get(id).name
                ))
                .with_label(iface.span, "cycle reaches this interface"),
            );
            return;
        }

        let mut methods: HashMap<Symbol, InterfaceMethod> = HashMap::new();
        for parent in sigs.interface_parents.get(&id).cloned().unwrap_or_default() {
            build_one(
                parent.interface,
                decls,
                resolved,
                sigs,
                diags,
                visiting,
                table,
            );
            let parent_generics = sigs
                .interface_generics
                .get(&parent.interface)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let parent_subst: HashMap<Symbol, Type> = parent_generics
                .iter()
                .cloned()
                .zip(parent.args.iter().cloned())
                .collect();
            for (name, inherited) in table.get(&parent.interface).cloned().unwrap_or_default() {
                let inherited = InterfaceMethod {
                    sig: specialize_fn_sig(&inherited.sig, &parent_subst),
                    default: inherited.default.map(|default| InterfaceDefault {
                        source: default.source,
                        subst: default
                            .subst
                            .into_iter()
                            .map(|(name, ty)| (name, substitute_generic(&ty, &parent_subst)))
                            .collect(),
                    }),
                    ambiguous_default: inherited.ambiguous_default,
                };
                if let Some(existing) = methods.get_mut(&name) {
                    if !method_signatures_match(&existing.sig, &inherited.sig) {
                        diags.push(
                            Diagnostic::error(format!(
                                "inherited method `{name}` has incompatible signatures in interface `{}`",
                                iface.name.name
                            ))
                            .with_label(iface.span, "conflicting parent interfaces"),
                        );
                    }
                    existing.ambiguous_default |= inherited.ambiguous_default;
                    match (&existing.default, &inherited.default) {
                        (Some(left), Some(right))
                            if left.source != right.source || left.subst != right.subst =>
                        {
                            existing.default = None;
                            existing.ambiguous_default = true;
                        }
                        (None, Some(default)) if !existing.ambiguous_default => {
                            existing.default = Some(default.clone());
                        }
                        _ => {}
                    }
                } else {
                    methods.insert(name, inherited);
                }
            }
        }

        let identity_subst: HashMap<Symbol, Type> = iface
            .generics
            .iter()
            .map(|generic| {
                (
                    generic.name.name.clone(),
                    Type::Generic(generic.name.name.clone()),
                )
            })
            .collect();
        for method in &iface.methods {
            methods.insert(
                method.name.name.clone(),
                InterfaceMethod {
                    sig: build_fn_sig(method, resolved, decls, diags),
                    default: method.body.as_ref().map(|_| InterfaceDefault {
                        source: id,
                        subst: identity_subst.clone(),
                    }),
                    ambiguous_default: false,
                },
            );
        }
        visiting.remove(&id);
        table.insert(id, methods);
    }

    let mut visiting = HashSet::new();
    for id in decls.interface_decls.keys().copied().collect::<Vec<_>>() {
        build_one(id, decls, resolved, sigs, diags, &mut visiting, &mut table);
    }
    for (interface, methods) in &table {
        for (name, method) in methods {
            sigs.interface_methods
                .insert((*interface, name.clone()), method.sig.clone());
        }
    }
    table
}

fn build_impl_methods(
    module: &Module,
    resolved: &ResolvedNames,
    decls: &DeclIndex,
    interface_methods: &InterfaceMethodTable,
    sigs: &mut Signatures,
    diags: &mut Vec<Diagnostic>,
) {
    // All user-written methods share one namespace per owner, regardless
    // of which freely mixed impl block contains them — except a concrete
    // specialization's own bucket, which only overrides its exact owner
    // arguments (`MethodSet`).
    struct RawMethod {
        owner: DefId,
        owner_name: Symbol,
        name: Symbol,
        name_span: Span,
        specialization: Option<Vec<Type>>,
        sig: FnSig,
    }
    let mut raw = Vec::new();
    for item in &module.items {
        let Item::Impl(b) = item else { continue };
        let Some(owner) = resolved.definitions.lookup_in(b.span.file, &b.target.name) else {
            continue;
        };
        let owner_generics = owner_generics_for_impl(b, owner, decls, resolved, diags);
        let specialization = impl_specialization_args(b, owner, decls, resolved, diags);
        if let Some(args) = &specialization {
            sigs.impl_specializations.insert(b.id, args.clone());
            if !b.interfaces.is_empty() {
                diags.push(
                    Diagnostic::error(
                        "a concrete specialization (`impl Owner<ConcreteArgs>`) cannot also \
                         implement an interface yet",
                    )
                    .with_label(b.span, "in this `impl` block"),
                );
            }
        }
        for m in &b.methods {
            if specialization.is_some() && m.self_param.is_none() {
                diags.push(
                    Diagnostic::error(
                        "a concrete specialization cannot override a static method yet — only \
                         `self`/`mut self` methods",
                    )
                    .with_label(m.name.span, "here"),
                );
                continue;
            }
            let mut sig = build_fn_sig(m, resolved, decls, diags);
            sig.generics.splice(0..0, owner_generics.clone());
            raw.push(RawMethod {
                owner,
                owner_name: b.target.name.clone(),
                name: m.name.name.clone(),
                name_span: m.name.span,
                specialization: specialization.clone(),
                sig,
            });
        }
    }

    let mut sets: HashMap<(DefId, Symbol), MethodSet> = HashMap::new();
    for entry in &raw {
        let key = (entry.owner, entry.name.clone());
        let set = sets.entry(key).or_default();
        match &entry.specialization {
            None => {
                if set.generic.is_some() {
                    diags.push(
                        Diagnostic::error(format!(
                            "method `{}` is defined more than once for `{}`",
                            entry.name, entry.owner_name
                        ))
                        .with_label(entry.name_span, "redefined here"),
                    );
                } else {
                    set.generic = Some(entry.sig.clone());
                }
            }
            Some(args) => {
                if set
                    .specializations
                    .iter()
                    .any(|(existing, _)| existing == args)
                {
                    diags.push(
                        Diagnostic::error(format!(
                            "method `{}` is defined more than once for this specialization of `{}`",
                            entry.name, entry.owner_name
                        ))
                        .with_label(entry.name_span, "redefined here"),
                    );
                } else {
                    set.specializations.push((args.clone(), entry.sig.clone()));
                }
            }
        }
    }
    for entry in &raw {
        let Some(args) = &entry.specialization else {
            continue;
        };
        let key = (entry.owner, entry.name.clone());
        let Some(generic_sig) = sets.get(&key).and_then(|set| set.generic.as_ref()) else {
            continue;
        };
        if !specialization_matches_generic(entry.owner, decls, generic_sig, args, &entry.sig) {
            diags.push(
                Diagnostic::error(format!(
                    "method `{}` on this specialization of `{}` must have the same signature as \
                     the generic `impl<...> {}<...>` version",
                    entry.name, entry.owner_name, entry.owner_name
                ))
                .with_label(entry.name_span, "signature does not match"),
            );
        }
    }
    sigs.methods = sets;

    struct Request {
        owner: DefId,
        owner_ty: Type,
        owner_name: Symbol,
        owner_span: Span,
        owner_generics: Vec<(Symbol, Option<GenericBound>)>,
        bound: GenericBound,
        allow_defaults: bool,
    }

    let mut requests = Vec::new();
    for (&owner, decl) in &decls.type_decls {
        for interface in &decl.interfaces {
            if let Some(bound) = lower_generic_bound(interface, resolved, decls, diags) {
                let owner_generics = owner_generic_params(owner, decls, resolved, diags);
                requests.push(Request {
                    owner,
                    owner_ty: owner_as_type_from_generics(owner, resolved, decls, &owner_generics),
                    owner_name: decl.name.name.clone(),
                    owner_span: interface.span(),
                    owner_generics,
                    bound,
                    allow_defaults: true,
                });
            }
        }
    }
    for (&owner, decl) in &decls.enum_decls {
        for interface in &decl.interfaces {
            if let Some(bound) = lower_generic_bound(interface, resolved, decls, diags) {
                let owner_generics = owner_generic_params(owner, decls, resolved, diags);
                requests.push(Request {
                    owner,
                    owner_ty: owner_as_type_from_generics(owner, resolved, decls, &owner_generics),
                    owner_name: decl.name.name.clone(),
                    owner_span: interface.span(),
                    owner_generics,
                    bound,
                    allow_defaults: true,
                });
            }
        }
    }
    for item in &module.items {
        let Item::Impl(block) = item else { continue };
        let Some(owner) = resolved
            .definitions
            .lookup_in(block.span.file, &block.target.name)
        else {
            continue;
        };
        for interface in &block.interfaces {
            if let Some(bound) = lower_generic_bound(interface, resolved, decls, diags) {
                let owner_generics = owner_generics_for_impl(block, owner, decls, resolved, diags);
                requests.push(Request {
                    owner,
                    owner_ty: owner_as_type_from_generics(owner, resolved, decls, &owner_generics),
                    owner_name: block.target.name.clone(),
                    owner_span: interface.span(),
                    owner_generics,
                    bound,
                    allow_defaults: false,
                });
            }
        }
    }

    let mut seen = HashSet::new();
    requests.retain(|request| {
        if seen.insert((request.owner, request.bound.clone())) {
            sigs.impls
                .insert((request.owner_ty.clone(), request.bound.clone()));
            true
        } else {
            diags.push(
                Diagnostic::error(format!(
                    "`{}` already implements `{}`",
                    request.owner_name,
                    resolved.definitions.get(request.bound.interface).name
                ))
                .with_label(request.owner_span, "duplicate implementation"),
            );
            false
        }
    });

    #[derive(Clone)]
    struct DefaultCandidate {
        source: DefId,
        subst: HashMap<Symbol, Type>,
        sig: FnSig,
    }
    struct DeclaredNeed {
        owner_name: Symbol,
        span: Span,
        interface_name: Symbol,
        sig: FnSig,
        defaults: Vec<DefaultCandidate>,
        ambiguous: bool,
    }
    let mut declared: HashMap<(DefId, Symbol), DeclaredNeed> = HashMap::new();

    for request in requests {
        let iface_id = request.bound.interface;
        let Some(methods) = interface_methods.get(&iface_id) else {
            continue;
        };
        let interface_generics = sigs
            .interface_generics
            .get(&iface_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let interface_subst: HashMap<Symbol, Type> = interface_generics
            .iter()
            .cloned()
            .zip(request.bound.args.iter().cloned())
            .collect();
        let owner_generics = request.owner_generics.clone();

        for (name, method) in methods {
            let mut expected = specialize_fn_sig(&method.sig, &interface_subst);
            expected.generics.splice(0..0, owner_generics.clone());
            let key = (request.owner, name.clone());
            if let Some(actual) = sigs.methods.get(&key).and_then(|set| set.generic.as_ref()) {
                if !method_signatures_match(actual, &expected) {
                    diags.push(
                        Diagnostic::error(format!(
                            "method `{name}` does not match its declaration in interface `{}`",
                            resolved.definitions.get(iface_id).name
                        ))
                        .with_label(request.owner_span, "implementation is here"),
                    );
                }
                continue;
            }

            if !request.allow_defaults {
                diags.push(
                    Diagnostic::error(format!(
                        "`{}` must explicitly implement method `{name}` of interface `{}`",
                        request.owner_name,
                        resolved.definitions.get(iface_id).name
                    ))
                    .with_label(request.owner_span, "explicit implementation is here"),
                );
                continue;
            }

            let candidate = method.default.as_ref().map(|default| DefaultCandidate {
                source: default.source,
                subst: default
                    .subst
                    .iter()
                    .map(|(name, ty)| (name.clone(), substitute_generic(ty, &interface_subst)))
                    .collect(),
                sig: expected.clone(),
            });
            let need = declared.entry(key).or_insert_with(|| DeclaredNeed {
                owner_name: request.owner_name.clone(),
                span: request.owner_span,
                interface_name: resolved.definitions.get(iface_id).name.clone(),
                sig: expected.clone(),
                defaults: Vec::new(),
                ambiguous: false,
            });
            if !method_signatures_match(&need.sig, &expected) {
                need.ambiguous = true;
                diags.push(
                    Diagnostic::error(format!(
                        "method `{name}` has incompatible signatures in implemented interfaces"
                    ))
                    .with_label(request.owner_span, "conflicting interface"),
                );
            }
            need.ambiguous |= method.ambiguous_default;
            if let Some(candidate) = candidate {
                if !need.defaults.iter().any(|existing| {
                    existing.source == candidate.source && existing.subst == candidate.subst
                }) {
                    need.defaults.push(candidate);
                }
            }
        }
    }

    for ((owner, name), need) in declared {
        if need.ambiguous || need.defaults.len() > 1 {
            diags.push(
                Diagnostic::error(format!(
                    "multiple default implementations of method `{name}` are available for `{}`; provide an explicit implementation",
                    need.owner_name
                ))
                .with_label(need.span, "ambiguous default"),
            );
        } else if let Some(default) = need.defaults.into_iter().next() {
            sigs.methods.entry((owner, name.clone())).or_default().generic = Some(default.sig);
            sigs.default_method_substitutions
                .insert((owner, name.clone()), default.subst);
            sigs.default_method_sources
                .insert((owner, name), default.source);
        } else {
            diags.push(
                Diagnostic::error(format!(
                    "`{}` does not implement required method `{name}` of interface `{}`",
                    need.owner_name, need.interface_name
                ))
                .with_label(need.span, "missing implementation"),
            );
        }
    }
}

fn owner_as_type(id: DefId, resolved: &ResolvedNames, decls: &DeclIndex) -> Type {
    let args: Vec<Type> = decls
        .type_decls
        .get(&id)
        .map(|decl| {
            decl.generics
                .iter()
                .map(|generic| Type::Generic(generic.name.name.clone()))
                .collect()
        })
        .or_else(|| {
            decls.enum_decls.get(&id).map(|decl| {
                decl.generics
                    .iter()
                    .map(|generic| Type::Generic(generic.name.name.clone()))
                    .collect()
            })
        })
        .unwrap_or_default();
    // `Array<T>` still lowers to the distinct `Type::Array` representation
    // (codegen/ARC layout depend on it), not `Type::Struct` — even though
    // it's now an ordinary `type Array<T>;` declaration like any other.
    if resolved.definitions.get(id).name.as_str() == "Array" {
        return Type::Array(Box::new(
            args.into_iter().next().unwrap_or(Type::Error),
        ));
    }
    match resolved.definitions.get(id).kind {
        DefKind::Enum => Type::Enum(id, args),
        _ => match decls.type_decls.get(&id).map(|t| &t.kind) {
            Some(TypeDeclKind::TupleStruct(_)) => Type::TupleStruct(id, args),
            _ => Type::Struct(id, args),
        },
    }
}

/// Like [`owner_as_type`], but takes its generic argument names from an
/// already-resolved owner-generics list instead of re-reading a
/// declaration. The only way to get a non-empty owner's type for a
/// declaration-less builtin (`Option`, `Result`) under an explicit
/// `impl<T> Option<T>: SomeInterface { ... }` block.
fn owner_as_type_from_generics(
    id: DefId,
    resolved: &ResolvedNames,
    decls: &DeclIndex,
    owner_generics: &[(Symbol, Option<GenericBound>)],
) -> Type {
    let args: Vec<Type> = owner_generics
        .iter()
        .map(|(name, _)| Type::Generic(name.clone()))
        .collect();
    // See `owner_as_type`'s matching comment: `Array<T>` keeps its own
    // distinct `Type` variant regardless of generic-scope source.
    if resolved.definitions.get(id).name.as_str() == "Array" {
        return Type::Array(Box::new(
            args.into_iter().next().unwrap_or(Type::Error),
        ));
    }
    match resolved.definitions.get(id).kind {
        DefKind::Enum => Type::Enum(id, args),
        _ => match decls.type_decls.get(&id).map(|t| &t.kind) {
            Some(TypeDeclKind::TupleStruct(_)) => Type::TupleStruct(id, args),
            _ => Type::Struct(id, args),
        },
    }
}

fn validate_finite_value_layouts(
    resolved: &ResolvedNames,
    decls: &DeclIndex,
    sigs: &Signatures,
    diags: &mut Vec<Diagnostic>,
) {
    for (&id, decl) in &decls.type_decls {
        let ty = owner_as_type(id, resolved, decls);
        if crate::alloc::alloc_kind(&ty, &resolved.definitions) == crate::alloc::AllocKind::Heap {
            continue;
        }
        if value_layout_reaches_cycle(&ty, resolved, sigs, &mut Vec::new()) {
            diags.push(
                Diagnostic::error(format!(
                    "value type `{}` has an infinitely recursive layout",
                    decl.name.name
                ))
                .with_label(
                    decl.span,
                    "introduce a heap-allocated PascalCase type or another indirection",
                ),
            );
        }
    }
    for (&id, decl) in &decls.enum_decls {
        let ty = owner_as_type(id, resolved, decls);
        if value_layout_reaches_cycle(&ty, resolved, sigs, &mut Vec::new()) {
            diags.push(
                Diagnostic::error(format!(
                    "enum `{}` has an infinitely recursive layout",
                    decl.name.name
                ))
                .with_label(
                    decl.span,
                    "introduce a heap-allocated PascalCase type or another indirection",
                ),
            );
        }
    }
}

fn value_layout_reaches_cycle(
    ty: &Type,
    resolved: &ResolvedNames,
    sigs: &Signatures,
    stack: &mut Vec<DefId>,
) -> bool {
    match ty {
        Type::Struct(id, _) | Type::TupleStruct(id, _) => {
            if crate::alloc::alloc_kind(ty, &resolved.definitions) == crate::alloc::AllocKind::Heap
            {
                return false;
            }
            if stack.contains(id) {
                return true;
            }
            stack.push(*id);
            let recursive = sigs
                .type_fields(ty)
                .unwrap_or_default()
                .iter()
                .any(|field| value_layout_reaches_cycle(field, resolved, sigs, stack));
            stack.pop();
            recursive
        }
        Type::Enum(id, _) => {
            if stack.contains(id) {
                return true;
            }
            stack.push(*id);
            let recursive = sigs.enum_sigs.get(id).is_some_and(|sig| {
                (0..sig.variants.len()).any(|variant| {
                    sigs.enum_payload(ty, variant as u32)
                        .unwrap_or_default()
                        .iter()
                        .any(|field| value_layout_reaches_cycle(field, resolved, sigs, stack))
                })
            });
            stack.pop();
            recursive
        }
        Type::Tuple(items) => items
            .iter()
            .any(|item| value_layout_reaches_cycle(item, resolved, sigs, stack)),
        // These all provide an indirection or have a fixed scalar layout.
        Type::Array(_)
        | Type::String
        | Type::Function(_, _)
        | Type::Weak(_)
        | Type::Primitive(_)
        | Type::Interface(_)
        | Type::Generic(_)
        | Type::Never
        | Type::Error => false,
    }
}

// ---------------------------------------------------------------------
// Top-level entry point
// ---------------------------------------------------------------------

/// Type-checks `module` using the names `resolver` already resolved.
///
/// See this crate's module docs for the full list of checks performed and
/// the documented simplifications (notably no `break`-value unification
/// for `loop` and intentionally local generic inference).
pub fn check(module: &Module, resolved: &ResolvedNames) -> (TypedTables, Vec<Diagnostic>) {
    let mut diagnostics = Vec::new();
    let decls = index_decls(module, resolved);

    let mut sigs = Signatures::default();
    build_type_shapes(&decls, resolved, &mut sigs, &mut diagnostics);
    build_enum_sigs(module, resolved, &decls, &mut sigs, &mut diagnostics);
    build_fn_sigs(module, resolved, &decls, &mut sigs, &mut diagnostics);
    let interface_methods =
        build_interface_method_table(&decls, resolved, &mut sigs, &mut diagnostics);
    build_impl_methods(
        module,
        resolved,
        &decls,
        &interface_methods,
        &mut sigs,
        &mut diagnostics,
    );
    validate_finite_value_layouts(resolved, &decls, &sigs, &mut diagnostics);

    let mut expr_types = HashMap::new();
    let mut local_types = HashMap::new();
    let mut call_generic_args = HashMap::new();
    for item in &module.items {
        match item {
            Item::Fn(f) => {
                if let Some(id) = resolved.definitions.lookup_in(f.span.file, &f.name.name) {
                    if let Some(sig) = sigs.fns.get(&id).cloned() {
                        if f.name.name.as_str() == "main"
                            && (!sig.params.is_empty()
                                || !sig.generics.is_empty()
                                || sig.ret != Type::unit())
                        {
                            diagnostics.push(
                                Diagnostic::error("`main` must have signature `fn main()`")
                                    .with_label(f.span, "invalid entry point"),
                            );
                            continue;
                        }
                        let mut checker = Checker::new(
                            resolved,
                            &sigs,
                            &decls,
                            &mut expr_types,
                            &mut local_types,
                            &mut call_generic_args,
                            &mut diagnostics,
                        );
                        checker.check_fn_decl(f, &sig, None);
                    }
                }
            }
            Item::Impl(b) => {
                if let Some(owner) = resolved.definitions.lookup_in(b.span.file, &b.target.name) {
                    let self_ty = owner_as_type(owner, resolved, &decls);
                    for m in &b.methods {
                        if let Some(sig) = sigs.method(owner, &m.name.name).cloned() {
                            let mut checker = Checker::new(
                                resolved,
                                &sigs,
                                &decls,
                                &mut expr_types,
                                &mut local_types,
                                &mut call_generic_args,
                                &mut diagnostics,
                            );
                            checker.check_fn_decl(m, &sig, Some(self_ty.clone()));
                        }
                    }
                }
            }
            Item::Interface(i) => {
                if let Some(id) = resolved.definitions.lookup_in(i.span.file, &i.name.name) {
                    for m in &i.methods {
                        if m.body.is_none() {
                            continue;
                        }
                        if let Some(method) = interface_methods
                            .get(&id)
                            .and_then(|ms| ms.get(&m.name.name))
                            .cloned()
                        {
                            let mut checking_sig = method.sig;
                            let interface_generics = i.generics.iter().map(|generic| {
                                (
                                    generic.name.name.clone(),
                                    generic.bound.as_ref().and_then(|bound| {
                                        lower_generic_bound(
                                            bound,
                                            resolved,
                                            &decls,
                                            &mut diagnostics,
                                        )
                                    }),
                                )
                            });
                            checking_sig.generics.splice(0..0, interface_generics);
                            let mut checker = Checker::new(
                                resolved,
                                &sigs,
                                &decls,
                                &mut expr_types,
                                &mut local_types,
                                &mut call_generic_args,
                                &mut diagnostics,
                            );
                            checker.check_fn_decl(m, &checking_sig, Some(Type::Interface(id)));
                        }
                    }
                }
            }
            Item::Type(_) | Item::Enum(_) | Item::Use(_) | Item::Mod(_) => {}
        }
    }

    (
        TypedTables {
            expr_types,
            local_types,
            call_generic_args,
            signatures: sigs,
        },
        diagnostics,
    )
}

fn describe_type(ty: &Type, resolved: &ResolvedNames) -> String {
    match ty {
        Type::Primitive(p) => format!("{p:?}").to_lowercase(),
        Type::Struct(id, args) | Type::TupleStruct(id, args) => {
            let name = resolved.definitions.get(*id).name.to_string();
            if args.is_empty() {
                name
            } else {
                let args = args
                    .iter()
                    .map(|arg| describe_type(arg, resolved))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{name}<{args}>")
            }
        }
        Type::Enum(id, args) => {
            let name = resolved.definitions.get(*id).name.to_string();
            if args.is_empty() {
                name
            } else {
                let args_str: Vec<String> =
                    args.iter().map(|a| describe_type(a, resolved)).collect();
                format!("{name}<{}>", args_str.join(", "))
            }
        }
        Type::Tuple(elems) => {
            format!(
                "({})",
                elems
                    .iter()
                    .map(|e| describe_type(e, resolved))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
        Type::Array(inner) => format!("[{}]", describe_type(inner, resolved)),
        Type::String => "String".to_string(),
        Type::Function(params, ret) => format!(
            "({}) => {}",
            params
                .iter()
                .map(|p| describe_type(p, resolved))
                .collect::<Vec<_>>()
                .join(", "),
            describe_type(ret, resolved)
        ),
        Type::Interface(id) => resolved.definitions.get(*id).name.to_string(),
        Type::Generic(name) => name.to_string(),
        Type::Weak(inner) => format!("weak {}", describe_type(inner, resolved)),
        Type::Never => "!".to_string(),
        Type::Error => "<error>".to_string(),
    }
}

fn substitute_generic(ty: &Type, subst: &HashMap<Symbol, Type>) -> Type {
    match ty {
        Type::Generic(name) => subst.get(name).cloned().unwrap_or_else(|| ty.clone()),
        Type::Struct(id, args) => Type::Struct(
            *id,
            args.iter()
                .map(|arg| substitute_generic(arg, subst))
                .collect(),
        ),
        Type::TupleStruct(id, args) => Type::TupleStruct(
            *id,
            args.iter()
                .map(|arg| substitute_generic(arg, subst))
                .collect(),
        ),
        Type::Array(inner) => Type::Array(Box::new(substitute_generic(inner, subst))),
        Type::Weak(inner) => Type::Weak(Box::new(substitute_generic(inner, subst))),
        Type::Tuple(elems) => {
            Type::Tuple(elems.iter().map(|e| substitute_generic(e, subst)).collect())
        }
        Type::Enum(id, args) => Type::Enum(
            *id,
            args.iter().map(|a| substitute_generic(a, subst)).collect(),
        ),
        Type::Function(params, ret) => Type::Function(
            params
                .iter()
                .map(|p| substitute_generic(p, subst))
                .collect(),
            Box::new(substitute_generic(ret, subst)),
        ),
        other => other.clone(),
    }
}

fn collect_generic_bindings(declared: &Type, actual: &Type, subst: &mut HashMap<Symbol, Type>) {
    match (declared, actual) {
        (Type::Generic(name), actual) if !actual.contains_error() && !actual.contains_generic() => {
            if subst
                .get(name)
                .is_none_or(|existing| existing.contains_error() || existing.contains_generic())
            {
                subst.insert(name.clone(), actual.clone());
            }
        }
        (Type::Array(a), Type::Array(b)) | (Type::Weak(a), Type::Weak(b)) => {
            collect_generic_bindings(a, b, subst);
        }
        (Type::Tuple(a), Type::Tuple(b))
        | (Type::Enum(_, a), Type::Enum(_, b))
        | (Type::Struct(_, a), Type::Struct(_, b))
        | (Type::TupleStruct(_, a), Type::TupleStruct(_, b)) => {
            for (declared, actual) in a.iter().zip(b) {
                collect_generic_bindings(declared, actual, subst);
            }
        }
        (Type::Function(a_params, a_ret), Type::Function(b_params, b_ret)) => {
            for (declared, actual) in a_params.iter().zip(b_params) {
                collect_generic_bindings(declared, actual, subst);
            }
            collect_generic_bindings(a_ret, b_ret, subst);
        }
        _ => {}
    }
}

/// Refines unknown generic-enum components from an enclosing expected
/// type without changing an otherwise incompatible type.  `Type::Error`
/// is also the checker's poison type, so it is replaced only with a
/// concrete expectation; diagnostics already emitted for genuine errors
/// still prevent the pipeline from continuing.
fn contextualize_unknowns(actual: &Type, expected: &Type) -> Type {
    match (actual, expected) {
        (Type::Error, expected) if !expected.contains_error() && !expected.contains_generic() => {
            expected.clone()
        }
        (Type::Tuple(actual), Type::Tuple(expected)) if actual.len() == expected.len() => {
            Type::Tuple(
                actual
                    .iter()
                    .zip(expected)
                    .map(|(actual, expected)| contextualize_unknowns(actual, expected))
                    .collect(),
            )
        }
        (Type::Enum(actual_id, actual), Type::Enum(expected_id, expected))
            if actual_id == expected_id && actual.len() == expected.len() =>
        {
            Type::Enum(
                *actual_id,
                actual
                    .iter()
                    .zip(expected)
                    .map(|(actual, expected)| contextualize_unknowns(actual, expected))
                    .collect(),
            )
        }
        (Type::Struct(actual_id, actual), Type::Struct(expected_id, expected))
            if actual_id == expected_id && actual.len() == expected.len() =>
        {
            Type::Struct(
                *actual_id,
                actual
                    .iter()
                    .zip(expected)
                    .map(|(actual, expected)| contextualize_unknowns(actual, expected))
                    .collect(),
            )
        }
        (Type::TupleStruct(actual_id, actual), Type::TupleStruct(expected_id, expected))
            if actual_id == expected_id && actual.len() == expected.len() =>
        {
            Type::TupleStruct(
                *actual_id,
                actual
                    .iter()
                    .zip(expected)
                    .map(|(actual, expected)| contextualize_unknowns(actual, expected))
                    .collect(),
            )
        }
        (Type::Array(actual), Type::Array(expected)) => {
            Type::Array(Box::new(contextualize_unknowns(actual, expected)))
        }
        (Type::Weak(actual), Type::Weak(expected)) => {
            Type::Weak(Box::new(contextualize_unknowns(actual, expected)))
        }
        (
            Type::Function(actual_params, actual_ret),
            Type::Function(expected_params, expected_ret),
        ) if actual_params.len() == expected_params.len() => Type::Function(
            actual_params
                .iter()
                .zip(expected_params)
                .map(|(actual, expected)| contextualize_unknowns(actual, expected))
                .collect(),
            Box::new(contextualize_unknowns(actual_ret, expected_ret)),
        ),
        _ => actual.clone(),
    }
}

fn prefer_concrete_type(first: Type, second: Type) -> Type {
    if first.contains_error() && !second.contains_error() {
        second
    } else {
        first
    }
}

// ---------------------------------------------------------------------
// Expression / statement checker
// ---------------------------------------------------------------------

struct Checker<'a> {
    resolved: &'a ResolvedNames,
    sigs: &'a Signatures,
    decls: &'a DeclIndex<'a>,
    expr_types: &'a mut HashMap<NodeId, Type>,
    local_types: &'a mut HashMap<NodeId, Type>,
    call_generic_args: &'a mut HashMap<NodeId, Vec<Type>>,
    diagnostics: &'a mut Vec<Diagnostic>,
    locals: HashMap<nether_resolver::LocalId, (Type, bool)>,
    generics: HashMap<Symbol, Option<GenericBound>>,
    return_ty: Type,
    loop_depth: usize,
}

impl<'a> Checker<'a> {
    fn new(
        resolved: &'a ResolvedNames,
        sigs: &'a Signatures,
        decls: &'a DeclIndex<'a>,
        expr_types: &'a mut HashMap<NodeId, Type>,
        local_types: &'a mut HashMap<NodeId, Type>,
        call_generic_args: &'a mut HashMap<NodeId, Vec<Type>>,
        diagnostics: &'a mut Vec<Diagnostic>,
    ) -> Self {
        Checker {
            resolved,
            sigs,
            decls,
            expr_types,
            local_types,
            call_generic_args,
            diagnostics,
            locals: HashMap::new(),
            generics: HashMap::new(),
            return_ty: Type::unit(),
            loop_depth: 0,
        }
    }

    /// Records a binding site's final type (see [`TypedTables::local_types`]
    /// docs) alongside registering it for use within this function's own
    /// body-checking (`self.locals`).
    fn bind_local(&mut self, site: NodeId, ty: Type, mutable: bool) {
        if let Some(local_id) = self.resolved.locals.get(&site) {
            self.locals.insert(*local_id, (ty.clone(), mutable));
        }
        self.local_types.insert(site, ty);
    }

    fn err(&mut self, span: Span, message: impl Into<String>) {
        self.diagnostics
            .push(Diagnostic::error(message).with_label(span, "here"));
    }

    fn describe(&self, ty: &Type) -> String {
        describe_type(ty, self.resolved)
    }

    fn lower_call_generic_args(&mut self, args: &[TypeExpr]) -> Vec<Type> {
        args.iter()
            .map(|arg| {
                let ty = lower_type_expr(arg, self.resolved, self.decls, self.diagnostics);
                self.validate_type_bounds(&ty, arg.span());
                ty
            })
            .collect()
    }

    fn check_fn_decl(&mut self, f: &FnDecl, sig: &FnSig, self_ty: Option<Type>) {
        self.generics = sig.generics.iter().cloned().collect();
        self.locals.clear();
        if let Some(ty) = self_ty {
            let mutable = matches!(f.self_param, Some(nether_ast::SelfParam::ByMutRef));
            self.bind_local(f.id, ty, mutable);
        }
        for (param_ast, param_sig) in f.params.iter().zip(&sig.params) {
            self.validate_type_bounds(&param_sig.ty, param_ast.ty.span());
            // `param_sig.ty` is the *element* type for a variadic
            // parameter — the body sees an ordinary `Array<element>`
            // local, matching what the call site actually passes in
            // (`nether_hir::lower::lower_variadic_aware_args`).
            let local_ty = if param_sig.variadic {
                Type::Array(Box::new(param_sig.ty.clone()))
            } else {
                param_sig.ty.clone()
            };
            self.bind_local(param_ast.id, local_ty, param_ast.mutable);
        }
        self.return_ty = sig.ret.clone();
        if let Some(ret) = &f.ret {
            self.validate_type_bounds(&self.return_ty.clone(), ret.span());
        }
        if let Some(body) = &f.body {
            let expected = self.return_ty.clone();
            let body_ty = self.check_block_with_expected(body, Some(&expected));
            if !body_ty.compatible(&self.return_ty) {
                let expected = self.describe(&self.return_ty.clone());
                let found = self.describe(&body_ty);
                self.err(
                    body.span,
                    format!("expected return type `{expected}`, found `{found}`"),
                );
            }
        }
    }

    fn check_block(&mut self, block: &Block) -> Type {
        self.check_block_with_expected(block, None)
    }

    fn check_block_with_expected(&mut self, block: &Block, expected: Option<&Type>) -> Type {
        // No explicit tail means the block would ordinarily be `()` — but
        // if a statement unconditionally diverges (`return`/`break`/
        // `continue`, `Type::Never`), the block never actually falls
        // through to "after the last statement" at all, so its type
        // should be `Never` too (compatible with anything, per
        // `Type::compatible`'s own doc) rather than a spurious `()`. There
        // is no dead-code diagnostic in this language, so a diverging
        // statement isn't necessarily the *last* one — track any, not
        // just the final one.
        let mut diverges = false;
        for stmt in &block.stmts {
            if matches!(self.check_stmt(stmt), Type::Never) {
                diverges = true;
            }
        }
        match &block.tail {
            Some(tail) => self.check_expr_with_expected(tail, expected),
            None if diverges => Type::Never,
            None => Type::unit(),
        }
    }

    fn check_stmt(&mut self, stmt: &Stmt) -> Type {
        match stmt {
            Stmt::Let(let_stmt) => {
                let declared_ty = let_stmt
                    .ty
                    .as_ref()
                    .map(|t| lower_type_expr(t, self.resolved, self.decls, self.diagnostics));
                let has_declared_type = declared_ty.is_some();
                let value_ty = self.check_expr_with_expected(&let_stmt.value, declared_ty.as_ref());
                let diverges = matches!(value_ty, Type::Never);
                let final_ty = match declared_ty {
                    Some(declared) => {
                        if !value_ty.compatible(&declared) {
                            let expected = self.describe(&declared);
                            let found = self.describe(&value_ty);
                            self.err(
                                let_stmt.value.span,
                                format!("expected `{expected}`, found `{found}`"),
                            );
                        }
                        declared
                    }
                    None => value_ty,
                };
                if !has_declared_type && !final_ty.is_error() && final_ty.contains_error() {
                    self.err(
                        let_stmt.value.span,
                        "cannot infer all generic type arguments from this initializer; add a type annotation",
                    );
                }
                self.bind_local(let_stmt.id, final_ty, let_stmt.mutable);
                if diverges {
                    Type::Never
                } else {
                    Type::unit()
                }
            }
            Stmt::Expr(expr) => {
                let ty = self.check_expr(expr);
                if !ty.is_error() && ty.contains_error() {
                    self.err(
                        expr.span,
                        "cannot infer all generic type arguments for this expression",
                    );
                }
                ty
            }
        }
    }

    fn check_expr(&mut self, expr: &Expr) -> Type {
        self.check_expr_with_expected(expr, None)
    }

    fn check_expr_with_expected(&mut self, expr: &Expr, expected: Option<&Type>) -> Type {
        let synthesized = self.synth_expr(expr, expected);
        let ty = expected
            .map(|expected| contextualize_unknowns(&synthesized, expected))
            .unwrap_or(synthesized);
        self.validate_type_bounds(&ty, expr.span);
        self.expr_types.insert(expr.id, ty.clone());
        ty
    }

    fn synth_expr(&mut self, expr: &Expr, expected: Option<&Type>) -> Type {
        match &expr.kind {
            ExprKind::Literal(Literal::Int(_)) => match expected {
                Some(Type::Primitive(p)) if p.is_integer() => Type::Primitive(*p),
                _ => Type::Primitive(PrimitiveKind::I32),
            },
            ExprKind::Literal(Literal::Float(_)) => match expected {
                Some(Type::Primitive(p)) if p.is_float() => Type::Primitive(*p),
                _ => Type::Primitive(PrimitiveKind::F64),
            },
            ExprKind::Literal(Literal::Bool(_)) => Type::Primitive(PrimitiveKind::Bool),
            ExprKind::Literal(Literal::Char(_)) => Type::Primitive(PrimitiveKind::Char),
            ExprKind::Literal(Literal::Str(_)) => Type::String,
            ExprKind::Path(path) => self.check_value_path(path, None, expected, None, &[]),
            ExprKind::Tuple(elems) => {
                let expected_elems = match expected {
                    Some(Type::Tuple(expected)) if expected.len() == elems.len() => Some(expected),
                    _ => None,
                };
                Type::Tuple(
                    elems
                        .iter()
                        .enumerate()
                        .map(|(index, elem)| {
                            self.check_expr_with_expected(
                                elem,
                                expected_elems.and_then(|expected| expected.get(index)),
                            )
                        })
                        .collect(),
                )
            }
            ExprKind::Array(elems) => self.check_array(elems, expected, expr.span),
            ExprKind::StringTemplate(parts) => {
                for part in parts {
                    if let TemplatePart::Expr(e) = part {
                        let ty = self.check_expr(e);
                        self.require_into_string(&ty, e.span);
                    }
                }
                Type::String
            }
            ExprKind::Unary { op, expr: inner } => self.check_unary(*op, inner),
            ExprKind::Binary { op, lhs, rhs } => self.check_binary(*op, lhs, rhs),
            ExprKind::Assign { target, value } => self.check_assign(target, value),
            ExprKind::Call {
                callee,
                generic_args,
                args,
            } => {
                let generic_args = self.lower_call_generic_args(generic_args);
                self.check_call(expr.id, callee, &generic_args, args, expected)
            }
            ExprKind::MutArg(inner) => self.check_expr(inner),
            ExprKind::MethodCall {
                receiver,
                method,
                generic_args,
                args,
            } => {
                let generic_args = self.lower_call_generic_args(generic_args);
                let receiver_ty = self.check_expr(receiver);
                let receiver_mutable = self.place_root_mutable(receiver);
                self.check_method_call_on(
                    &receiver_ty,
                    method,
                    &generic_args,
                    args,
                    method.span,
                    Some(expr.id),
                    receiver_mutable,
                )
            }
            ExprKind::Field { base, field } => {
                let base_ty = self.check_expr(base);
                self.check_field_access(&base_ty, field)
            }
            ExprKind::Index { base, index } => self.check_index(base, index),
            ExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => self.check_if(cond, then_branch, else_branch, expected),
            ExprKind::Match { scrutinee, arms } => {
                self.check_match(scrutinee, arms, expr.span, expected)
            }
            ExprKind::Block(block) => self.check_block_with_expected(block, expected),
            ExprKind::While { cond, body } => {
                let cond_ty = self.check_expr(cond);
                self.require_bool(&cond_ty, cond.span, "`while` condition");
                self.loop_depth += 1;
                self.check_block(body);
                self.loop_depth -= 1;
                Type::unit()
            }
            ExprKind::ForIn {
                pattern,
                iter,
                body,
            } => self.check_for_in(pattern, iter, body),
            ExprKind::Loop { body } => {
                self.loop_depth += 1;
                self.check_block(body);
                self.loop_depth -= 1;
                Type::unit()
            }
            ExprKind::Break(value) => {
                if self.loop_depth == 0 {
                    self.err(expr.span, "`break` is only valid inside a loop");
                }
                if let Some(v) = value {
                    self.check_expr(v);
                }
                Type::Never
            }
            ExprKind::Continue => {
                if self.loop_depth == 0 {
                    self.err(expr.span, "`continue` is only valid inside a loop");
                }
                Type::Never
            }
            ExprKind::Return(value) => self.check_return(value, expr.span),
            ExprKind::Closure { params, body } => self.check_closure(params, body, expected),
            ExprKind::StructLit { path, fields } => self.check_struct_lit(path, fields, expected),
        }
    }

    fn check_array(&mut self, elems: &[Expr], expected: Option<&Type>, span: Span) -> Type {
        if elems.is_empty() {
            return match expected {
                Some(Type::Array(inner)) => Type::Array(inner.clone()),
                _ => {
                    self.err(
                        span,
                        "cannot infer the element type of an empty array literal without context",
                    );
                    Type::Error
                }
            };
        }
        let elem_expected = match expected {
            Some(Type::Array(inner)) => Some((**inner).clone()),
            _ => None,
        };
        let first = self.check_expr_with_expected(&elems[0], elem_expected.as_ref());
        for e in &elems[1..] {
            let t = self.check_expr_with_expected(e, Some(&first));
            if !t.compatible(&first) {
                let expected_s = self.describe(&first);
                let found_s = self.describe(&t);
                self.err(
                    e.span,
                    format!("expected `{expected_s}`, found `{found_s}` in array literal"),
                );
            }
        }
        Type::Array(Box::new(first))
    }

    fn require_bool(&mut self, ty: &Type, span: Span, what: &str) {
        if !matches!(ty, Type::Primitive(PrimitiveKind::Bool)) && !ty.is_error() {
            self.err(span, format!("{what} must be `bool`"));
        }
    }

    fn check_unary(&mut self, op: UnaryOp, inner: &Expr) -> Type {
        let ty = self.check_expr(inner);
        match op {
            UnaryOp::Neg => {
                if !matches!(&ty, Type::Primitive(p) if p.is_numeric()) && !ty.is_error() {
                    self.err(inner.span, "`-` requires a numeric operand");
                    return Type::Error;
                }
                ty
            }
            UnaryOp::Not => {
                self.require_bool(&ty, inner.span, "`!` operand");
                Type::Primitive(PrimitiveKind::Bool)
            }
        }
    }

    fn check_binary(&mut self, op: BinaryOp, lhs: &Expr, rhs: &Expr) -> Type {
        let lhs_ty = self.check_expr(lhs);
        let rhs_ty = self.check_expr_with_expected(rhs, Some(&lhs_ty));
        match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => {
                if !matches!(&lhs_ty, Type::Primitive(p) if p.is_numeric()) && !lhs_ty.is_error() {
                    self.err(lhs.span, "arithmetic operators require numeric operands");
                    return Type::Error;
                }
                if !lhs_ty.compatible(&rhs_ty) {
                    self.err(
                        rhs.span,
                        "both operands of an arithmetic operator must have the same type",
                    );
                    return Type::Error;
                }
                lhs_ty
            }
            BinaryOp::Eq | BinaryOp::Ne => {
                if !lhs_ty.compatible(&rhs_ty) {
                    self.err(
                        rhs.span,
                        "both operands of `==`/`!=` must have the same type",
                    );
                }
                Type::Primitive(PrimitiveKind::Bool)
            }
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                if !matches!(&lhs_ty, Type::Primitive(p) if p.is_numeric()) && !lhs_ty.is_error() {
                    self.err(lhs.span, "comparison operators require numeric operands");
                } else if !lhs_ty.compatible(&rhs_ty) {
                    self.err(
                        rhs.span,
                        "both operands of a comparison must have the same type",
                    );
                }
                Type::Primitive(PrimitiveKind::Bool)
            }
            BinaryOp::And | BinaryOp::Or => {
                self.require_bool(&lhs_ty, lhs.span, "`&&`/`||` operand");
                self.require_bool(&rhs_ty, rhs.span, "`&&`/`||` operand");
                Type::Primitive(PrimitiveKind::Bool)
            }
        }
    }

    fn check_assign(&mut self, target: &Expr, value: &Expr) -> Type {
        if !self.assign_target_has_local_root(target) {
            self.err(
                target.span,
                "assignment target must be a local binding or one of its fields/elements",
            );
        }
        let target_ty = self.check_assign_target_type(target);
        self.expr_types.insert(target.id, target_ty.clone());
        let value_ty = self.check_expr_with_expected(value, Some(&target_ty));
        if !value_ty.compatible(&target_ty) {
            let expected = self.describe(&target_ty);
            let found = self.describe(&value_ty);
            self.err(
                value.span,
                format!("expected `{expected}`, found `{found}`"),
            );
        }
        self.check_assign_target_mutable(target);
        Type::unit()
    }

    fn assign_target_has_local_root(&self, target: &Expr) -> bool {
        match &target.kind {
            ExprKind::Path(path) => self
                .resolved
                .path_res
                .get(&path.id)
                .is_some_and(|resolution| matches!(resolution.base, Resolution::Local(_))),
            ExprKind::Field { base, .. } | ExprKind::Index { base, .. } => {
                self.assign_target_has_local_root(base)
            }
            _ => false,
        }
    }

    /// An assignment target's *raw* storage type — deliberately does not
    /// go through [`Self::upgrade_weak`] the way an ordinary read
    /// (`Self::check_expr`) does: assigning into a `weak T` place must be
    /// checked against `weak T` itself (that's `weak T`'s only
    /// construction syntax — an implicit `T` coercion, `Self::compatible`'s
    /// own doc), not against the `Option<T>` a *read* of that same place
    /// would produce.
    fn check_assign_target_type(&mut self, target: &Expr) -> Type {
        match &target.kind {
            // `self.field`/`x.field` is one multi-segment `Path` node, not
            // `ExprKind::Field` (spec §10 — module/static/member access
            // all share `.`, so the parser only ever produces a dotted
            // -path node for a bare identifier chain like this).
            ExprKind::Path(path) => self.check_assign_path_type(path),
            ExprKind::Field {
                base,
                field: nether_ast::FieldAccessor::Named(ident),
            } => {
                let base_ty = self.check_expr(base);
                self.field_type_named(&base_ty, ident)
            }
            _ => self.check_expr(target),
        }
    }

    /// [`Self::check_assign_target_type`]'s handling for a `Path` target —
    /// mirrors [`Self::check_value_path`]'s own segment-walking loop, but
    /// the *last* segment (the actual place being assigned into) uses the
    /// raw, non-upgrading [`Self::field_type_named`] rather than
    /// [`Self::check_field_access_named`]; every earlier segment is an
    /// ordinary read on the way there, so it upgrades as normal.
    fn check_assign_path_type(&mut self, path: &Path) -> Type {
        let Some(res) = self.resolved.path_res.get(&path.id).cloned() else {
            return Type::Error;
        };
        let total = path.segments.len();
        let mut current_ty = match res.base {
            Resolution::Local(id) => self
                .locals
                .get(&id)
                .map(|(t, _)| t.clone())
                .unwrap_or(Type::Error),
            // No other resolution kind can ever name a `weak`-typed place
            // (only locals/fields can be declared `weak T`), so falling
            // back to the normal, upgrading path is safe here.
            _ => return self.check_value_path(path, None, None, None, &[]),
        };
        for i in res.consumed..total {
            let seg = &path.segments[i];
            current_ty = if i + 1 == total {
                self.field_type_named(&current_ty, seg)
            } else {
                self.check_field_access_named(&current_ty, seg)
            };
        }
        current_ty
    }

    /// The mutability of `expr`'s root local, or `None` if `expr` isn't a
    /// traceable place (a dotted Path/Field/Index chain rooted at a local)
    /// — e.g. a temporary (struct literal, call result), which can't
    /// satisfy a mutation requirement either way.
    ///
    /// `self.field`/`x.a.b.c` is one multi-segment `Path` node, not
    /// nested `ExprKind::Field`s (doc comment on
    /// [`Self::check_assign_target_type`]), so the `Path` arm below
    /// deliberately ignores `PathResolution::consumed`: `res.base`
    /// already identifies the root local regardless of how many trailing
    /// field segments follow it, and mutability of everything reachable
    /// through that root — a direct field write, or a `mut self` method
    /// call anywhere along the chain — is governed by that one root,
    /// exactly like Rust's own field-mutation rule (`holder.child.name =
    /// x` requires `holder` itself to be `mut`, transitively).
    fn place_root_mutable(&self, expr: &Expr) -> Option<bool> {
        match &expr.kind {
            ExprKind::Path(path) => {
                let res = self.resolved.path_res.get(&path.id)?;
                match res.base {
                    Resolution::Local(id) => self.locals.get(&id).map(|(_, m)| *m),
                    _ => None,
                }
            }
            ExprKind::Field { base, .. } | ExprKind::Index { base, .. } => {
                self.place_root_mutable(base)
            }
            _ => None,
        }
    }

    fn check_assign_target_mutable(&mut self, target: &Expr) {
        if let Some(false) = self.place_root_mutable(target) {
            self.err(
                target.span,
                "cannot assign to an immutable binding — declare it with `let mut`",
            );
        }
    }

    fn check_call(
        &mut self,
        call_id: NodeId,
        callee: &Expr,
        generic_args: &[Type],
        args: &[Expr],
        expected: Option<&Type>,
    ) -> Type {
        if let ExprKind::Path(path) = &callee.kind {
            let ty = self.check_value_path(path, Some(args), expected, Some(call_id), generic_args);
            self.expr_types.insert(callee.id, ty.clone());
            ty
        } else {
            if !generic_args.is_empty() {
                self.err(
                    callee.span,
                    "explicit generic arguments require a named function or method",
                );
            }
            let callee_ty = self.check_expr(callee);
            self.check_call_value(&callee_ty, args, callee.span)
        }
    }

    fn check_call_value(&mut self, ty: &Type, args: &[Expr], span: Span) -> Type {
        match ty {
            Type::Function(params, ret) => {
                if args.len() != params.len() {
                    self.err(
                        span,
                        format!(
                            "expected {} argument(s), found {}",
                            params.len(),
                            args.len()
                        ),
                    );
                }
                for (p, a) in params.iter().zip(args.iter()) {
                    let actual = self.check_expr_with_expected(a, Some(p));
                    if !actual.compatible(p) {
                        let expected = self.describe(p);
                        let found = self.describe(&actual);
                        self.err(a.span, format!("expected `{expected}`, found `{found}`"));
                    }
                }
                (**ret).clone()
            }
            Type::Error => Type::Error,
            other => {
                let desc = self.describe(other);
                self.err(span, format!("`{desc}` is not callable"));
                Type::Error
            }
        }
    }

    /// The heart of language-spec §10's disambiguation: `path.segments`
    /// beyond what `resolver` already consumed are walked here as a
    /// field/method access chain now that types are known — see
    /// `nether_resolver::PathResolution` docs.
    fn check_value_path(
        &mut self,
        path: &Path,
        call_args: Option<&[Expr]>,
        expected: Option<&Type>,
        call_id: Option<NodeId>,
        generic_args: &[Type],
    ) -> Type {
        let Some(res) = self.resolved.path_res.get(&path.id).cloned() else {
            return Type::Error;
        };
        let total = path.segments.len();
        let direct_call_args = if res.consumed == total {
            call_args
        } else {
            None
        };

        let mut current_ty = match res.base {
            Resolution::Local(id) => {
                let ty = self
                    .locals
                    .get(&id)
                    .map(|(t, _)| t.clone())
                    .unwrap_or(Type::Error);
                if res.consumed == total {
                    if let Some(args) = call_args {
                        if !generic_args.is_empty() {
                            self.err(
                                path.span,
                                "explicit generic arguments cannot be applied to a function value",
                            );
                        }
                        return self.check_call_value(&ty, args, path.span);
                    }
                }
                self.upgrade_weak(ty)
            }
            Resolution::Def(id) => self.resolve_def_value(
                id,
                path.span,
                direct_call_args,
                expected,
                call_id,
                generic_args,
            ),
            Resolution::EnumVariant(enum_id, idx) => {
                if !generic_args.is_empty() {
                    self.err(
                        path.span,
                        "enum variants do not accept function generic arguments",
                    );
                }
                self.enum_variant_value_type(enum_id, idx, path.span, direct_call_args, expected)
            }
            Resolution::StaticMember(owner_id, idx) => self.static_member_call_or_value(
                owner_id,
                idx,
                path.span,
                direct_call_args,
                call_id,
                generic_args,
            ),
            Resolution::GenericParam | Resolution::Error => Type::Error,
        };

        for i in res.consumed..total {
            let seg = &path.segments[i];
            let is_last = i + 1 == total;
            if is_last {
                if let Some(args) = call_args {
                    let receiver_mutable = match res.base {
                        Resolution::Local(id) => self.locals.get(&id).map(|(_, m)| *m),
                        _ => None,
                    };
                    return self.check_method_call_on(
                        &current_ty,
                        seg,
                        generic_args,
                        args,
                        seg.span,
                        call_id,
                        receiver_mutable,
                    );
                }
            }
            current_ty = self.check_field_access_named(&current_ty, seg);
        }
        current_ty
    }

    fn resolve_def_value(
        &mut self,
        id: DefId,
        span: Span,
        call_args: Option<&[Expr]>,
        expected: Option<&Type>,
        call_id: Option<NodeId>,
        generic_args: &[Type],
    ) -> Type {
        let def = self.resolved.definitions.get(id);
        let name = def.name.clone();
        match def.kind {
            DefKind::Primitive => {
                self.err(span, format!("`{name}` is a type, not a value"));
                Type::Error
            }
            DefKind::Interface => {
                self.err(
                    span,
                    format!("`{name}` is an interface and has no value form"),
                );
                Type::Error
            }
            DefKind::Imported => Type::Error,
            DefKind::Enum => {
                self.err(
                    span,
                    format!("`{name}` is an enum type, not a value — use one of its variants"),
                );
                Type::Error
            }
            DefKind::Fn => {
                self.resolve_fn_value(id, &name, span, call_args, expected, call_id, generic_args)
            }
            DefKind::Type => {
                if !generic_args.is_empty() {
                    self.err(
                        span,
                        "type constructors do not accept function generic arguments",
                    );
                }
                self.resolve_type_value(id, &name, span, call_args, expected)
            }
        }
    }

    fn resolve_fn_value(
        &mut self,
        id: DefId,
        name: &Symbol,
        span: Span,
        call_args: Option<&[Expr]>,
        expected: Option<&Type>,
        call_id: Option<NodeId>,
        generic_args: &[Type],
    ) -> Type {
        if name.as_str() == "println" || name.as_str() == "print" {
            if !generic_args.is_empty() {
                self.err(span, format!("builtin function `{name}` is not generic"));
            }
            return match call_args {
                Some(args) => {
                    for a in args {
                        let ty = self.check_expr(a);
                        self.require_into_string(&ty, a.span);
                    }
                    Type::unit()
                }
                None => {
                    self.err(span, format!("`{name}` must be called"));
                    Type::Error
                }
            };
        }
        match self.sigs.fns.get(&id).cloned() {
            Some(sig) => match call_args {
                Some(args) => {
                    let subst = self.check_call_args(
                        &sig,
                        args,
                        span,
                        expected,
                        None,
                        generic_args,
                        call_id,
                    );
                    substitute_generic(&sig.ret, &subst)
                }
                None if sig.generics.is_empty() => Type::Function(
                    sig.params.iter().map(|p| p.ty.clone()).collect(),
                    Box::new(sig.ret),
                ),
                None => {
                    self.err(
                        span,
                        format!(
                            "generic function `{name}` cannot be used as a value because its type arguments cannot be inferred"
                        ),
                    );
                    Type::Error
                }
            },
            None => Type::Error,
        }
    }

    fn resolve_type_value(
        &mut self,
        id: DefId,
        name: &Symbol,
        span: Span,
        call_args: Option<&[Expr]>,
        expected: Option<&Type>,
    ) -> Type {
        let generic_names = self
            .sigs
            .type_generics
            .get(&id)
            .cloned()
            .unwrap_or_default();
        let mut subst: HashMap<Symbol, Type> = HashMap::new();
        match expected {
            Some(Type::Struct(expected_id, args)) | Some(Type::TupleStruct(expected_id, args))
                if *expected_id == id && args.len() == generic_names.len() =>
            {
                subst.extend(generic_names.iter().cloned().zip(args.iter().cloned()));
            }
            _ => {}
        }
        match (self.sigs.type_shapes.get(&id).cloned(), call_args) {
            (Some(TypeShape::Unit), None) => Type::Struct(
                id,
                generic_names
                    .iter()
                    .map(|name| subst.get(name).cloned().unwrap_or(Type::Error))
                    .collect(),
            ),
            (Some(TypeShape::TupleStruct(field_tys)), Some(args)) => {
                if args.len() != field_tys.len() {
                    self.err(
                        span,
                        format!(
                            "expected {} argument(s), found {}",
                            field_tys.len(),
                            args.len()
                        ),
                    );
                }
                for (a, declared) in args.iter().zip(field_tys.iter()) {
                    let concrete_expected = substitute_generic(declared, &subst);
                    let actual = self.check_expr_with_expected(a, Some(&concrete_expected));
                    collect_generic_bindings(declared, &actual, &mut subst);
                    let concrete_expected = substitute_generic(declared, &subst);
                    if !actual.compatible(&concrete_expected) {
                        let expected_s = self.describe(&concrete_expected);
                        let found_s = self.describe(&actual);
                        self.err(
                            a.span,
                            format!("expected `{expected_s}`, found `{found_s}`"),
                        );
                    }
                }
                Type::TupleStruct(
                    id,
                    generic_names
                        .iter()
                        .map(|name| subst.get(name).cloned().unwrap_or(Type::Error))
                        .collect(),
                )
            }
            _ => {
                self.err(
                    span,
                    format!(
                        "`{name}` cannot be used this way — construct it with `{{ .. }}` (struct) or `(..)` \
                         (tuple-struct) syntax matching its declaration"
                    ),
                );
                Type::Error
            }
        }
    }

    fn enum_variant_value_type(
        &mut self,
        enum_id: DefId,
        idx: u32,
        span: Span,
        call_args: Option<&[Expr]>,
        expected: Option<&Type>,
    ) -> Type {
        let Some(sig) = self.sigs.enum_sigs.get(&enum_id) else {
            self.err(
                span,
                "internal type information for this enum is unavailable",
            );
            return Type::Error;
        };
        let Some((_, payload)) = sig.variants.get(idx as usize) else {
            self.err(span, "unknown enum variant");
            return Type::Error;
        };
        let payload = payload.clone();
        let generic_names = sig.generics.clone();
        let mut subst = HashMap::new();
        if let Some(Type::Enum(expected_id, args)) = expected {
            if *expected_id == enum_id && args.len() == generic_names.len() {
                subst.extend(generic_names.iter().cloned().zip(args.iter().cloned()));
            }
        }
        if !payload.is_empty() {
            match call_args {
                Some(args) => {
                    if args.len() != payload.len() {
                        self.err(
                            span,
                            format!(
                                "variant expects {} argument(s), found {}",
                                payload.len(),
                                args.len()
                            ),
                        );
                    }
                    for (a, declared) in args.iter().zip(payload.iter()) {
                        let concrete_expected = substitute_generic(declared, &subst);
                        let actual = self.check_expr_with_expected(a, Some(&concrete_expected));
                        collect_generic_bindings(declared, &actual, &mut subst);
                        let concrete_expected = substitute_generic(declared, &subst);
                        if !actual.compatible(&concrete_expected) {
                            let expected_s = self.describe(&concrete_expected);
                            let found_s = self.describe(&actual);
                            self.err(
                                a.span,
                                format!("expected `{expected_s}`, found `{found_s}`"),
                            );
                        }
                    }
                }
                None => self.err(span, "this variant requires payload arguments"),
            }
        }
        Type::Enum(
            enum_id,
            generic_names
                .iter()
                .map(|name| subst.get(name).cloned().unwrap_or(Type::Error))
                .collect(),
        )
    }

    fn static_member_call_or_value(
        &mut self,
        owner_id: DefId,
        idx: u32,
        span: Span,
        call_args: Option<&[Expr]>,
        call_id: Option<NodeId>,
        generic_args: &[Type],
    ) -> Type {
        let Some(name) = self
            .resolved
            .definitions
            .get(owner_id)
            .methods
            .get(idx as usize)
            .cloned()
        else {
            return Type::Error;
        };
        let Some(sig) = self.sigs.method(owner_id, &name).cloned() else {
            return Type::Error;
        };
        if sig.self_param.is_some() {
            self.err(
                span,
                format!("`{name}` is an instance method and cannot be called as a static member"),
            );
            return Type::Error;
        }
        match call_args {
            Some(args) => {
                let subst =
                    self.check_call_args(&sig, args, span, None, None, generic_args, call_id);
                substitute_generic(&sig.ret, &subst)
            }
            None => Type::Function(
                sig.params.iter().map(|p| p.ty.clone()).collect(),
                Box::new(sig.ret),
            ),
        }
    }

    fn check_method_call_on(
        &mut self,
        base_ty: &Type,
        method: &Ident,
        generic_args: &[Type],
        args: &[Expr],
        span: Span,
        call_id: Option<NodeId>,
        receiver_mutable: Option<bool>,
    ) -> Type {
        if let Type::Generic(name) = base_ty {
            return self.check_generic_method_call(name, method, generic_args, args, span, call_id);
        }
        if let Type::Array(elem_ty) = base_ty {
            if matches!(method.name.as_str(), "len" | "push" | "pop") {
                if !generic_args.is_empty() {
                    self.err(method.span, "array methods are not generic");
                }
                if matches!(method.name.as_str(), "push" | "pop")
                    && receiver_mutable != Some(true)
                {
                    self.err(method.span, "cannot call a `mut self` method through an immutable receiver — declare it with `let mut`");
                }
                return self.check_array_method_call(elem_ty, method, args, span);
            }
        }
        let owner_id = match base_ty {
            Type::Struct(id, _) | Type::TupleStruct(id, _) => Some(*id),
            Type::Enum(id, _) => Some(*id),
            Type::Array(_) => self.resolved.definitions.lookup(&Symbol::new("Array")),
            _ => None,
        };
        let Some(owner_id) = owner_id else {
            if !base_ty.is_error() {
                let desc = self.describe(base_ty);
                self.err(
                    method.span,
                    format!("`{desc}` has no method named `{}`", method.name),
                );
            }
            return Type::Error;
        };
        // Specialization-aware: an exact-match concrete override
        // (`impl Option<i32> { ... }`) wins over the generic fallback —
        // see `MethodSet::for_args`. `receiver_args` still containing an
        // unsubstituted `Type::Generic` (this call site is itself inside
        // another still-generic function) can never exactly match a
        // specialization, so it naturally resolves to the generic sig
        // here; `monomorphization` re-resolves the same way once the
        // enclosing function is instantiated for a concrete type and picks
        // up the override then (`docs/generics.md` § "Methods on generic
        // types").
        let receiver_args: &[Type] = match base_ty {
            Type::Struct(_, args) | Type::TupleStruct(_, args) | Type::Enum(_, args) => args,
            Type::Array(elem) => std::slice::from_ref(elem.as_ref()),
            _ => &[],
        };
        let Some(method_set) = self.sigs.methods.get(&(owner_id, method.name.clone())) else {
            let desc = self.describe(base_ty);
            self.err(
                method.span,
                format!("`{desc}` has no method named `{}`", method.name),
            );
            return Type::Error;
        };
        let is_specialized = method_set
            .specializations
            .iter()
            .any(|(args, _)| args.as_slice() == receiver_args);
        let Some(sig) = method_set.for_args(receiver_args).cloned() else {
            let desc = self.describe(base_ty);
            self.err(
                method.span,
                format!("`{desc}` has no method named `{}`", method.name),
            );
            return Type::Error;
        };
        if sig.self_param == Some(SelfParam::ByMutRef) && receiver_mutable != Some(true) {
            self.err(method.span, "cannot call a `mut self` method through an immutable receiver — declare it with `let mut`");
        }
        // Build a placeholder `Owner<T, ...>` to structurally match against
        // `base_ty`'s concrete arguments below, binding each owner
        // parameter from the receiver. The names must be `sig`'s own — not
        // re-read from a declaration — since a builtin owner such as
        // `Option` has none; `sig.generics`' first `owner_arity` entries
        // are exactly the owner's parameters, in the receiver's positional
        // order, however this particular method's `impl` block happened to
        // name them (`build_impl_methods` always splices them in first). A
        // specialized `sig` has no owner placeholders at all — everything
        // in it is already concrete — so there is nothing to bind.
        let owner_arity = if is_specialized { 0 } else { receiver_args.len() };
        let owner_names = sig.generics.iter().take(owner_arity).map(|(name, _)| Type::Generic(name.clone()));
        let owner_pattern = match base_ty {
            Type::Struct(id, _) => Type::Struct(*id, owner_names.collect()),
            Type::TupleStruct(id, _) => Type::TupleStruct(*id, owner_names.collect()),
            Type::Enum(id, _) => Type::Enum(*id, owner_names.collect()),
            Type::Array(_) => {
                Type::Array(Box::new(owner_names.into_iter().next().unwrap_or(Type::Error)))
            }
            _ => unreachable!("owner_id was only set for Struct/TupleStruct/Enum/Array above"),
        };
        let mut owner_subst = HashMap::new();
        collect_generic_bindings(&owner_pattern, base_ty, &mut owner_subst);
        let subst = self.check_call_args(
            &sig,
            args,
            span,
            None,
            Some(owner_subst),
            generic_args,
            call_id,
        );
        substitute_generic(&sig.ret, &subst)
    }

    fn check_generic_method_call(
        &mut self,
        name: &Symbol,
        method: &Ident,
        generic_args: &[Type],
        args: &[Expr],
        span: Span,
        call_id: Option<NodeId>,
    ) -> Type {
        let bound = self.generics.get(name).cloned().flatten();
        if let Some(bound) = bound {
            let bound_name = self.resolved.definitions.get(bound.interface).name.as_str();
            if bound_name == "Into"
                && bound.args == [Type::String]
                && method.name.as_str() == "into_string"
            {
                if !generic_args.is_empty() {
                    self.err(method.span, "`into_string` is not generic");
                }
                if !args.is_empty() {
                    self.err(
                        span,
                        format!("expected 0 argument(s), found {}", args.len()),
                    );
                }
                return Type::String;
            }
            if let Some(raw_sig) = self
                .sigs
                .interface_methods
                .get(&(bound.interface, method.name.clone()))
                .cloned()
            {
                let interface_subst: HashMap<Symbol, Type> = self
                    .sigs
                    .interface_generics
                    .get(&bound.interface)
                    .into_iter()
                    .flatten()
                    .cloned()
                    .zip(bound.args)
                    .collect();
                let sig = specialize_fn_sig(&raw_sig, &interface_subst);
                let subst =
                    self.check_call_args(&sig, args, span, None, None, generic_args, call_id);
                return substitute_generic(&sig.ret, &subst);
            }
        }
        self.err(
            method.span,
            format!(
                "no method named `{}` found for generic type `{name}`",
                method.name
            ),
        );
        Type::Error
    }

    /// `Array<T>`'s runtime methods (language-spec §3.5: "Runtime methods:
    /// push, pop, len") — provided by `runtime/array`, not by any `impl`
    /// block a user could write, so there is no [`crate::sig::FnSig`] for
    /// them in [`Signatures::methods`] to look up; special-cased here the
    /// same way `println`/`print` are special-cased in
    /// [`Checker::resolve_fn_value`].
    fn check_array_method_call(
        &mut self,
        elem_ty: &Type,
        method: &Ident,
        args: &[Expr],
        span: Span,
    ) -> Type {
        match method.name.as_str() {
            "len" => {
                if !args.is_empty() {
                    self.err(
                        span,
                        format!("expected 0 argument(s), found {}", args.len()),
                    );
                }
                Type::Primitive(PrimitiveKind::Usize)
            }
            "push" => {
                if args.len() != 1 {
                    self.err(
                        span,
                        format!("expected 1 argument(s), found {}", args.len()),
                    );
                }
                if let Some(arg) = args.first() {
                    let actual = self.check_expr_with_expected(arg, Some(elem_ty));
                    if !actual.compatible(elem_ty) {
                        let expected_s = self.describe(elem_ty);
                        let found_s = self.describe(&actual);
                        self.err(
                            arg.span,
                            format!("expected `{expected_s}`, found `{found_s}`"),
                        );
                    }
                }
                Type::unit()
            }
            "pop" => {
                if !args.is_empty() {
                    self.err(
                        span,
                        format!("expected 0 argument(s), found {}", args.len()),
                    );
                }
                match self.resolved.definitions.lookup(&Symbol::new("Option")) {
                    Some(option_id) => Type::Enum(option_id, vec![elem_ty.clone()]),
                    None => Type::Error,
                }
            }
            other => {
                self.err(
                    method.span,
                    format!("`Array` has no method named `{other}`"),
                );
                Type::Error
            }
        }
    }

    /// Reading a `weak T`-typed place *as a value* never yields a bare
    /// `weak T` — it yields `Option<T>` directly, forcing the "might be
    /// gone" case to be handled at every use (spec §3.4, `arc-model.md`
    /// §3.5). This is the one place that conversion happens: every read
    /// site (a bare local reference, a field access) routes its result
    /// through this before returning it, so `weak T` never actually
    /// appears as an expression's type — only as a `let`/field/parameter
    /// declared *storage* type, which is a different thing entirely (see
    /// `Self::compatible`'s own doc for the construction side).
    fn upgrade_weak(&self, ty: Type) -> Type {
        match ty {
            Type::Weak(inner) => match self.resolved.definitions.lookup(&Symbol::new("Option")) {
                Some(option_id) => Type::Enum(option_id, vec![*inner]),
                None => Type::Error,
            },
            other => other,
        }
    }

    fn check_field_access(&mut self, base_ty: &Type, field: &nether_ast::FieldAccessor) -> Type {
        match field {
            nether_ast::FieldAccessor::Named(ident) => {
                self.check_field_access_named(base_ty, ident)
            }
            nether_ast::FieldAccessor::Index(idx, span) => {
                self.check_tuple_index(base_ty, *idx, *span)
            }
        }
    }

    fn check_field_access_named(&mut self, base_ty: &Type, ident: &Ident) -> Type {
        let ty = self.field_type_named(base_ty, ident);
        self.upgrade_weak(ty)
    }

    /// The field's raw declared type — shared by [`Self::check_field_access_named`]
    /// (a normal read, which upgrades a `weak` result) and
    /// [`Self::check_assign_target_type`] (an assignment target, which
    /// must not: the new value needs checking against the real `weak T`
    /// storage type, not the `Option<T>` reading it back out would give).
    fn field_type_named(&mut self, base_ty: &Type, ident: &Ident) -> Type {
        if let Type::Struct(_, _) = base_ty {
            if let Some(fields) = self.sigs.named_type_fields(base_ty) {
                if let Some((_, ty)) = fields.into_iter().find(|(n, _)| n == &ident.name) {
                    return ty.clone();
                }
            }
        }
        if base_ty.is_error() {
            return Type::Error;
        }
        let desc = self.describe(base_ty);
        self.err(
            ident.span,
            format!("`{desc}` has no field named `{}`", ident.name),
        );
        Type::Error
    }

    fn check_tuple_index(&mut self, base_ty: &Type, idx: u32, span: Span) -> Type {
        let elems: Option<Vec<Type>> = match base_ty {
            Type::Tuple(elems) => Some(elems.clone()),
            Type::TupleStruct(_, _) => self.sigs.type_fields(base_ty),
            _ => None,
        };
        match elems.and_then(|e| e.get(idx as usize).cloned()) {
            Some(ty) => ty,
            None => {
                if !base_ty.is_error() {
                    let desc = self.describe(base_ty);
                    self.err(span, format!("`{desc}` has no field `.{idx}`"));
                }
                Type::Error
            }
        }
    }

    fn check_index(&mut self, base: &Expr, index: &Expr) -> Type {
        let base_ty = self.check_expr(base);
        let index_ty = self.check_expr(index);
        if !matches!(&index_ty, Type::Primitive(p) if p.is_integer()) && !index_ty.is_error() {
            self.err(index.span, "array index must be an integer type");
        }
        match base_ty {
            Type::Array(inner) => *inner,
            Type::Error => Type::Error,
            other => {
                let desc = self.describe(&other);
                self.err(base.span, format!("`{desc}` cannot be indexed"));
                Type::Error
            }
        }
    }

    fn check_if(
        &mut self,
        cond: &Expr,
        then_branch: &Block,
        else_branch: &Option<Box<Expr>>,
        expected: Option<&Type>,
    ) -> Type {
        let cond_ty = self.check_expr(cond);
        self.require_bool(&cond_ty, cond.span, "`if` condition");
        let then_ty = self.check_block_with_expected(then_branch, expected);
        match else_branch {
            Some(e) => {
                let else_expected = expected.or(Some(&then_ty));
                let else_ty = self.check_expr_with_expected(e, else_expected);
                if !then_ty.compatible(&else_ty) {
                    let then_s = self.describe(&then_ty);
                    let else_s = self.describe(&else_ty);
                    self.err(e.span, format!("`if`/`else` branches have incompatible types: `{then_s}` vs `{else_s}`"));
                }
                let result = if matches!(then_ty, Type::Never) {
                    else_ty
                } else {
                    prefer_concrete_type(then_ty, else_ty)
                };
                if let Some(tail) = &then_branch.tail {
                    if self
                        .expr_types
                        .get(&tail.id)
                        .is_some_and(|ty| ty.compatible(&result) && ty.contains_error())
                    {
                        self.expr_types.insert(tail.id, result.clone());
                    }
                }
                result
            }
            None => Type::unit(),
        }
    }

    fn check_match(
        &mut self,
        scrutinee: &Expr,
        arms: &[nether_ast::MatchArm],
        span: Span,
        expected: Option<&Type>,
    ) -> Type {
        let scrutinee_ty = self.check_expr(scrutinee);
        let mut result_ty: Option<Type> = None;
        let mut covered: HashSet<u32> = HashSet::new();
        let mut has_catch_all = false;
        for arm in arms {
            self.check_pattern(
                &arm.pattern,
                &scrutinee_ty,
                &mut covered,
                &mut has_catch_all,
            );
            let arm_expected = expected.or(result_ty.as_ref());
            let body_ty = self.check_expr_with_expected(&arm.body, arm_expected);
            result_ty = Some(match result_ty {
                None => body_ty,
                Some(prev) => {
                    if !prev.compatible(&body_ty) {
                        let prev_s = self.describe(&prev);
                        let body_s = self.describe(&body_ty);
                        self.err(
                            arm.body.span,
                            format!(
                                "`match` arms have incompatible types: `{prev_s}` vs `{body_s}`"
                            ),
                        );
                    }
                    if matches!(prev, Type::Never) {
                        body_ty
                    } else {
                        prefer_concrete_type(prev, body_ty)
                    }
                }
            });
        }
        if let Type::Enum(enum_id, _) = &scrutinee_ty {
            if !has_catch_all {
                if let Some(enum_sig) = self.sigs.enum_sigs.get(enum_id) {
                    let missing: Vec<&str> = enum_sig
                        .variants
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| !covered.contains(&(*i as u32)))
                        .map(|(_, (name, _))| name.as_str())
                        .collect();
                    if !missing.is_empty() {
                        self.err(
                            span,
                            format!(
                                "match is not exhaustive: missing variant(s) {}",
                                missing.join(", ")
                            ),
                        );
                    }
                }
            }
        }
        let result = result_ty.unwrap_or_else(Type::unit);
        // Context-free unit variants such as `Option.None` initially
        // carry unknown generic arguments. Once all arms determine the
        // match result, give those direct arm expressions the concrete
        // enum type so HIR/MIR/codegen use one consistent layout.
        for arm in arms {
            if let Some(arm_ty) = self.expr_types.get(&arm.body.id) {
                if arm_ty.compatible(&result)
                    && matches!(arm_ty, Type::Enum(_, args) if args.iter().any(Type::is_error))
                {
                    self.expr_types.insert(arm.body.id, result.clone());
                }
            }
        }
        result
    }

    fn check_pattern(
        &mut self,
        pattern: &Pattern,
        scrutinee_ty: &Type,
        covered: &mut HashSet<u32>,
        has_catch_all: &mut bool,
    ) {
        self.check_pattern_inner(pattern, scrutinee_ty, covered, has_catch_all, true);
    }

    fn check_pattern_inner(
        &mut self,
        pattern: &Pattern,
        scrutinee_ty: &Type,
        covered: &mut HashSet<u32>,
        has_catch_all: &mut bool,
        top_level: bool,
    ) {
        match pattern {
            Pattern::Wildcard(_) => {
                if top_level {
                    *has_catch_all = true;
                }
            }
            Pattern::Binding(id, _ident) => {
                if self.resolved.locals.contains_key(id) {
                    self.bind_local(*id, scrutinee_ty.clone(), false);
                    if top_level {
                        *has_catch_all = true;
                    }
                } else if let Some(res) = self.resolved.path_res.get(id) {
                    if let Resolution::EnumVariant(enum_id, idx) = res.base {
                        self.check_variant_pattern(
                            pattern.span(),
                            enum_id,
                            idx,
                            &[],
                            scrutinee_ty,
                            covered,
                            has_catch_all,
                            top_level,
                        );
                    }
                }
            }
            Pattern::Literal(literal, span) => {
                let compatible = match literal {
                    Literal::Int(_) => {
                        matches!(scrutinee_ty, Type::Primitive(kind) if kind.is_integer())
                    }
                    Literal::Float(_) => {
                        matches!(scrutinee_ty, Type::Primitive(kind) if kind.is_float())
                    }
                    Literal::Bool(_) => {
                        matches!(scrutinee_ty, Type::Primitive(PrimitiveKind::Bool))
                    }
                    Literal::Char(_) => {
                        matches!(scrutinee_ty, Type::Primitive(PrimitiveKind::Char))
                    }
                    Literal::Str(_) => matches!(scrutinee_ty, Type::String),
                };
                if !compatible && !scrutinee_ty.is_error() {
                    let found = self.describe(scrutinee_ty);
                    self.err(
                        *span,
                        format!("literal pattern is incompatible with `{found}`"),
                    );
                }
            }
            Pattern::Tuple(elems, span) => {
                let elem_tys: Vec<Type> = match scrutinee_ty {
                    Type::Tuple(tys) => {
                        if tys.len() != elems.len() {
                            self.err(
                                *span,
                                format!(
                                    "tuple pattern has {} element(s), but the scrutinee has {}",
                                    elems.len(),
                                    tys.len()
                                ),
                            );
                        }
                        tys.clone()
                    }
                    Type::Error => vec![Type::Error; elems.len()],
                    other => {
                        let found = self.describe(other);
                        self.err(
                            *span,
                            format!("tuple pattern requires a tuple, found `{found}`"),
                        );
                        vec![Type::Error; elems.len()]
                    }
                };
                for (index, pattern) in elems.iter().enumerate() {
                    let ty = elem_tys.get(index).cloned().unwrap_or(Type::Error);
                    self.check_pattern_inner(pattern, &ty, covered, has_catch_all, false);
                }
            }
            Pattern::Variant { path, payload, .. } => {
                if let Some(res) = self.resolved.path_res.get(&path.id) {
                    if let Resolution::EnumVariant(enum_id, idx) = res.base {
                        self.check_variant_pattern(
                            pattern.span(),
                            enum_id,
                            idx,
                            payload,
                            scrutinee_ty,
                            covered,
                            has_catch_all,
                            top_level,
                        );
                    }
                }
            }
        }
    }

    fn check_variant_pattern(
        &mut self,
        span: Span,
        enum_id: DefId,
        variant: u32,
        payload: &[Pattern],
        scrutinee_ty: &Type,
        covered: &mut HashSet<u32>,
        has_catch_all: &mut bool,
        top_level: bool,
    ) {
        let same_enum =
            matches!(scrutinee_ty, Type::Enum(scrutinee_id, _) if *scrutinee_id == enum_id);
        if !same_enum {
            if !scrutinee_ty.is_error() {
                let pattern_enum = self.resolved.definitions.get(enum_id).name.to_string();
                let found = self.describe(scrutinee_ty);
                self.err(
                    span,
                    format!("variant pattern from enum `{pattern_enum}` cannot match `{found}`"),
                );
            }
        } else if top_level {
            covered.insert(variant);
        }

        let declared_arity = self
            .sigs
            .enum_sigs
            .get(&enum_id)
            .and_then(|sig| sig.variants.get(variant as usize))
            .map_or(0, |(_, fields)| fields.len());
        if payload.len() != declared_arity {
            self.err(
                span,
                format!(
                    "variant pattern expects {declared_arity} payload field(s), found {}",
                    payload.len()
                ),
            );
        }

        let payload_tys = if same_enum {
            self.sigs
                .enum_payload(scrutinee_ty, variant)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        for (index, pattern) in payload.iter().enumerate() {
            let ty = payload_tys.get(index).cloned().unwrap_or(Type::Error);
            self.check_pattern_inner(pattern, &ty, covered, has_catch_all, false);
        }
    }

    fn check_for_in(&mut self, pattern: &Pattern, iter: &Expr, body: &Block) -> Type {
        let iter_ty = self.check_expr(iter);
        let elem_ty = match &iter_ty {
            Type::Array(inner) => (**inner).clone(),
            Type::Error => Type::Error,
            other => {
                let desc = self.describe(other);
                self.err(
                    iter.span,
                    format!("`for`-`in` requires an `Array`, found `{desc}`"),
                );
                Type::Error
            }
        };
        let mut covered = HashSet::new();
        let mut has_catch_all = false;
        self.check_pattern(pattern, &elem_ty, &mut covered, &mut has_catch_all);
        self.loop_depth += 1;
        self.check_block(body);
        self.loop_depth -= 1;
        Type::unit()
    }

    fn check_return(&mut self, value: &Option<Box<Expr>>, span: Span) -> Type {
        let expected_ret = self.return_ty.clone();
        let actual = match value {
            Some(v) => self.check_expr_with_expected(v, Some(&expected_ret)),
            None => Type::unit(),
        };
        if !actual.compatible(&expected_ret) {
            let expected_s = self.describe(&expected_ret);
            let found_s = self.describe(&actual);
            self.err(
                span,
                format!("expected return type `{expected_s}`, found `{found_s}`"),
            );
        }
        Type::Never
    }

    fn check_closure(
        &mut self,
        params: &[nether_ast::Param],
        body: &Expr,
        expected: Option<&Type>,
    ) -> Type {
        let param_tys: Vec<Type> = params
            .iter()
            .map(|p| lower_type_expr(&p.ty, self.resolved, self.decls, self.diagnostics))
            .collect();
        for (p, t) in params.iter().zip(param_tys.iter()) {
            self.bind_local(p.id, t.clone(), p.mutable);
        }
        let expected_ret = match expected {
            Some(Type::Function(expected_params, expected_ret))
                if expected_params.len() == param_tys.len()
                    && expected_params
                        .iter()
                        .zip(&param_tys)
                        .all(|(expected, actual)| actual.compatible(expected)) =>
            {
                Some(expected_ret.as_ref())
            }
            _ => None,
        };
        let outer_loop_depth = std::mem::replace(&mut self.loop_depth, 0);
        let ret_ty = self.check_expr_with_expected(body, expected_ret);
        self.loop_depth = outer_loop_depth;
        Type::Function(param_tys, Box::new(ret_ty))
    }

    fn check_struct_lit(
        &mut self,
        path: &Path,
        fields: &[(Ident, Expr)],
        expected: Option<&Type>,
    ) -> Type {
        let Some(res) = self.resolved.path_res.get(&path.id).cloned() else {
            return Type::Error;
        };
        let Resolution::Def(id) = res.base else {
            if !matches!(res.base, Resolution::Error) {
                self.err(path.span, "struct literal path does not name a type");
            }
            return Type::Error;
        };
        let def = self.resolved.definitions.get(id);
        if def.kind != DefKind::Type {
            self.err(path.span, format!("`{}` is not a struct type", def.name));
            return Type::Error;
        }
        let name = def.name.clone();
        let Some(TypeShape::Struct(decl_fields)) = self.sigs.type_shapes.get(&id).cloned() else {
            self.err(
                path.span,
                format!("`{name}` is not a struct with named fields"),
            );
            return Type::Error;
        };
        let generic_names = self
            .sigs
            .type_generics
            .get(&id)
            .cloned()
            .unwrap_or_default();
        let mut subst: HashMap<Symbol, Type> = HashMap::new();
        if let Some(Type::Struct(expected_id, args)) = expected {
            if *expected_id == id && args.len() == generic_names.len() {
                subst.extend(generic_names.iter().cloned().zip(args.iter().cloned()));
            }
        }
        let mut seen = HashSet::new();
        for (fname, value) in fields {
            seen.insert(fname.name.clone());
            if let Some((_, declared)) = decl_fields.iter().find(|(n, _)| n == &fname.name) {
                let concrete_expected = substitute_generic(declared, &subst);
                let actual = self.check_expr_with_expected(value, Some(&concrete_expected));
                collect_generic_bindings(declared, &actual, &mut subst);
                let concrete_expected = substitute_generic(declared, &subst);
                if !actual.compatible(&concrete_expected) {
                    let expected_s = self.describe(&concrete_expected);
                    let found_s = self.describe(&actual);
                    self.err(
                        value.span,
                        format!(
                            "expected `{expected_s}`, found `{found_s}` for field `{}`",
                            fname.name
                        ),
                    );
                }
            } else {
                self.err(
                    fname.span,
                    format!("`{name}` has no field named `{}`", fname.name),
                );
                self.check_expr(value);
            }
        }
        for (field_name, _) in &decl_fields {
            if !seen.contains(field_name) {
                self.err(
                    path.span,
                    format!("missing field `{field_name}` in struct literal for `{name}`"),
                );
            }
        }
        Type::Struct(
            id,
            generic_names
                .iter()
                .map(|name| subst.get(name).cloned().unwrap_or(Type::Error))
                .collect(),
        )
    }

    fn check_call_args(
        &mut self,
        sig: &FnSig,
        args: &[Expr],
        call_span: Span,
        expected_return: Option<&Type>,
        initial_subst: Option<HashMap<Symbol, Type>>,
        explicit_generic_args: &[Type],
        call_id: Option<NodeId>,
    ) -> HashMap<Symbol, Type> {
        // A trailing variadic parameter (`args: ...String`) collects zero
        // or more trailing call-site arguments — checked separately below
        // against its *element* type, since `sig.params.len()` no longer
        // says how many arguments the call needs.
        let variadic = sig.params.last().is_some_and(|p| p.variadic);
        let fixed_params = if variadic {
            &sig.params[..sig.params.len() - 1]
        } else {
            sig.params.as_slice()
        };
        if variadic {
            if args.len() < fixed_params.len() {
                self.err(
                    call_span,
                    format!(
                        "expected at least {} argument(s), found {}",
                        fixed_params.len(),
                        args.len()
                    ),
                );
            }
        } else if args.len() != sig.params.len() {
            self.err(
                call_span,
                format!(
                    "expected {} argument(s), found {}",
                    sig.params.len(),
                    args.len()
                ),
            );
        }
        let mut subst = initial_subst.unwrap_or_default();
        if !explicit_generic_args.is_empty() {
            let remaining: Vec<Symbol> = sig
                .generics
                .iter()
                .map(|(name, _)| name)
                .filter(|name| !subst.contains_key(*name))
                .cloned()
                .collect();
            if explicit_generic_args.len() != remaining.len() {
                self.err(
                    call_span,
                    format!(
                        "expected {} explicit generic argument(s), found {}",
                        remaining.len(),
                        explicit_generic_args.len()
                    ),
                );
            }
            subst.extend(
                remaining
                    .into_iter()
                    .zip(explicit_generic_args.iter().cloned()),
            );
        }
        if let Some(expected_return) = expected_return {
            collect_generic_bindings(&sig.ret, expected_return, &mut subst);
        }
        let fixed_arg_count = args.len().min(fixed_params.len());
        for (param, arg) in fixed_params.iter().zip(&args[..fixed_arg_count]) {
            let (inner_expr, is_mut_arg) = match &arg.kind {
                ExprKind::MutArg(inner) => (inner.as_ref(), true),
                _ => (arg, false),
            };
            if param.mutable != is_mut_arg {
                if param.mutable {
                    self.err(arg.span, format!("parameter `{}` is `mut`; the caller must also write `mut` at the call site", param.name));
                } else {
                    self.err(
                        arg.span,
                        "`mut` is only valid for arguments passed to a `mut` parameter",
                    );
                }
            } else if param.mutable {
                self.check_mut_arg_target(inner_expr);
            }
            let expected = substitute_generic(&param.ty, &subst);
            let actual = self.check_expr_with_expected(inner_expr, Some(&expected));
            if is_mut_arg {
                self.expr_types.insert(arg.id, actual.clone());
            }
            collect_generic_bindings(&param.ty, &actual, &mut subst);
            let expected = substitute_generic(&param.ty, &subst);
            if !actual.compatible(&expected) {
                let expected_s = self.describe(&expected);
                let found_s = self.describe(&actual);
                self.err(
                    inner_expr.span,
                    format!("expected `{expected_s}`, found `{found_s}`"),
                );
            }
        }
        if variadic {
            let elem_ty = &sig.params[sig.params.len() - 1].ty;
            for arg in &args[fixed_arg_count..] {
                if matches!(arg.kind, ExprKind::MutArg(_)) {
                    self.err(arg.span, "`mut` is not valid for a variadic argument");
                    continue;
                }
                let expected = substitute_generic(elem_ty, &subst);
                let actual = self.check_expr_with_expected(arg, Some(&expected));
                if matches!(expected, Type::String) {
                    // Mirrors template-string interpolation: any
                    // `Into<String>` value is accepted here, not just a
                    // literal `String` — `nether_hir::into_string_expr`
                    // performs the matching conversion when lowering this
                    // same call.
                    self.require_into_string(&actual, arg.span);
                } else {
                    collect_generic_bindings(elem_ty, &actual, &mut subst);
                    let expected = substitute_generic(elem_ty, &subst);
                    if !actual.compatible(&expected) {
                        let expected_s = self.describe(&expected);
                        let found_s = self.describe(&actual);
                        self.err(
                            arg.span,
                            format!("expected `{expected_s}`, found `{found_s}`"),
                        );
                    }
                }
            }
        }
        for (name, bound) in &sig.generics {
            let Some(concrete) = subst.get(name) else {
                self.err(
                    call_span,
                    format!("cannot infer generic parameter `{name}` from this call's arguments"),
                );
                continue;
            };
            let Some(bound) = bound else { continue };
            let concrete_bound = GenericBound {
                interface: bound.interface,
                args: bound
                    .args
                    .iter()
                    .map(|arg| substitute_generic(arg, &subst))
                    .collect(),
            };
            let satisfies = self.type_satisfies_bound(concrete, &concrete_bound);
            if !satisfies {
                let concrete_s = self.describe(concrete);
                let iface_name = self.describe_bound(&concrete_bound);
                self.err(call_span, format!("`{concrete_s}` does not implement `{iface_name}`, required by generic parameter `{name}`"));
            }
        }
        if let Some(call_id) = call_id {
            if !sig.generics.is_empty() {
                self.call_generic_args.insert(
                    call_id,
                    sig.generics
                        .iter()
                        .map(|(name, _)| subst.get(name).cloned().unwrap_or(Type::Error))
                        .collect(),
                );
            }
        }
        subst
    }

    fn validate_type_bounds(&mut self, ty: &Type, span: Span) {
        match ty {
            Type::Struct(id, args) | Type::TupleStruct(id, args) | Type::Enum(id, args) => {
                let names = self
                    .sigs
                    .type_generics
                    .get(id)
                    .cloned()
                    .or_else(|| self.sigs.enum_sigs.get(id).map(|sig| sig.generics.clone()))
                    .unwrap_or_default();
                let bounds = self
                    .sigs
                    .generic_type_bounds
                    .get(id)
                    .cloned()
                    .unwrap_or_default();
                let subst: HashMap<Symbol, Type> =
                    names.into_iter().zip(args.iter().cloned()).collect();
                for (index, bound) in bounds.into_iter().enumerate() {
                    let Some(bound) = bound else { continue };
                    let Some(actual) = args.get(index) else {
                        continue;
                    };
                    if actual.contains_error() {
                        continue;
                    }
                    let concrete_bound = GenericBound {
                        interface: bound.interface,
                        args: bound
                            .args
                            .iter()
                            .map(|arg| substitute_generic(arg, &subst))
                            .collect(),
                    };
                    if !self.type_satisfies_bound(actual, &concrete_bound) {
                        let actual = self.describe(actual);
                        let bound = self.describe_bound(&concrete_bound);
                        self.err(
                            span,
                            format!(
                                "`{actual}` does not implement `{bound}`, required by this generic type"
                            ),
                        );
                    }
                }
                for arg in args {
                    self.validate_type_bounds(arg, span);
                }
            }
            Type::Tuple(items) => {
                for item in items {
                    self.validate_type_bounds(item, span);
                }
            }
            Type::Array(inner) | Type::Weak(inner) => {
                self.validate_type_bounds(inner, span);
            }
            Type::Function(params, ret) => {
                for param in params {
                    self.validate_type_bounds(param, span);
                }
                self.validate_type_bounds(ret, span);
            }
            _ => {}
        }
    }

    fn describe_bound(&self, bound: &GenericBound) -> String {
        let name = self
            .resolved
            .definitions
            .get(bound.interface)
            .name
            .to_string();
        if bound.args.is_empty() {
            name
        } else {
            format!(
                "{name}<{}>",
                bound
                    .args
                    .iter()
                    .map(|arg| self.describe(arg))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    }

    fn is_into_string_bound(&self, bound: &GenericBound) -> bool {
        self.resolved.definitions.get(bound.interface).name.as_str() == "Into"
            && bound.args == [Type::String]
    }

    fn type_satisfies_bound(&self, ty: &Type, bound: &GenericBound) -> bool {
        if self.is_into_string_bound(bound) {
            return self.is_into_string_convertible(ty);
        }
        match ty {
            Type::Struct(_, _) | Type::TupleStruct(_, _) | Type::Enum(_, _) => {
                self.sigs.satisfies(ty, bound)
            }
            Type::Generic(name) => self
                .generics
                .get(name)
                .and_then(Option::as_ref)
                .is_some_and(|actual| self.sigs.bound_satisfies(actual, bound)),
            Type::Error => true,
            _ => false,
        }
    }

    fn is_into_string_convertible(&self, ty: &Type) -> bool {
        match ty {
            Type::String | Type::Primitive(_) | Type::Error => true,
            Type::Struct(id, _) | Type::TupleStruct(id, _) | Type::Enum(id, _) => {
                let into = self.resolved.definitions.lookup(&Symbol::new("Into"));
                let bound = into.map(|interface| GenericBound {
                    interface,
                    args: vec![Type::String],
                });
                bound
                    .as_ref()
                    .is_some_and(|bound| self.sigs.satisfies(ty, bound))
                    && self
                        .sigs
                        .method(*id, &Symbol::new("into_string"))
                        .is_some_and(|sig| sig.self_param.is_some() && sig.ret == Type::String)
            }
            Type::Generic(name) => self
                .generics
                .get(name)
                .and_then(Option::as_ref)
                .is_some_and(|bound| self.is_into_string_bound(bound)),
            _ => false,
        }
    }

    fn require_into_string(&mut self, ty: &Type, span: Span) {
        if !self.is_into_string_convertible(ty) {
            let description = self.describe(ty);
            self.err(
                span,
                format!(
                    "`{description}` cannot be converted to `String`; implement `Into<String>`"
                ),
            );
        }
    }

    fn check_mut_arg_target(&mut self, expr: &Expr) {
        if let ExprKind::Path(path) = &expr.kind {
            if let Some(res) = self.resolved.path_res.get(&path.id) {
                if res.consumed == path.segments.len() {
                    if let Resolution::Local(id) = res.base {
                        match self.locals.get(&id) {
                            Some((_, true)) => return,
                            Some((_, false)) => {
                                self.err(expr.span, "cannot pass an immutable binding as a `mut` argument — declare it with `let mut`");
                                return;
                            }
                            None => {}
                        }
                    }
                }
            }
        }
        self.err(
            expr.span,
            "a `mut` argument must be a plain mutable local variable",
        );
    }
}
