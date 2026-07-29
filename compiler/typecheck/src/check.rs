use std::collections::{HashMap, HashSet};

use nether_ast::{
    BinaryOp, Block, EnumDecl, Expr, ExprKind, FnDecl, Ident, InterfaceDecl, Item, Literal,
    Module, NodeId, Path, Pattern, Stmt, Symbol, TemplatePart, TypeDecl, TypeDeclKind, TypeExpr,
    UnaryOp,
};
use nether_diagnostics::{Diagnostic, Span};
use nether_resolver::{DefId, DefKind, Resolution, ResolvedNames};

use crate::sig::{EnumSig, FnSig, GenericBound, Signatures, TypeShape};
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
                if let Some(id) = resolved
                    .definitions
                    .lookup_in(t.span.file, &t.name.name)
                {
                    idx.type_decls.insert(id, t);
                }
            }
            Item::Enum(e) => {
                if let Some(id) = resolved
                    .definitions
                    .lookup_in(e.span.file, &e.name.name)
                {
                    idx.enum_decls.insert(id, e);
                }
            }
            Item::Interface(i) => {
                if let Some(id) = resolved
                    .definitions
                    .lookup_in(i.span.file, &i.name.name)
                {
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
fn lower_type_expr(ty: &TypeExpr, resolved: &ResolvedNames, decls: &DeclIndex, diags: &mut Vec<Diagnostic>) -> Type {
    match ty {
        TypeExpr::Named { path, generics, .. } => lower_named_type(path, generics, resolved, decls, diags),
        TypeExpr::Tuple(elems, _) => {
            Type::Tuple(elems.iter().map(|e| lower_type_expr(e, resolved, decls, diags)).collect())
        }
        TypeExpr::Array(inner, _) => Type::Array(Box::new(lower_type_expr(inner, resolved, decls, diags))),
        TypeExpr::Weak(inner, span) => {
            let inner_ty = lower_type_expr(inner, resolved, decls, diags);
            if !inner_ty.is_error() && crate::alloc::alloc_kind(&inner_ty, &resolved.definitions) != crate::alloc::AllocKind::Heap
            {
                diags.push(
                    Diagnostic::error("`weak` can only wrap a heap-allocated type")
                        .with_label(*span, "this type is not heap-allocated"),
                );
            }
            Type::Weak(Box::new(inner_ty))
        }
        TypeExpr::Function { params, ret, .. } => Type::Function(
            params.iter().map(|p| lower_type_expr(p, resolved, decls, diags)).collect(),
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
                    Type::Array(Box::new(lower_type_expr(&generics[0], resolved, decls, diags)))
                } else {
                    diags.push(Diagnostic::error("`Array` takes exactly one type argument").with_label(path.span, "here"));
                    Type::Error
                };
            }
            if let Some(prim) = PrimitiveKind::from_name(name) {
                return Type::Primitive(prim);
            }
            match def.kind {
                DefKind::Enum => {
                    let expected = decls
                        .enum_decls
                        .get(&id)
                        .map(|decl| decl.generics.len())
                        .unwrap_or_else(|| match name {
                            "Option" => 1,
                            "Result" => 2,
                            _ => 0,
                        });
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
                    let args = generics.iter().map(|g| lower_type_expr(g, resolved, decls, diags)).collect();
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
                        .with_label(
                            path.span,
                            "use it as a generic bound instead",
                        ),
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
    let TypeExpr::Named { path, generics, span } = ty else {
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

fn build_fn_sig(f: &FnDecl, resolved: &ResolvedNames, decls: &DeclIndex, diags: &mut Vec<Diagnostic>) -> FnSig {
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
            ty: lower_type_expr(&p.ty, resolved, decls, diags),
        })
        .collect();
    let ret = f.ret.as_ref().map(|r| lower_type_expr(r, resolved, decls, diags)).unwrap_or_else(Type::unit);
    FnSig { self_param: f.self_param, params, ret, generics }
}

fn build_type_shapes(decls: &DeclIndex, resolved: &ResolvedNames, sigs: &mut Signatures, diags: &mut Vec<Diagnostic>) {
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
                        .and_then(|bound| {
                            lower_generic_bound(
                                bound, resolved, decls, diags,
                            )
                        })
                })
                .collect(),
        );
        let shape = match &t.kind {
            TypeDeclKind::Struct(fields) => TypeShape::Struct(
                fields.iter().map(|f| (f.name.name.clone(), lower_type_expr(&f.ty, resolved, decls, diags))).collect(),
            ),
            TypeDeclKind::TupleStruct(tys) => {
                TypeShape::TupleStruct(tys.iter().map(|ty| lower_type_expr(ty, resolved, decls, diags)).collect())
            }
            TypeDeclKind::Unit => TypeShape::Unit,
        };
        sigs.type_shapes.insert(id, shape);
    }
}

fn build_enum_sigs(module: &Module, resolved: &ResolvedNames, decls: &DeclIndex, sigs: &mut Signatures, diags: &mut Vec<Diagnostic>) {
    // Option/Result are ordinary built-in generic enums.  They have no AST
    // declarations, so their structural signatures must be seeded here
    // explicitly just like their names/variants are seeded by resolver.
    if let Some(id) = resolved.definitions.lookup(&Symbol::new("Option")) {
        let t = Symbol::new("T");
        sigs.enum_sigs.insert(
            id,
            EnumSig {
                generics: vec![t.clone()],
                variants: vec![
                    (Symbol::new("Some"), vec![Type::Generic(t)]),
                    (Symbol::new("None"), Vec::new()),
                ],
            },
        );
        sigs.generic_type_bounds.insert(id, vec![None]);
    }
    if let Some(id) = resolved.definitions.lookup(&Symbol::new("Result")) {
        let t = Symbol::new("T");
        let e = Symbol::new("E");
        sigs.enum_sigs.insert(
            id,
            EnumSig {
                generics: vec![t.clone(), e.clone()],
                variants: vec![
                    (Symbol::new("Ok"), vec![Type::Generic(t)]),
                    (Symbol::new("Error"), vec![Type::Generic(e)]),
                ],
            },
        );
        sigs.generic_type_bounds.insert(id, vec![None, None]);
    }
    for item in &module.items {
        let Item::Enum(e) = item else { continue };
        let Some(id) = resolved
            .definitions
            .lookup_in(e.span.file, &e.name.name)
        else { continue };
        let generics = e.generics.iter().map(|g| g.name.name.clone()).collect();
        sigs.generic_type_bounds.insert(
            id,
            e.generics
                .iter()
                .map(|generic| {
                    generic
                        .bound
                        .as_ref()
                        .and_then(|bound| {
                            lower_generic_bound(
                                bound, resolved, decls, diags,
                            )
                        })
                })
                .collect(),
        );
        let variants = e
            .variants
            .iter()
            .map(|v| (v.name.name.clone(), v.payload.iter().map(|t| lower_type_expr(t, resolved, decls, diags)).collect()))
            .collect();
        sigs.enum_sigs.insert(id, EnumSig { generics, variants });
    }
}

fn build_fn_sigs(module: &Module, resolved: &ResolvedNames, decls: &DeclIndex, sigs: &mut Signatures, diags: &mut Vec<Diagnostic>) {
    for item in &module.items {
        if let Item::Fn(f) = item {
            if let Some(id) = resolved
                .definitions
                .lookup_in(f.span.file, &f.name.name)
            {
                let sig = build_fn_sig(f, resolved, decls, diags);
                sigs.fns.insert(id, sig);
            }
        }
    }
}

type InterfaceMethodTable = HashMap<DefId, HashMap<Symbol, (FnSig, bool)>>;

fn specialize_fn_sig(sig: &FnSig, subst: &HashMap<Symbol, Type>) -> FnSig {
    FnSig {
        self_param: sig.self_param,
        params: sig
            .params
            .iter()
            .map(|param| crate::sig::ParamSig {
                name: param.name.clone(),
                mutable: param.mutable,
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
                    .and_then(|bound| {
                        lower_generic_bound(bound, resolved, decls, diags)
                    }),
            )
        })
        .collect()
}

fn build_interface_method_table(decls: &DeclIndex, resolved: &ResolvedNames, diags: &mut Vec<Diagnostic>) -> InterfaceMethodTable {
    let mut table = HashMap::new();
    for (&id, iface) in &decls.interface_decls {
        let methods = iface
            .methods
            .iter()
            .map(|m| (m.name.name.clone(), (build_fn_sig(m, resolved, decls, diags), m.body.is_some())))
            .collect();
        table.insert(id, methods);
    }
    table
}

/// Merges each `impl` block's own methods into [`Signatures::methods`],
/// records `(type, interface)` pairs in [`Signatures::impls`], and — for
/// every interface an `impl` declares — inherits any default method the
/// impl didn't override, reporting a diagnostic for any *required*
/// (no-default) method the impl is missing (language-spec §7).
fn build_impl_methods(
    module: &Module,
    resolved: &ResolvedNames,
    decls: &DeclIndex,
    interface_methods: &InterfaceMethodTable,
    sigs: &mut Signatures,
    diags: &mut Vec<Diagnostic>,
) {
    for item in &module.items {
        let Item::Impl(b) = item else { continue };
        let Some(owner) = resolved
            .definitions
            .lookup_in(b.span.file, &b.target.name)
        else { continue };
        let owner_ty = owner_as_type(owner, resolved, decls);
        let owner_generics =
            owner_generic_params(owner, decls, resolved, diags);

        for m in &b.methods {
            let mut sig = build_fn_sig(m, resolved, decls, diags);
            sig.generics.splice(0..0, owner_generics.clone());
            let key = (owner, m.name.name.clone());
            if sigs.methods.contains_key(&key) {
                diags.push(
                    Diagnostic::error(format!(
                        "method `{}` is defined more than once for `{}`",
                        m.name.name, b.target.name
                    ))
                    .with_label(m.name.span, "redefined here"),
                );
            } else {
                sigs.methods.insert(key, sig);
            }
        }

        let Some(interface_ty) = &b.interface else { continue };
        let Some(bound) = lower_generic_bound(interface_ty, resolved, decls, diags) else { continue };
        let iface_id = bound.interface;
        if sigs
            .impls
            .iter()
            .any(|(impl_owner, existing)| {
                nominal_id(impl_owner) == Some(owner)
                    && existing.interface == bound.interface
            })
        {
            diags.push(
                Diagnostic::error(format!(
                    "`{}` already implements `{}`",
                    b.target.name,
                    resolved.definitions.get(iface_id).name
                ))
                .with_label(interface_ty.span(), "duplicate implementation"),
            );
            continue;
        }
        sigs.impls.insert((owner_ty, bound.clone()));

        let Some(iface_methods) = interface_methods.get(&iface_id) else { continue };
        let interface_generics = decls
            .interface_decls
            .get(&iface_id)
            .map(|decl| decl.generics.as_slice())
            .unwrap_or(&[]);
        let interface_subst: HashMap<Symbol, Type> = interface_generics
            .iter()
            .map(|generic| generic.name.name.clone())
            .zip(bound.args.iter().cloned())
            .collect();
        for (name, (raw_sig, has_default)) in iface_methods {
            let mut sig = specialize_fn_sig(raw_sig, &interface_subst);
            sig.generics.splice(0..0, owner_generics.clone());
            if sigs.methods.contains_key(&(owner, name.clone())) {
                if let Some(actual) = sigs.methods.get(&(owner, name.clone())) {
                    if !method_signatures_match(actual, &sig) {
                        diags.push(
                            Diagnostic::error(format!(
                                "method `{name}` does not match its declaration in interface `{}`",
                                resolved.definitions.get(iface_id).name
                            ))
                            .with_label(b.span, "implementation is here"),
                        );
                    }
                }
                continue;
            }
            if *has_default {
                sigs.methods.insert((owner, name.clone()), sig);
                sigs.default_method_substitutions
                    .insert((owner, name.clone()), interface_subst.clone());
            } else {
                diags.push(
                    Diagnostic::error(format!(
                        "`{}` does not implement required method `{}` of interface `{}`",
                        b.target.name,
                        name,
                        resolved.definitions.get(iface_id).name
                    ))
                    .with_label(b.target.span, "here"),
                );
            }
        }
    }
}

fn owner_as_type(id: DefId, resolved: &ResolvedNames, decls: &DeclIndex) -> Type {
    let args = decls
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
    match resolved.definitions.get(id).kind {
        DefKind::Enum => Type::Enum(id, args),
        _ => match decls.type_decls.get(&id).map(|t| &t.kind) {
            Some(TypeDeclKind::TupleStruct(_)) => Type::TupleStruct(id, args),
            _ => Type::Struct(id, args),
        },
    }
}

fn nominal_id(ty: &Type) -> Option<DefId> {
    match ty {
        Type::Struct(id, _)
        | Type::TupleStruct(id, _)
        | Type::Enum(id, _) => Some(*id),
        _ => None,
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
        if crate::alloc::alloc_kind(&ty, &resolved.definitions)
            == crate::alloc::AllocKind::Heap
        {
            continue;
        }
        if value_layout_reaches_cycle(
            &ty,
            resolved,
            sigs,
            &mut Vec::new(),
        ) {
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
        if value_layout_reaches_cycle(
            &ty,
            resolved,
            sigs,
            &mut Vec::new(),
        ) {
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
            if crate::alloc::alloc_kind(ty, &resolved.definitions)
                == crate::alloc::AllocKind::Heap
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
                .any(|field| {
                    value_layout_reaches_cycle(
                        field, resolved, sigs, stack,
                    )
                });
            stack.pop();
            recursive
        }
        Type::Enum(id, _) => {
            if stack.contains(id) {
                return true;
            }
            stack.push(*id);
            let recursive = sigs
                .enum_sigs
                .get(id)
                .is_some_and(|sig| {
                    (0..sig.variants.len()).any(|variant| {
                        sigs.enum_payload(ty, variant as u32)
                            .unwrap_or_default()
                            .iter()
                            .any(|field| {
                                value_layout_reaches_cycle(
                                    field, resolved, sigs, stack,
                                )
                            })
                    })
                });
            stack.pop();
            recursive
        }
        Type::Tuple(items) => items.iter().any(|item| {
            value_layout_reaches_cycle(item, resolved, sigs, stack)
        }),
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
    let interface_methods = build_interface_method_table(&decls, resolved, &mut diagnostics);
    build_impl_methods(module, resolved, &decls, &interface_methods, &mut sigs, &mut diagnostics);
    validate_finite_value_layouts(
        resolved,
        &decls,
        &sigs,
        &mut diagnostics,
    );

    let mut expr_types = HashMap::new();
    let mut local_types = HashMap::new();
    for item in &module.items {
        match item {
            Item::Fn(f) => {
                if let Some(id) = resolved
                    .definitions
                    .lookup_in(f.span.file, &f.name.name)
                {
                    if let Some(sig) = sigs.fns.get(&id).cloned() {
                        if f.name.name.as_str() == "main"
                            && (!sig.params.is_empty()
                                || !sig.generics.is_empty()
                                || sig.ret != Type::unit())
                        {
                            diagnostics.push(
                                Diagnostic::error(
                                    "`main` must have signature `fn main()`",
                                )
                                .with_label(f.span, "invalid entry point"),
                            );
                            continue;
                        }
                        let mut checker = Checker::new(resolved, &sigs, &decls, &mut expr_types, &mut local_types, &mut diagnostics);
                        checker.check_fn_decl(f, &sig, None);
                    }
                }
            }
            Item::Impl(b) => {
                if let Some(owner) = resolved
                    .definitions
                    .lookup_in(b.span.file, &b.target.name)
                {
                    let self_ty = owner_as_type(owner, resolved, &decls);
                    for m in &b.methods {
                        if let Some(sig) = sigs.method(owner, &m.name.name).cloned() {
                            let mut checker = Checker::new(resolved, &sigs, &decls, &mut expr_types, &mut local_types, &mut diagnostics);
                            checker.check_fn_decl(m, &sig, Some(self_ty.clone()));
                        }
                    }
                }
            }
            Item::Interface(i) => {
                if let Some(id) = resolved
                    .definitions
                    .lookup_in(i.span.file, &i.name.name)
                {
                    for m in &i.methods {
                        if m.body.is_none() {
                            continue;
                        }
                        if let Some((sig, _)) = interface_methods.get(&id).and_then(|ms| ms.get(&m.name.name)).cloned() {
                            let mut checking_sig = sig;
                            let interface_generics = i.generics.iter().map(|generic| {
                                (
                                    generic.name.name.clone(),
                                    generic
                                        .bound
                                        .as_ref()
                                        .and_then(|bound| {
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
                            let mut checker = Checker::new(resolved, &sigs, &decls, &mut expr_types, &mut local_types, &mut diagnostics);
                            checker.check_fn_decl(m, &checking_sig, Some(Type::Interface(id)));
                        }
                    }
                }
            }
            Item::Type(_) | Item::Enum(_) | Item::Use(_) => {}
        }
    }

    (TypedTables { expr_types, local_types, signatures: sigs }, diagnostics)
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
                let args_str: Vec<String> = args.iter().map(|a| describe_type(a, resolved)).collect();
                format!("{name}<{}>", args_str.join(", "))
            }
        }
        Type::Tuple(elems) => {
            format!("({})", elems.iter().map(|e| describe_type(e, resolved)).collect::<Vec<_>>().join(", "))
        }
        Type::Array(inner) => format!("[{}]", describe_type(inner, resolved)),
        Type::String => "String".to_string(),
        Type::Function(params, ret) => format!(
            "({}) => {}",
            params.iter().map(|p| describe_type(p, resolved)).collect::<Vec<_>>().join(", "),
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
        Type::Struct(id, args) => {
            Type::Struct(*id, args.iter().map(|arg| substitute_generic(arg, subst)).collect())
        }
        Type::TupleStruct(id, args) => Type::TupleStruct(
            *id,
            args.iter().map(|arg| substitute_generic(arg, subst)).collect(),
        ),
        Type::Array(inner) => Type::Array(Box::new(substitute_generic(inner, subst))),
        Type::Weak(inner) => Type::Weak(Box::new(substitute_generic(inner, subst))),
        Type::Tuple(elems) => Type::Tuple(elems.iter().map(|e| substitute_generic(e, subst)).collect()),
        Type::Enum(id, args) => Type::Enum(*id, args.iter().map(|a| substitute_generic(a, subst)).collect()),
        Type::Function(params, ret) => {
            Type::Function(params.iter().map(|p| substitute_generic(p, subst)).collect(), Box::new(substitute_generic(ret, subst)))
        }
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
        (Type::Error, expected) if !expected.contains_error() && !expected.contains_generic() => expected.clone(),
        (Type::Tuple(actual), Type::Tuple(expected)) if actual.len() == expected.len() => Type::Tuple(
            actual
                .iter()
                .zip(expected)
                .map(|(actual, expected)| contextualize_unknowns(actual, expected))
                .collect(),
        ),
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
        (
            Type::TupleStruct(actual_id, actual),
            Type::TupleStruct(expected_id, expected),
        ) if actual_id == expected_id && actual.len() == expected.len() => Type::TupleStruct(
            *actual_id,
            actual
                .iter()
                .zip(expected)
                .map(|(actual, expected)| contextualize_unknowns(actual, expected))
                .collect(),
        ),
        (Type::Array(actual), Type::Array(expected)) => {
            Type::Array(Box::new(contextualize_unknowns(actual, expected)))
        }
        (Type::Weak(actual), Type::Weak(expected)) => {
            Type::Weak(Box::new(contextualize_unknowns(actual, expected)))
        }
        (Type::Function(actual_params, actual_ret), Type::Function(expected_params, expected_ret))
            if actual_params.len() == expected_params.len() =>
        {
            Type::Function(
                actual_params
                    .iter()
                    .zip(expected_params)
                    .map(|(actual, expected)| contextualize_unknowns(actual, expected))
                    .collect(),
                Box::new(contextualize_unknowns(actual_ret, expected_ret)),
            )
        }
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
        diagnostics: &'a mut Vec<Diagnostic>,
    ) -> Self {
        Checker {
            resolved,
            sigs,
            decls,
            expr_types,
            local_types,
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
        self.diagnostics.push(Diagnostic::error(message).with_label(span, "here"));
    }

    fn describe(&self, ty: &Type) -> String {
        describe_type(ty, self.resolved)
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
            self.bind_local(param_ast.id, param_sig.ty.clone(), param_ast.mutable);
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
                self.err(body.span, format!("expected return type `{expected}`, found `{found}`"));
            }
        }
    }

    fn check_block(&mut self, block: &Block) -> Type {
        self.check_block_with_expected(block, None)
    }

    fn check_block_with_expected(&mut self, block: &Block, expected: Option<&Type>) -> Type {
        for stmt in &block.stmts {
            self.check_stmt(stmt);
        }
        match &block.tail {
            Some(tail) => self.check_expr_with_expected(tail, expected),
            None => Type::unit(),
        }
    }

    fn check_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let(let_stmt) => {
                let declared_ty = let_stmt.ty.as_ref().map(|t| lower_type_expr(t, self.resolved, self.decls, self.diagnostics));
                let has_declared_type = declared_ty.is_some();
                let value_ty = self.check_expr_with_expected(&let_stmt.value, declared_ty.as_ref());
                let final_ty = match declared_ty {
                    Some(declared) => {
                        if !value_ty.compatible(&declared) {
                            let expected = self.describe(&declared);
                            let found = self.describe(&value_ty);
                            self.err(let_stmt.value.span, format!("expected `{expected}`, found `{found}`"));
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
            }
            Stmt::Expr(expr) => {
                let ty = self.check_expr(expr);
                if !ty.is_error() && ty.contains_error() {
                    self.err(expr.span, "cannot infer all generic type arguments for this expression");
                }
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
            ExprKind::Path(path) => self.check_value_path(path, None, expected),
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
            ExprKind::Call { callee, args } => self.check_call(callee, args, expected),
            ExprKind::MutArg(inner) => self.check_expr(inner),
            ExprKind::MethodCall { receiver, method, args } => {
                let receiver_ty = self.check_expr(receiver);
                self.check_method_call_on(&receiver_ty, method, args, method.span)
            }
            ExprKind::Field { base, field } => {
                let base_ty = self.check_expr(base);
                self.check_field_access(&base_ty, field)
            }
            ExprKind::Index { base, index } => self.check_index(base, index),
            ExprKind::If { cond, then_branch, else_branch } => {
                self.check_if(cond, then_branch, else_branch, expected)
            }
            ExprKind::Match { scrutinee, arms } => self.check_match(scrutinee, arms, expr.span, expected),
            ExprKind::Block(block) => self.check_block_with_expected(block, expected),
            ExprKind::While { cond, body } => {
                let cond_ty = self.check_expr(cond);
                self.require_bool(&cond_ty, cond.span, "`while` condition");
                self.loop_depth += 1;
                self.check_block(body);
                self.loop_depth -= 1;
                Type::unit()
            }
            ExprKind::ForIn { pattern, iter, body } => self.check_for_in(pattern, iter, body),
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
                    self.err(
                        expr.span,
                        "`continue` is only valid inside a loop",
                    );
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
                    self.err(span, "cannot infer the element type of an empty array literal without context");
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
                self.err(e.span, format!("expected `{expected_s}`, found `{found_s}` in array literal"));
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
                    self.err(rhs.span, "both operands of an arithmetic operator must have the same type");
                    return Type::Error;
                }
                lhs_ty
            }
            BinaryOp::Eq | BinaryOp::Ne => {
                if !lhs_ty.compatible(&rhs_ty) {
                    self.err(rhs.span, "both operands of `==`/`!=` must have the same type");
                }
                Type::Primitive(PrimitiveKind::Bool)
            }
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                if !matches!(&lhs_ty, Type::Primitive(p) if p.is_numeric()) && !lhs_ty.is_error() {
                    self.err(lhs.span, "comparison operators require numeric operands");
                } else if !lhs_ty.compatible(&rhs_ty) {
                    self.err(rhs.span, "both operands of a comparison must have the same type");
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
            self.err(value.span, format!("expected `{expected}`, found `{found}`"));
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
                .is_some_and(|resolution| {
                    matches!(resolution.base, Resolution::Local(_))
                }),
            ExprKind::Field { base, .. }
            | ExprKind::Index { base, .. } => {
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
            ExprKind::Field { base, field: nether_ast::FieldAccessor::Named(ident) } => {
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
            Resolution::Local(id) => self.locals.get(&id).map(|(t, _)| t.clone()).unwrap_or(Type::Error),
            // No other resolution kind can ever name a `weak`-typed place
            // (only locals/fields can be declared `weak T`), so falling
            // back to the normal, upgrading path is safe here.
            _ => return self.check_value_path(path, None, None),
        };
        for i in res.consumed..total {
            let seg = &path.segments[i];
            current_ty =
                if i + 1 == total { self.field_type_named(&current_ty, seg) } else { self.check_field_access_named(&current_ty, seg) };
        }
        current_ty
    }

    fn check_assign_target_mutable(&mut self, target: &Expr) {
        match &target.kind {
            ExprKind::Path(path) => {
                if let Some(res) = self.resolved.path_res.get(&path.id) {
                    if res.consumed == path.segments.len() {
                        if let Resolution::Local(id) = res.base {
                            if let Some((_, false)) = self.locals.get(&id) {
                                self.err(target.span, "cannot assign to an immutable binding — declare it with `let mut`");
                            }
                        }
                    }
                }
            }
            ExprKind::Field { base, .. } | ExprKind::Index { base, .. } => self.check_assign_target_mutable(base),
            _ => {}
        }
    }

    fn check_call(&mut self, callee: &Expr, args: &[Expr], expected: Option<&Type>) -> Type {
        if let ExprKind::Path(path) = &callee.kind {
            let ty = self.check_value_path(path, Some(args), expected);
            self.expr_types.insert(callee.id, ty.clone());
            ty
        } else {
            let callee_ty = self.check_expr(callee);
            self.check_call_value(&callee_ty, args, callee.span)
        }
    }

    fn check_call_value(&mut self, ty: &Type, args: &[Expr], span: Span) -> Type {
        match ty {
            Type::Function(params, ret) => {
                if args.len() != params.len() {
                    self.err(span, format!("expected {} argument(s), found {}", params.len(), args.len()));
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
    ) -> Type {
        let Some(res) = self.resolved.path_res.get(&path.id).cloned() else {
            return Type::Error;
        };
        let total = path.segments.len();
        let direct_call_args = if res.consumed == total { call_args } else { None };

        let mut current_ty = match res.base {
            Resolution::Local(id) => {
                let ty = self.locals.get(&id).map(|(t, _)| t.clone()).unwrap_or(Type::Error);
                if res.consumed == total {
                    if let Some(args) = call_args {
                        return self.check_call_value(&ty, args, path.span);
                    }
                }
                self.upgrade_weak(ty)
            }
            Resolution::Def(id) => {
                self.resolve_def_value(id, path.span, direct_call_args, expected)
            }
            Resolution::EnumVariant(enum_id, idx) => {
                self.enum_variant_value_type(enum_id, idx, path.span, direct_call_args, expected)
            }
            Resolution::StaticMember(owner_id, idx) => {
                self.static_member_call_or_value(owner_id, idx, path.span, direct_call_args)
            }
            Resolution::GenericParam | Resolution::Error => Type::Error,
        };

        for i in res.consumed..total {
            let seg = &path.segments[i];
            let is_last = i + 1 == total;
            if is_last {
                if let Some(args) = call_args {
                    return self.check_method_call_on(&current_ty, seg, args, seg.span);
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
    ) -> Type {
        let def = self.resolved.definitions.get(id);
        let name = def.name.clone();
        match def.kind {
            DefKind::Primitive => {
                self.err(span, format!("`{name}` is a type, not a value"));
                Type::Error
            }
            DefKind::Interface => {
                self.err(span, format!("`{name}` is an interface and has no value form"));
                Type::Error
            }
            DefKind::Imported => Type::Error,
            DefKind::Enum => {
                self.err(span, format!("`{name}` is an enum type, not a value — use one of its variants"));
                Type::Error
            }
            DefKind::Fn => self.resolve_fn_value(id, &name, span, call_args, expected),
            DefKind::Type => self.resolve_type_value(id, &name, span, call_args, expected),
        }
    }

    fn resolve_fn_value(
        &mut self,
        id: DefId,
        name: &Symbol,
        span: Span,
        call_args: Option<&[Expr]>,
        expected: Option<&Type>,
    ) -> Type {
        if name.as_str() == "println" || name.as_str() == "print" {
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
                    let subst =
                        self.check_call_args(&sig, args, span, expected, None);
                    substitute_generic(&sig.ret, &subst)
                }
                None if sig.generics.is_empty() => {
                    Type::Function(sig.params.iter().map(|p| p.ty.clone()).collect(), Box::new(sig.ret))
                }
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
            Some(Type::Struct(expected_id, args))
            | Some(Type::TupleStruct(expected_id, args))
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
                    self.err(span, format!("expected {} argument(s), found {}", field_tys.len(), args.len()));
                }
                for (a, declared) in args.iter().zip(field_tys.iter()) {
                    let concrete_expected = substitute_generic(declared, &subst);
                    let actual = self.check_expr_with_expected(a, Some(&concrete_expected));
                    collect_generic_bindings(declared, &actual, &mut subst);
                    let concrete_expected = substitute_generic(declared, &subst);
                    if !actual.compatible(&concrete_expected) {
                        let expected_s = self.describe(&concrete_expected);
                        let found_s = self.describe(&actual);
                        self.err(a.span, format!("expected `{expected_s}`, found `{found_s}`"));
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
            self.err(span, "internal type information for this enum is unavailable");
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
                        self.err(span, format!("variant expects {} argument(s), found {}", payload.len(), args.len()));
                    }
                    for (a, declared) in args.iter().zip(payload.iter()) {
                        let concrete_expected = substitute_generic(declared, &subst);
                        let actual = self.check_expr_with_expected(a, Some(&concrete_expected));
                        collect_generic_bindings(declared, &actual, &mut subst);
                        let concrete_expected = substitute_generic(declared, &subst);
                        if !actual.compatible(&concrete_expected) {
                            let expected_s = self.describe(&concrete_expected);
                            let found_s = self.describe(&actual);
                            self.err(a.span, format!("expected `{expected_s}`, found `{found_s}`"));
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

    fn static_member_call_or_value(&mut self, owner_id: DefId, idx: u32, span: Span, call_args: Option<&[Expr]>) -> Type {
        let Some(name) = self.resolved.definitions.get(owner_id).methods.get(idx as usize).cloned() else {
            return Type::Error;
        };
        let Some(sig) = self.sigs.method(owner_id, &name).cloned() else { return Type::Error };
        if sig.self_param.is_some() {
            self.err(span, format!("`{name}` is an instance method and cannot be called as a static member"));
            return Type::Error;
        }
        match call_args {
            Some(args) => {
                let subst =
                    self.check_call_args(&sig, args, span, None, None);
                substitute_generic(&sig.ret, &subst)
            }
            None => Type::Function(sig.params.iter().map(|p| p.ty.clone()).collect(), Box::new(sig.ret)),
        }
    }

    fn check_method_call_on(&mut self, base_ty: &Type, method: &Ident, args: &[Expr], span: Span) -> Type {
        if let Type::Generic(name) = base_ty {
            return self.check_generic_method_call(name, method, args, span);
        }
        if let Type::Array(elem_ty) = base_ty {
            return self.check_array_method_call(elem_ty, method, args, span);
        }
        let owner_id = match base_ty {
            Type::Struct(id, _) | Type::TupleStruct(id, _) => Some(*id),
            Type::Enum(id, _) => Some(*id),
            _ => None,
        };
        let Some(owner_id) = owner_id else {
            if !base_ty.is_error() {
                let desc = self.describe(base_ty);
                self.err(method.span, format!("`{desc}` has no method named `{}`", method.name));
            }
            return Type::Error;
        };
        let Some(sig) = self.sigs.method(owner_id, &method.name).cloned() else {
            let desc = self.describe(base_ty);
            self.err(method.span, format!("`{desc}` has no method named `{}`", method.name));
            return Type::Error;
        };
        if sig.self_param.is_none() {
            self.err(method.span, format!("`{}` is a static method; call it as `Type.{}(...)`", method.name, method.name));
            return Type::Error;
        }
        let owner_pattern = owner_as_type(owner_id, self.resolved, self.decls);
        let mut owner_subst = HashMap::new();
        collect_generic_bindings(&owner_pattern, base_ty, &mut owner_subst);
        let subst =
            self.check_call_args(&sig, args, span, None, Some(owner_subst));
        substitute_generic(&sig.ret, &subst)
    }

    fn check_generic_method_call(&mut self, name: &Symbol, method: &Ident, args: &[Expr], span: Span) -> Type {
        let bound = self.generics.get(name).cloned().flatten();
        if let Some(bound) = bound {
            let bound_name = self.resolved.definitions.get(bound.interface).name.as_str();
            if bound_name == "Into"
                && bound.args == [Type::String]
                && method.name.as_str() == "into_string"
            {
                if !args.is_empty() {
                    self.err(span, format!("expected 0 argument(s), found {}", args.len()));
                }
                return Type::String;
            }
            if let Some(iface) = self.decls.interface_decls.get(&bound.interface) {
                if let Some(m) = iface.methods.iter().find(|m| m.name.name == method.name) {
                    let raw_sig = build_fn_sig(m, self.resolved, self.decls, self.diagnostics);
                    let interface_subst: HashMap<Symbol, Type> = iface
                        .generics
                        .iter()
                        .map(|generic| generic.name.name.clone())
                        .zip(bound.args)
                        .collect();
                    let sig = specialize_fn_sig(&raw_sig, &interface_subst);
                    let subst =
                        self.check_call_args(&sig, args, span, None, None);
                    return substitute_generic(&sig.ret, &subst);
                }
            }
        }
        self.err(method.span, format!("no method named `{}` found for generic type `{name}`", method.name));
        Type::Error
    }

    /// `Array<T>`'s runtime methods (language-spec §3.5: "Runtime methods:
    /// push, pop, len") — provided by `runtime/array`, not by any `impl`
    /// block a user could write, so there is no [`crate::sig::FnSig`] for
    /// them in [`Signatures::methods`] to look up; special-cased here the
    /// same way `println`/`print` are special-cased in
    /// [`Checker::resolve_fn_value`].
    fn check_array_method_call(&mut self, elem_ty: &Type, method: &Ident, args: &[Expr], span: Span) -> Type {
        match method.name.as_str() {
            "len" => {
                if !args.is_empty() {
                    self.err(span, format!("expected 0 argument(s), found {}", args.len()));
                }
                Type::Primitive(PrimitiveKind::Usize)
            }
            "push" => {
                if args.len() != 1 {
                    self.err(span, format!("expected 1 argument(s), found {}", args.len()));
                }
                if let Some(arg) = args.first() {
                    let actual = self.check_expr_with_expected(arg, Some(elem_ty));
                    if !actual.compatible(elem_ty) {
                        let expected_s = self.describe(elem_ty);
                        let found_s = self.describe(&actual);
                        self.err(arg.span, format!("expected `{expected_s}`, found `{found_s}`"));
                    }
                }
                Type::unit()
            }
            "pop" => {
                if !args.is_empty() {
                    self.err(span, format!("expected 0 argument(s), found {}", args.len()));
                }
                match self.resolved.definitions.lookup(&Symbol::new("Option")) {
                    Some(option_id) => Type::Enum(option_id, vec![elem_ty.clone()]),
                    None => Type::Error,
                }
            }
            other => {
                self.err(method.span, format!("`Array` has no method named `{other}`"));
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
            nether_ast::FieldAccessor::Named(ident) => self.check_field_access_named(base_ty, ident),
            nether_ast::FieldAccessor::Index(idx, span) => self.check_tuple_index(base_ty, *idx, *span),
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
        self.err(ident.span, format!("`{desc}` has no field named `{}`", ident.name));
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
            self.check_pattern(&arm.pattern, &scrutinee_ty, &mut covered, &mut has_catch_all);
            let arm_expected = expected.or(result_ty.as_ref());
            let body_ty = self.check_expr_with_expected(&arm.body, arm_expected);
            result_ty = Some(match result_ty {
                None => body_ty,
                Some(prev) => {
                    if !prev.compatible(&body_ty) {
                        let prev_s = self.describe(&prev);
                        let body_s = self.describe(&body_ty);
                        self.err(arm.body.span, format!("`match` arms have incompatible types: `{prev_s}` vs `{body_s}`"));
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
                        self.err(span, format!("match is not exhaustive: missing variant(s) {}", missing.join(", ")));
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

    fn check_pattern(&mut self, pattern: &Pattern, scrutinee_ty: &Type, covered: &mut HashSet<u32>, has_catch_all: &mut bool) {
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
                    self.err(*span, format!("literal pattern is incompatible with `{found}`"));
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
                        self.err(*span, format!("tuple pattern requires a tuple, found `{found}`"));
                        vec![Type::Error; elems.len()]
                    }
                };
                for (index, pattern) in elems.iter().enumerate() {
                    let ty = elem_tys.get(index).cloned().unwrap_or(Type::Error);
                    self.check_pattern_inner(
                        pattern,
                        &ty,
                        covered,
                        has_catch_all,
                        false,
                    );
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
        let same_enum = matches!(scrutinee_ty, Type::Enum(scrutinee_id, _) if *scrutinee_id == enum_id);
        if !same_enum {
            if !scrutinee_ty.is_error() {
                let pattern_enum = self.resolved.definitions.get(enum_id).name.to_string();
                let found = self.describe(scrutinee_ty);
                self.err(
                    span,
                    format!(
                        "variant pattern from enum `{pattern_enum}` cannot match `{found}`"
                    ),
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
            self.check_pattern_inner(
                pattern,
                &ty,
                covered,
                has_catch_all,
                false,
            );
        }
    }

    fn check_for_in(&mut self, pattern: &Pattern, iter: &Expr, body: &Block) -> Type {
        let iter_ty = self.check_expr(iter);
        let elem_ty = match &iter_ty {
            Type::Array(inner) => (**inner).clone(),
            Type::Error => Type::Error,
            other => {
                let desc = self.describe(other);
                self.err(iter.span, format!("`for`-`in` requires an `Array`, found `{desc}`"));
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
            self.err(span, format!("expected return type `{expected_s}`, found `{found_s}`"));
        }
        Type::Never
    }

    fn check_closure(&mut self, params: &[nether_ast::Param], body: &Expr, expected: Option<&Type>) -> Type {
        let param_tys: Vec<Type> = params.iter().map(|p| lower_type_expr(&p.ty, self.resolved, self.decls, self.diagnostics)).collect();
        for (p, t) in params.iter().zip(param_tys.iter()) {
            self.bind_local(p.id, t.clone(), p.mutable);
        }
        let expected_ret = match expected {
            Some(Type::Function(expected_params, expected_ret))
                if expected_params.len() == param_tys.len()
                    && expected_params.iter().zip(&param_tys).all(|(expected, actual)| actual.compatible(expected)) =>
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
            self.err(path.span, format!("`{name}` is not a struct with named fields"));
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
                    self.err(value.span, format!("expected `{expected_s}`, found `{found_s}` for field `{}`", fname.name));
                }
            } else {
                self.err(fname.span, format!("`{name}` has no field named `{}`", fname.name));
                self.check_expr(value);
            }
        }
        for (field_name, _) in &decl_fields {
            if !seen.contains(field_name) {
                self.err(path.span, format!("missing field `{field_name}` in struct literal for `{name}`"));
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
    ) -> HashMap<Symbol, Type> {
        if args.len() != sig.params.len() {
            self.err(call_span, format!("expected {} argument(s), found {}", sig.params.len(), args.len()));
        }
        let mut subst = initial_subst.unwrap_or_default();
        if let Some(expected_return) = expected_return {
            collect_generic_bindings(&sig.ret, expected_return, &mut subst);
        }
        for (param, arg) in sig.params.iter().zip(args.iter()) {
            let (inner_expr, is_mut_arg) = match &arg.kind {
                ExprKind::MutArg(inner) => (inner.as_ref(), true),
                _ => (arg, false),
            };
            if param.mutable != is_mut_arg {
                if param.mutable {
                    self.err(arg.span, format!("parameter `{}` is `mut`; the caller must also write `mut` at the call site", param.name));
                } else {
                    self.err(arg.span, "`mut` is only valid for arguments passed to a `mut` parameter");
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
                self.err(inner_expr.span, format!("expected `{expected_s}`, found `{found_s}`"));
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
            let satisfies =
                self.type_satisfies_bound(concrete, &concrete_bound);
            if !satisfies {
                let concrete_s = self.describe(concrete);
                let iface_name = self.describe_bound(&concrete_bound);
                self.err(call_span, format!("`{concrete_s}` does not implement `{iface_name}`, required by generic parameter `{name}`"));
            }
        }
        subst
    }

    fn validate_type_bounds(&mut self, ty: &Type, span: Span) {
        match ty {
            Type::Struct(id, args)
            | Type::TupleStruct(id, args)
            | Type::Enum(id, args) => {
                let names = self
                    .sigs
                    .type_generics
                    .get(id)
                    .cloned()
                    .or_else(|| {
                        self.sigs
                            .enum_sigs
                            .get(id)
                            .map(|sig| sig.generics.clone())
                    })
                    .unwrap_or_default();
                let bounds = self
                    .sigs
                    .generic_type_bounds
                    .get(id)
                    .cloned()
                    .unwrap_or_default();
                let subst: HashMap<Symbol, Type> = names
                    .into_iter()
                    .zip(args.iter().cloned())
                    .collect();
                for (index, bound) in bounds.into_iter().enumerate() {
                    let Some(bound) = bound else { continue };
                    let Some(actual) = args.get(index) else { continue };
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
        let name = self.resolved.definitions.get(bound.interface).name.to_string();
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
                .is_some_and(|actual| actual == bound),
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
            Type::Generic(name) => {
                self.generics
                    .get(name)
                    .and_then(Option::as_ref)
                    .is_some_and(|bound| self.is_into_string_bound(bound))
            }
            _ => false,
        }
    }

    fn require_into_string(&mut self, ty: &Type, span: Span) {
        if !self.is_into_string_convertible(ty) {
            let description = self.describe(ty);
            self.err(span, format!("`{description}` cannot be converted to `String`; implement `Into<String>`"));
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
        self.err(expr.span, "a `mut` argument must be a plain mutable local variable");
    }
}
