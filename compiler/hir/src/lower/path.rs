use super::*;

impl Lowerer<'_> {
    /// Lowers a value path and its trailing field/method chain.
    pub(super) fn lower_value_path(
        &mut self,
        path: &Path,
        call_args: Option<&[Expr]>,
        generic_args: &[Type],
        result_ty: Type,
    ) -> HirExpr {
        let Some(res) = self.resolved.path_res.get(&path.id).cloned() else {
            return HirExpr {
                kind: HirExprKind::Unit,
                ty: result_ty,
            };
        };
        let total = path.segments.len();
        let direct_call_args = if res.consumed == total {
            call_args
        } else {
            None
        };

        let (mut current, mut current_ty) = match res.base {
            Resolution::Local(id) => {
                let ty = self.local_ty(id);
                let local = self.local_for(id);
                let expr = local_ref(local, ty.clone());
                if res.consumed == total {
                    if let Some(args) = call_args {
                        return self.lower_call_value(expr, args, result_ty);
                    }
                }
                let upgraded_ty = self.upgrade_weak(ty.clone());
                let kind = self.weak_upgrade_kind(expr, &ty);
                (
                    HirExpr {
                        kind,
                        ty: upgraded_ty.clone(),
                    },
                    upgraded_ty,
                )
            }
            Resolution::Def(id) => {
                self.lower_def_value(id, direct_call_args, generic_args, &result_ty)
            }
            Resolution::EnumVariant(enum_id, idx) => {
                self.lower_enum_variant_value(enum_id, idx, direct_call_args, &result_ty)
            }
            Resolution::StaticMember(owner_id, idx) => {
                self.lower_static_member(owner_id, idx, direct_call_args, generic_args, &result_ty)
            }
            Resolution::GenericParam | Resolution::Error => (
                HirExpr {
                    kind: HirExprKind::Unit,
                    ty: Type::Error,
                },
                Type::Error,
            ),
        };

        for i in res.consumed..total {
            let seg = &path.segments[i];
            let is_last = i + 1 == total;
            if is_last {
                if let Some(args) = call_args {
                    return self.lower_method_call_on(
                        current,
                        &current_ty,
                        seg,
                        args,
                        generic_args,
                        &result_ty,
                    );
                }
            }
            let (next, next_ty) = self.lower_field_access_named(current, &current_ty, seg);
            current = next;
            current_ty = next_ty;
        }
        current
    }

    pub(super) fn lower_def_value(
        &mut self,
        id: DefId,
        call_args: Option<&[Expr]>,
        generic_args: &[Type],
        result_ty: &Type,
    ) -> (HirExpr, Type) {
        let def = self.resolved.definitions.get(id);
        match def.kind {
            nether_resolver::DefKind::Fn => {
                let name = def.name.clone();
                if name.as_str() == "println" || name.as_str() == "print" {
                    let args = self
                        .lower_args(call_args.unwrap_or(&[]))
                        .into_iter()
                        .map(|arg| self.convert_to_string(arg))
                        .collect();
                    let expr = HirExpr {
                        kind: HirExprKind::CallBuiltin { name, args },
                        ty: result_ty.clone(),
                    };
                    return (expr, result_ty.clone());
                }
                let fn_id = self.fn_by_def.get(&id).copied();
                match (fn_id, call_args) {
                    (Some(fid), Some(args)) => {
                        let sig = self.sigs.fns.get(&id).cloned();
                        let args = match &sig {
                            Some(sig) => self.lower_variadic_aware_args(sig, args),
                            None => self.lower_args(args),
                        };
                        let expr = HirExpr {
                            kind: HirExprKind::CallStatic {
                                fn_id: fid,
                                generic_args: generic_args.to_vec(),
                                args,
                            },
                            ty: result_ty.clone(),
                        };
                        (expr, result_ty.clone())
                    }
                    (Some(fid), None) => {
                        let expr = HirExpr {
                            kind: HirExprKind::FnRef(fid),
                            ty: result_ty.clone(),
                        };
                        (expr, result_ty.clone())
                    }
                    (None, _) => (
                        HirExpr {
                            kind: HirExprKind::Unit,
                            ty: Type::Error,
                        },
                        Type::Error,
                    ),
                }
            }
            nether_resolver::DefKind::Type => {
                let fields = match call_args {
                    Some(args) => self.lower_args(args),
                    None => Vec::new(),
                };
                let expr = HirExpr {
                    kind: HirExprKind::Construct { ty: id, fields },
                    ty: result_ty.clone(),
                };
                (expr, result_ty.clone())
            }
            _ => (
                HirExpr {
                    kind: HirExprKind::Unit,
                    ty: Type::Error,
                },
                Type::Error,
            ),
        }
    }

    pub(super) fn lower_enum_variant_value(
        &mut self,
        enum_id: DefId,
        idx: u32,
        call_args: Option<&[Expr]>,
        result_ty: &Type,
    ) -> (HirExpr, Type) {
        let has_payload = self
            .sigs
            .enum_sigs
            .get(&enum_id)
            .and_then(|s| s.variants.get(idx as usize))
            .map(|(_, p)| !p.is_empty())
            .unwrap_or(false);
        let payload = if has_payload {
            self.lower_args(call_args.unwrap_or(&[]))
        } else {
            Vec::new()
        };
        let expr = HirExpr {
            kind: HirExprKind::ConstructVariant {
                enum_id,
                variant: idx,
                payload,
            },
            ty: result_ty.clone(),
        };
        (expr, result_ty.clone())
    }

    pub(super) fn lower_static_member(
        &mut self,
        owner_id: DefId,
        idx: u32,
        call_args: Option<&[Expr]>,
        generic_args: &[Type],
        result_ty: &Type,
    ) -> (HirExpr, Type) {
        let Some(name) = self
            .resolved
            .definitions
            .get(owner_id)
            .methods
            .get(idx as usize)
            .cloned()
        else {
            return (
                HirExpr {
                    kind: HirExprKind::Unit,
                    ty: Type::Error,
                },
                Type::Error,
            );
        };
        // Static-member calls are always non-specialized — specialization
        // is scoped to instance methods, `nether_typecheck` rejects a
        // static method inside a concrete-specialization `impl` block.
        let Some(fn_id) = self
            .methods
            .get(&(owner_id, name))
            .and_then(|set| set.generic)
        else {
            return (
                HirExpr {
                    kind: HirExprKind::Unit,
                    ty: Type::Error,
                },
                Type::Error,
            );
        };
        match call_args {
            Some(args) => {
                let args = self.lower_args(args);
                let expr = HirExpr {
                    kind: HirExprKind::CallStatic {
                        fn_id,
                        generic_args: generic_args.to_vec(),
                        args,
                    },
                    ty: result_ty.clone(),
                };
                (expr, result_ty.clone())
            }
            None => {
                let expr = HirExpr {
                    kind: HirExprKind::FnRef(fn_id),
                    ty: result_ty.clone(),
                };
                (expr, result_ty.clone())
            }
        }
    }

    pub(super) fn lower_method_call_on(
        &mut self,
        receiver: HirExpr,
        receiver_ty: &Type,
        method: &Ident,
        args: &[Expr],
        generic_args: &[Type],
        result_ty: &Type,
    ) -> HirExpr {
        if let Type::Array(_) = receiver_ty {
            if matches!(method.name.as_str(), "len" | "push" | "pop") {
                let lowered = self.lower_args(args);
                return HirExpr {
                    kind: HirExprKind::CallArrayMethod {
                        receiver: Box::new(receiver),
                        method: method.name.clone(),
                        args: lowered,
                    },
                    ty: result_ty.clone(),
                };
            }
        }
        if let Type::Generic(name) = receiver_ty {
            let bound = self.generics.get(name).cloned().flatten();
            let is_static = bound
                .as_ref()
                .and_then(|bound| {
                    self.sigs
                        .interface_methods
                        .get(&(bound.interface, method.name.clone()))
                })
                .is_some_and(|sig| sig.self_param.is_none());
            let lowered = self.lower_args(args);
            return match bound {
                Some(bound) => HirExpr {
                    kind: HirExprKind::CallGenericMethod {
                        receiver: Box::new(receiver),
                        bound_interface: bound.interface,
                        method_name: method.name.clone(),
                        is_static,
                        generic_args: generic_args.to_vec(),
                        args: lowered,
                    },
                    ty: result_ty.clone(),
                },
                None => HirExpr {
                    kind: HirExprKind::Unit,
                    ty: Type::Error,
                },
            };
        }
        let owner_id = match receiver_ty {
            Type::Struct(id, _) | Type::TupleStruct(id, _) => Some(*id),
            Type::Enum(id, _) => Some(*id),
            Type::Array(_) => self.array_owner,
            _ => None,
        };
        let method_set = owner_id.and_then(|id| self.methods.get(&(id, method.name.clone())));
        let lowered = self.lower_args(args);
        match method_set {
            Some(set) => {
                let is_static = owner_id
                    .and_then(|id| self.sigs.method(id, &method.name))
                    .is_some_and(|sig| sig.self_param.is_none());
                if is_static {
                    // Static methods are never specialized (rejected at
                    // `nether_typecheck::check::build_impl_methods`), so
                    // `set.generic` alone is authoritative — the same
                    // fixed-`fn_id` shape as an ordinary free-function call.
                    let Some(fid) = set.generic else {
                        return HirExpr {
                            kind: HirExprKind::Unit,
                            ty: Type::Error,
                        };
                    };
                    let call = HirExpr {
                        kind: HirExprKind::CallStatic {
                            fn_id: fid,
                            generic_args: generic_args.to_vec(),
                            args: lowered,
                        },
                        ty: result_ty.clone(),
                    };
                    HirExpr {
                        kind: HirExprKind::Block(
                            vec![HirStmt {
                                kind: HirStmtKind::Expr(receiver),
                            }],
                            Some(Box::new(call)),
                        ),
                        ty: result_ty.clone(),
                    }
                } else {
                    // Deferred to `monomorphization`: which body runs can
                    // depend on the receiver's *substituted* argument
                    // types (a concrete specialization), not knowable
                    // until the enclosing function itself is instantiated
                    // — see `HirExprKind::CallMethod`'s own docs.
                    HirExpr {
                        kind: HirExprKind::CallMethod {
                            receiver: Box::new(receiver),
                            method_name: method.name.clone(),
                            generic_args: generic_args.to_vec(),
                            args: lowered,
                        },
                        ty: result_ty.clone(),
                    }
                }
            }
            None => HirExpr {
                kind: HirExprKind::Unit,
                ty: Type::Error,
            },
        }
    }

    pub(super) fn lower_field_access_named(
        &mut self,
        base: HirExpr,
        base_ty: &Type,
        ident: &Ident,
    ) -> (HirExpr, Type) {
        let (raw, field_ty) = self.raw_field_access(base, base_ty, ident);
        if field_ty.is_error() {
            return (raw, field_ty);
        }
        let upgraded_ty = self.upgrade_weak(field_ty.clone());
        let kind = self.weak_upgrade_kind(raw, &field_ty);
        (
            HirExpr {
                kind,
                ty: upgraded_ty.clone(),
            },
            upgraded_ty,
        )
    }

    /// Reading a `weak T`-typed place *as a value* never yields a bare
    /// `weak T` — see `nether_typecheck::check::TypeChecker::upgrade_weak`
    /// (this is the same rule, one stage later: that decided the *type*
    /// every such read has; this decides the *shape* — wrapping the raw
    /// place-read in a call to the `__weak_upgrade` builtin, which
    /// `nether_mir`/`nether_codegen` lower into a real
    /// `nether_rt_arc_weak_upgrade` call producing the `Option<T>` this
    /// expression's type promises).
    pub(super) fn upgrade_weak(&self, ty: Type) -> Type {
        match ty {
            Type::Weak(inner) => match self.resolved.definitions.lookup(&Symbol::new("Option")) {
                Some(option_id) => Type::Enum(option_id, vec![*inner]),
                None => Type::Error,
            },
            other => other,
        }
    }

    /// `expr`'s own kind, wrapped in a `__weak_upgrade` builtin call if
    /// `raw_ty` (its *un-upgraded* declared type) is `weak T` — see
    /// [`Self::upgrade_weak`].
    pub(super) fn weak_upgrade_kind(&self, expr: HirExpr, raw_ty: &Type) -> HirExprKind {
        if matches!(raw_ty, Type::Weak(_)) {
            HirExprKind::CallBuiltin {
                name: Symbol::new("__weak_upgrade"),
                args: vec![expr],
            }
        } else {
            expr.kind
        }
    }
}
