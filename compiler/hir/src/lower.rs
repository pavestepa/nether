use std::collections::{HashMap, HashSet};

use nether_ast::{
    BinaryOp, Block, Expr, ExprKind, FieldAccessor, FnDecl, Ident, Item, Literal, MatchArm, Module,
    NodeId, Param, Path, Pattern, Stmt, Symbol, TemplatePart, TraitDecl,
};
use nether_resolver::{DefId, LocalId as ResolverLocalId, Resolution, ResolvedNames};
use nether_typecheck::{
    CaptureMode, FnSig, GenericBound, PrimitiveKind, ReceiverDomain, Signatures, Type, TypeShape,
    TypedTables,
};

use crate::node::{
    HirCapture, HirExpr, HirExprKind, HirFnId, HirFunction, HirLocalId, HirMatchArm, HirModule,
    HirParam, HirPattern, HirStmt, HirStmtKind, MethodFnSet,
};

mod call;
mod captures;
mod closure;
mod construct;
mod expr;
mod lowerer;
mod path;

use captures::closure_captures;
use lowerer::Lowerer;

/// Lowers a fully resolved, fully type-checked [`Module`] into a
/// [`HirModule`]. See this crate's module docs for the desugaring rules
/// applied and the documented simplifications.
///
/// Consumes `tables` (rather than borrowing it) because
/// [`nether_typecheck::Signatures`] moves straight into the resulting
/// [`HirModule`] unchanged — it is already exactly the structural,
/// desugared type information `monomorphization`/`mir` need, so this
/// crate carries it forward rather than re-deriving or re-wrapping it.
pub fn lower(module: &Module, resolved: &ResolvedNames, tables: TypedTables) -> HirModule {
    let TypedTables {
        expr_types,
        local_types,
        call_generic_args,
        existential_coercions,
        signatures,
    } = tables;

    // `resolver::LocalId` -> `Type`, built once by cross-referencing
    // `resolved.locals` (binding site -> LocalId) against `local_types`
    // (binding site -> Type) — see `nether_typecheck::TypedTables::local_types`
    // docs for why this indirection exists.
    let local_types_by_id: HashMap<ResolverLocalId, Type> = resolved
        .locals
        .iter()
        .filter_map(|(node_id, local_id)| {
            local_types.get(node_id).map(|ty| (*local_id, ty.clone()))
        })
        .collect();

    // `None` for a module compiled without the bundled prelude (e.g. an
    // isolated codegen/HIR test fixture) — such a module can still use
    // array *literals* (`Type::Array` is produced directly by literal
    // syntax, independent of any declaration), it just can't declare or
    // call a non-builtin `impl<T> Array<T>` method, which is the only
    // thing that ever actually dereferences this.
    let array_owner = resolved.definitions.lookup(&Symbol::new("Array"));

    let trait_decls = index_traits(module, resolved);
    let pending = collect_pending_fns(module, resolved, &signatures, &trait_decls);

    let mut fn_by_name = HashMap::new();
    let mut fn_by_def = HashMap::new();
    let mut methods: HashMap<(DefId, Symbol, ReceiverDomain), MethodFnSet> = HashMap::new();
    for (i, p) in pending.iter().enumerate() {
        let id = HirFnId(i as u32);
        match p.owner {
            None => {
                fn_by_name.insert(p.name.clone(), id);
                if let Some(def_id) = p.def_id {
                    fn_by_def.insert(def_id, id);
                }
            }
            Some(owner) => {
                let domain = ReceiverDomain::of_self_param(p.sig.self_param.as_ref());
                let set = methods.entry((owner, p.name.clone(), domain)).or_default();
                match &p.specialization {
                    None => set.generic = Some(id),
                    Some(args) => set.specializations.push((args.clone(), id)),
                }
            }
        }
    }

    let mut fns = Vec::with_capacity(pending.len());
    for (i, p) in pending.iter().enumerate() {
        let id = HirFnId(i as u32);
        let mut lowerer = Lowerer {
            resolved,
            expr_types: &expr_types,
            local_types_by_id: &local_types_by_id,
            call_generic_args: &call_generic_args,
            existential_coercions: &existential_coercions,
            sigs: &signatures,
            fn_by_def: &fn_by_def,
            methods: &methods,
            locals_map: HashMap::new(),
            next_local: 0,
            mutable_locals: HashSet::new(),
            generics: p.sig.generics.iter().cloned().collect(),
            type_subst: p.type_subst.clone(),
            self_override: None,
            array_owner,
        };
        fns.push(lowerer.lower_fn(p, id));
    }

    HirModule {
        signatures,
        fns,
        fn_by_name,
        fn_by_def,
        methods,
        array_owner,
    }
}

fn index_traits<'a>(module: &'a Module, resolved: &ResolvedNames) -> HashMap<DefId, &'a TraitDecl> {
    let mut map = HashMap::new();
    for item in &module.items {
        if let Item::Trait(i) = item {
            if let Some(id) = resolved.definitions.lookup_in(i.span.file, &i.name.name) {
                map.insert(id, i);
            }
        }
    }
    map
}

fn owner_type(owner: DefId, sigs: &Signatures, array_owner: Option<DefId>) -> Type {
    // `Array<T>` keeps its own distinct `Type` variant (codegen/ARC layout
    // depend on it) even though it's an ordinary declared generic type now
    // — mirrors `nether_typecheck::check::owner_as_type`'s same special case.
    if array_owner == Some(owner) {
        let elem_name = sigs
            .type_generics
            .get(&owner)
            .and_then(|generics| generics.first())
            .cloned()
            .unwrap_or_else(|| Symbol::new("T"));
        return Type::Array(Box::new(Type::Generic(elem_name)));
    }
    if let Some(sig) = sigs.enum_sigs.get(&owner) {
        Type::Enum(
            owner,
            sig.generics.iter().cloned().map(Type::Generic).collect(),
        )
    } else if matches!(
        sigs.type_shapes.get(&owner),
        Some(TypeShape::TupleStruct(_))
    ) {
        Type::TupleStruct(
            owner,
            sigs.type_generics
                .get(&owner)
                .into_iter()
                .flatten()
                .cloned()
                .map(Type::Generic)
                .collect(),
        )
    } else {
        Type::Struct(
            owner,
            sigs.type_generics
                .get(&owner)
                .into_iter()
                .flatten()
                .cloned()
                .map(Type::Generic)
                .collect(),
        )
    }
}

/// Like [`owner_type`], but for a concrete specialization
/// (`impl Option<i32> { ... }`) — `args` (already-resolved concrete
/// types, from `Signatures::impl_specializations`) replace the ordinary
/// generic-placeholder arguments `owner_type` would otherwise build.
fn concrete_owner_type(
    owner: DefId,
    sigs: &Signatures,
    array_owner: Option<DefId>,
    args: &[Type],
) -> Type {
    match owner_type(owner, sigs, array_owner) {
        Type::Enum(id, _) => Type::Enum(id, args.to_vec()),
        Type::TupleStruct(id, _) => Type::TupleStruct(id, args.to_vec()),
        Type::Struct(id, _) => Type::Struct(id, args.to_vec()),
        Type::Array(inner) => Type::Array(Box::new(args.first().cloned().unwrap_or(*inner))),
        other => other,
    }
}

fn subst_type(ty: &Type, subst: &HashMap<Symbol, Type>) -> Type {
    match ty {
        Type::Generic(name) => subst.get(name).cloned().unwrap_or_else(|| ty.clone()),
        Type::Struct(id, args) => {
            Type::Struct(*id, args.iter().map(|arg| subst_type(arg, subst)).collect())
        }
        Type::TupleStruct(id, args) => {
            Type::TupleStruct(*id, args.iter().map(|arg| subst_type(arg, subst)).collect())
        }
        Type::Tuple(items) => {
            Type::Tuple(items.iter().map(|item| subst_type(item, subst)).collect())
        }
        Type::Enum(id, args) => {
            Type::Enum(*id, args.iter().map(|arg| subst_type(arg, subst)).collect())
        }
        Type::Array(inner) => Type::Array(Box::new(subst_type(inner, subst))),
        Type::Function(params, ret) => Type::Function(
            params
                .iter()
                .map(|param| subst_type(param, subst))
                .collect(),
            Box::new(subst_type(ret, subst)),
        ),
        Type::Weak(inner) => Type::Weak(Box::new(subst_type(inner, subst))),
        _ => ty.clone(),
    }
}

/// One function/method still to be lowered, with its already-built
/// [`FnSig`] and a reference to the AST node supplying its body — which,
/// for an inherited trait default (not overridden by an `impl`), is
/// the trait's own [`FnDecl`], not anything in the `impl` block.
struct PendingFn<'a> {
    name: Symbol,
    def_id: Option<DefId>,
    owner: Option<DefId>,
    decl: &'a FnDecl,
    sig: FnSig,
    type_subst: HashMap<Symbol, Type>,
    /// `Some(args)` when this body came from a concrete specialization
    /// (`impl Option<i32> { ... }`) — mirrors
    /// `Signatures::impl_specializations`, read once per `impl` block
    /// rather than re-derived, so this crate never re-lowers a
    /// `TypeExpr` itself. Always `None` for a standalone `fn` or an
    /// inherited trait default (specialization is scoped to plain
    /// instance methods — see `nether_typecheck::sig::MethodSet`).
    specialization: Option<Vec<Type>>,
}

fn collect_pending_fns<'a>(
    module: &'a Module,
    resolved: &ResolvedNames,
    sigs: &Signatures,
    trait_decls: &HashMap<DefId, &'a TraitDecl>,
) -> Vec<PendingFn<'a>> {
    let mut pending = Vec::new();
    for item in &module.items {
        match item {
            Item::Fn(f) => {
                if let Some(id) = resolved.definitions.lookup_in(f.span.file, &f.name.name) {
                    if let Some(sig) = sigs.fns.get(&id).cloned() {
                        pending.push(PendingFn {
                            name: f.name.name.clone(),
                            def_id: Some(id),
                            owner: None,
                            decl: f,
                            sig,
                            type_subst: HashMap::new(),
                            specialization: None,
                        });
                    }
                }
            }
            Item::Extern(block) => {
                for f in &block.functions {
                    if let Some(id) = resolved.definitions.lookup_in(f.span.file, &f.name.name) {
                        if let Some(sig) = sigs.fns.get(&id).cloned() {
                            pending.push(PendingFn {
                                name: f.name.name.clone(),
                                def_id: Some(id),
                                owner: None,
                                decl: f,
                                sig,
                                type_subst: HashMap::new(),
                                specialization: None,
                            });
                        }
                    }
                }
            }
            Item::Impl(b) => {
                let Some(owner) = resolved.definitions.lookup_in(b.span.file, &b.target.name)
                else {
                    continue;
                };
                let specialization = sigs.impl_specializations.get(&b.id);
                for m in &b.methods {
                    let domain = ReceiverDomain::of_self_param(m.self_param.as_ref());
                    let Some(set) = sigs.methods.get(&(owner, m.name.name.clone(), domain)) else {
                        continue;
                    };
                    let sig = match specialization {
                        None => set.generic.clone(),
                        Some(args) => set
                            .specializations
                            .iter()
                            .find(|(existing, _)| existing == args)
                            .map(|(_, sig)| sig.clone()),
                    };
                    let Some(sig) = sig else { continue };
                    pending.push(PendingFn {
                        name: m.name.name.clone(),
                        def_id: None,
                        owner: Some(owner),
                        decl: m,
                        sig,
                        type_subst: HashMap::new(),
                        specialization: specialization.cloned(),
                    });
                }
            }
            _ => {}
        }
    }

    let mut defaults: Vec<_> = sigs.default_method_sources.iter().collect();
    defaults.sort_by_key(|((owner, name, domain), _)| {
        (
            resolved.definitions.get(*owner).name.clone(),
            name.clone(),
            *domain as u8,
        )
    });
    for ((owner, name, domain), source) in defaults {
        let Some(decl) = trait_decls
            .get(source)
            .and_then(|trait_decl| trait_decl.methods.iter().find(|m| m.name.name == *name))
        else {
            continue;
        };
        let Some(sig) = sigs.method(*owner, name, *domain).cloned() else {
            continue;
        };
        pending.push(PendingFn {
            name: name.clone(),
            def_id: None,
            owner: Some(*owner),
            decl,
            sig,
            type_subst: sigs
                .default_method_substitutions
                .get(&(*owner, name.clone(), *domain))
                .cloned()
                .unwrap_or_default(),
            specialization: None,
        });
    }
    pending
}

fn local_ref(local: HirLocalId, ty: Type) -> HirExpr {
    HirExpr {
        kind: HirExprKind::Local(local),
        ty,
    }
}
