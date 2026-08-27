use super::*;

impl Lowerer<'_> {
    pub(super) fn lower_call(
        &mut self,
        call_id: NodeId,
        callee: &Expr,
        args: &[Expr],
        result_ty: Type,
    ) -> HirExpr {
        if let ExprKind::Path(path) = &callee.kind {
            if path.segments.len() == 2
                && path.segments[0].name.as_str() == "task"
                && path.segments[1].name.as_str() == "spawn"
            {
                return HirExpr {
                    kind: HirExprKind::CallBuiltin {
                        name: Symbol::new("__task_spawn"),
                        args: self.lower_args(args),
                    },
                    ty: result_ty,
                };
            }
            if path.segments.len() == 2
                && path.segments[0].name.as_str() == "thread"
                && path.segments[1].name.as_str() == "spawn"
            {
                return HirExpr {
                    kind: HirExprKind::CallBuiltin {
                        name: Symbol::new("__thread_spawn"),
                        args: self.lower_args(args),
                    },
                    ty: result_ty,
                };
            }
            if path.segments.len() == 2
                && path.segments[0].name.as_str() == "timer"
                && path.segments[1].name.as_str() == "sleep"
            {
                return HirExpr {
                    kind: HirExprKind::CallBuiltin {
                        name: Symbol::new("__timer_sleep"),
                        args: self.lower_args(args),
                    },
                    ty: result_ty,
                };
            }
        }
        let generic_args = self.generic_args_for(call_id);
        if let ExprKind::Path(path) = &callee.kind {
            self.lower_value_path(path, Some(args), &generic_args, result_ty)
        } else {
            let callee_hir = self.lower_expr(callee);
            self.lower_call_value(callee_hir, args, result_ty)
        }
    }

    pub(super) fn lower_call_value(
        &mut self,
        callee: HirExpr,
        args: &[Expr],
        result_ty: Type,
    ) -> HirExpr {
        let args = self.lower_args(args);
        HirExpr {
            kind: HirExprKind::Call {
                callee: Box::new(callee),
                args,
            },
            ty: result_ty,
        }
    }

    pub(super) fn lower_method_call(
        &mut self,
        call_id: NodeId,
        receiver: &Expr,
        method: &Ident,
        args: &[Expr],
        result_ty: Type,
    ) -> HirExpr {
        let generic_args = self.generic_args_for(call_id);
        // Computed from the receiver AST node's *raw* (un-stripped) type —
        // see `receiver_domain_of`'s own docs — before lowering the
        // receiver itself erases the owned/ARC distinction.
        let receiver_domain = self.receiver_domain_of(receiver.id);
        let receiver_hir = self.lower_expr(receiver);
        let receiver_ty = receiver_hir.ty.clone();
        self.lower_method_call_on(
            receiver_hir,
            &receiver_ty,
            receiver_domain,
            method,
            args,
            &generic_args,
            &result_ty,
        )
    }

    pub(super) fn lower_field(
        &mut self,
        base: &Expr,
        field: &FieldAccessor,
        result_ty: Type,
    ) -> HirExpr {
        let base_hir = self.lower_expr(base);
        match field {
            FieldAccessor::Named(ident) => {
                let base_ty = base_hir.ty.clone();
                let (expr, _) = self.lower_field_access_named(base_hir, &base_ty, ident);
                HirExpr {
                    kind: expr.kind,
                    ty: result_ty,
                }
            }
            FieldAccessor::Index(idx, _) => HirExpr {
                kind: HirExprKind::Field {
                    base: Box::new(base_hir),
                    index: *idx,
                },
                ty: result_ty,
            },
        }
    }

    /// Lowers an assignment's target place — deliberately bypasses the
    /// `weak T` → `Option<T>` upgrade a normal read applies
    /// (`Self::weak_upgrade_kind`): the target must stay a plain,
    /// assignable place (`nether_mir::build::FnBuilder::lower_place`'s own
    /// invariant — it panics on anything else), carrying its *real*
    /// `weak T` type, not an upgrade-call expression, which produces a
    /// value, not a location. Mirrors `nether_typecheck::check::TypeChecker::
    /// check_assign_target_type`'s own doc for why this needs to exist at
    /// all.
    pub(super) fn lower_assign_target(&mut self, target: &Expr) -> HirExpr {
        let ty = self.ty_of(target.id);
        match &target.kind {
            // `self.field`/`x.field` is one multi-segment `Path` node, not
            // `ExprKind::Field` — see `nether_typecheck::check::TypeChecker::
            // check_assign_target_type`'s own matching note.
            ExprKind::Path(path) => self.lower_assign_path_target(path, ty),
            ExprKind::Field {
                base,
                field: FieldAccessor::Named(ident),
            } => {
                let base_hir = self.lower_expr(base);
                let base_ty = base_hir.ty.clone();
                let (expr, _) = self.raw_field_access(base_hir, &base_ty, ident);
                HirExpr {
                    kind: expr.kind,
                    ty,
                }
            }
            _ => self.lower_expr(target),
        }
    }

    /// [`Self::lower_assign_target`]'s handling for a `Path` target —
    /// mirrors [`Self::lower_value_path`]'s own segment-walking loop, but
    /// the *last* segment uses the raw, non-upgrading
    /// [`Self::raw_field_access`] rather than
    /// [`Self::lower_field_access_named`]; every earlier segment is an
    /// ordinary read on the way there, so it upgrades as normal.
    pub(super) fn lower_assign_path_target(&mut self, path: &Path, ty: Type) -> HirExpr {
        let Some(res) = self.resolved.path_res.get(&path.id).cloned() else {
            return HirExpr {
                kind: HirExprKind::Unit,
                ty: Type::Error,
            };
        };
        let total = path.segments.len();
        let (mut current, mut current_ty) = match res.base {
            Resolution::Local(id) => {
                let local_ty = self.local_ty(id);
                (local_ref(self.local_for(id), local_ty.clone()), local_ty)
            }
            _ => return self.lower_value_path(path, None, &[], ty),
        };
        if res.consumed == total
            && matches!(current_ty, Type::MutRef(_))
            && !matches!(ty, Type::Ref(_) | Type::MutRef(_))
        {
            return HirExpr {
                kind: HirExprKind::Deref(Box::new(current)),
                ty,
            };
        }
        for i in res.consumed..total {
            let seg = &path.segments[i];
            if i + 1 == total {
                let (expr, _) = self.raw_field_access(current, &current_ty, seg);
                return HirExpr {
                    kind: expr.kind,
                    ty,
                };
            }
            let (next, next_ty) = self.lower_field_access_named(current, &current_ty, seg);
            current = next;
            current_ty = next_ty;
        }
        current
    }

    /// The raw, un-upgraded field projection — shared by
    /// [`Self::lower_assign_target`]/[`Self::lower_assign_path_target`]
    /// (an assignment target's final segment, which must stay a plain
    /// place) and [`Self::lower_field_access_named`] (which wraps this in
    /// the `weak`-upgrade check for a normal read).
    pub(super) fn raw_field_access(
        &mut self,
        base: HirExpr,
        base_ty: &Type,
        ident: &Ident,
    ) -> (HirExpr, Type) {
        let base_ty = base_ty.strip_indirection();
        if let Type::Struct(_, _) = base_ty {
            if let Some(fields) = self.sigs.named_type_fields(base_ty) {
                if let Some(idx) = fields.iter().position(|(n, _)| n == &ident.name) {
                    let field_ty = fields[idx].1.clone();
                    let expr = HirExpr {
                        kind: HirExprKind::Field {
                            base: Box::new(base),
                            index: idx as u32,
                        },
                        ty: field_ty.clone(),
                    };
                    return (expr, field_ty);
                }
            }
        }
        (
            HirExpr {
                kind: HirExprKind::Unit,
                ty: Type::Error,
            },
            Type::Error,
        )
    }
}
