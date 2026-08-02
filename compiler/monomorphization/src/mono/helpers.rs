use super::*;

pub(super) fn placeholder_for(hir_fn: &HirFunction) -> MonoFunction {
    MonoFunction {
        id: MonoFnId(0),
        name: hir_fn.name.clone(),
        owner: hir_fn.owner,
        is_closure: false,
        self_param: hir_fn.self_param,
        self_ty: hir_fn.self_ty.clone(),
        self_local: hir_fn.self_local,
        captures: Vec::new(),
        params: Vec::new(),
        ret: Type::Error,
        body: MonoExpr {
            kind: MonoExprKind::Unit,
            ty: Type::unit(),
        },
    }
}

/// `Struct`/`TupleStruct`/`Enum`/`Array` are the only `Type` variants that
/// can own an `impl` block (language-spec §7/§8) — everything else
/// (primitives, tuples, strings, functions, interfaces-as-bounds) never
/// reaches here for a well-typed program, since `typecheck` only ever
/// produces a `CallGenericMethod`/`CallMethod` when the bound check
/// succeeded against a declared `impl`. Unlike the other three, a
/// `Type::Array(_)` carries no `DefId` of its own — `array_owner` (looked
/// up once in `hir::lower` and threaded through `HirModule`) supplies it.
pub(super) fn owner_def_id(ty: &Type, array_owner: Option<DefId>) -> Option<DefId> {
    match ty {
        Type::Struct(id, _) | Type::TupleStruct(id, _) => Some(*id),
        Type::Enum(id, _) => Some(*id),
        Type::Array(_) => array_owner,
        _ => None,
    }
}

pub(super) fn subst_type(ty: &Type, subst: &HashMap<Symbol, Type>) -> Type {
    match ty {
        Type::Generic(name) => subst.get(name).cloned().unwrap_or_else(|| ty.clone()),
        Type::Struct(id, args) => {
            Type::Struct(*id, args.iter().map(|t| subst_type(t, subst)).collect())
        }
        Type::TupleStruct(id, args) => {
            Type::TupleStruct(*id, args.iter().map(|t| subst_type(t, subst)).collect())
        }
        Type::Tuple(items) => Type::Tuple(items.iter().map(|t| subst_type(t, subst)).collect()),
        Type::Enum(id, args) => {
            Type::Enum(*id, args.iter().map(|t| subst_type(t, subst)).collect())
        }
        Type::Array(elem) => Type::Array(Box::new(subst_type(elem, subst))),
        Type::Function(params, ret) => Type::Function(
            params.iter().map(|t| subst_type(t, subst)).collect(),
            Box::new(subst_type(ret, subst)),
        ),
        Type::Weak(inner) => Type::Weak(Box::new(subst_type(inner, subst))),
        Type::Primitive(_) | Type::String | Type::Interface(_) | Type::Never | Type::Error => {
            ty.clone()
        }
    }
}

/// Walks `declared` (a generic function's param type, possibly containing
/// `Type::Generic`) alongside `concrete` (the same param's actual argument
/// type at a call site, already fully substituted) and records every
/// `Type::Generic(name) -> concrete-subtree` binding found. Mirrors
/// `subst_type`'s own recursion shape so every position a substitution
/// could be written back into is also a position this can read one out of.
pub(super) fn collect_generic_bindings(
    declared: &Type,
    concrete: &Type,
    out: &mut HashMap<Symbol, Type>,
) {
    match declared {
        Type::Generic(name) => {
            out.entry(name.clone()).or_insert_with(|| concrete.clone());
        }
        Type::Tuple(items) => {
            if let Type::Tuple(concrete_items) = concrete {
                for (d, c) in items.iter().zip(concrete_items) {
                    collect_generic_bindings(d, c, out);
                }
            }
        }
        Type::Enum(_, args) => {
            if let Type::Enum(_, concrete_args) = concrete {
                for (d, c) in args.iter().zip(concrete_args) {
                    collect_generic_bindings(d, c, out);
                }
            }
        }
        Type::Struct(_, args) => {
            if let Type::Struct(_, concrete_args) = concrete {
                for (d, c) in args.iter().zip(concrete_args) {
                    collect_generic_bindings(d, c, out);
                }
            }
        }
        Type::TupleStruct(_, args) => {
            if let Type::TupleStruct(_, concrete_args) = concrete {
                for (d, c) in args.iter().zip(concrete_args) {
                    collect_generic_bindings(d, c, out);
                }
            }
        }
        Type::Array(elem) => {
            if let Type::Array(concrete_elem) = concrete {
                collect_generic_bindings(elem, concrete_elem, out);
            }
        }
        Type::Function(params, ret) => {
            if let Type::Function(concrete_params, concrete_ret) = concrete {
                for (d, c) in params.iter().zip(concrete_params) {
                    collect_generic_bindings(d, c, out);
                }
                collect_generic_bindings(ret, concrete_ret, out);
            }
        }
        Type::Weak(inner) => {
            if let Type::Weak(concrete_inner) = concrete {
                collect_generic_bindings(inner, concrete_inner, out);
            }
        }
        Type::Primitive(_) | Type::String | Type::Interface(_) | Type::Never | Type::Error => {}
    }
}
