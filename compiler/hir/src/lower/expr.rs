use super::*;

impl Lowerer<'_> {
    pub(super) fn lower_expr(&mut self, expr: &Expr) -> HirExpr {
        let mut lowered = self.lower_expr_unpacked(expr);
        let Some(concrete) = self.existential_coercions.get(&expr.id).cloned() else {
            return lowered;
        };
        let existential_ty = self.ty_of(expr.id);
        let trait_id = match &existential_ty {
            Type::Any(id, _) | Type::Some(id, _) => *id,
            _ => return lowered,
        };
        lowered.ty = self.runtime_ty(&concrete);
        HirExpr {
            kind: HirExprKind::PackExistential {
                value: Box::new(lowered),
                concrete,
                trait_id,
            },
            ty: existential_ty,
        }
    }

    fn lower_expr_unpacked(&mut self, expr: &Expr) -> HirExpr {
        let ty = self.ty_of(expr.id);
        match &expr.kind {
            ExprKind::Await(inner) => HirExpr {
                kind: HirExprKind::Await(Box::new(self.lower_expr(inner))),
                ty,
            },
            ExprKind::Literal(lit) => HirExpr {
                kind: HirExprKind::Literal(lit.clone()),
                ty,
            },
            ExprKind::Path(path) => self.lower_value_path(path, None, &[], ty),
            ExprKind::Tuple(elems) => HirExpr {
                kind: HirExprKind::Tuple(elems.iter().map(|e| self.lower_expr(e)).collect()),
                ty,
            },
            ExprKind::Array(elems) => HirExpr {
                kind: HirExprKind::Array(elems.iter().map(|e| self.lower_expr(e)).collect()),
                ty,
            },
            ExprKind::StringTemplate(parts) => self.lower_template(parts, ty),
            ExprKind::Unary { op, expr: inner } => HirExpr {
                kind: HirExprKind::Unary {
                    op: *op,
                    expr: Box::new(self.lower_expr(inner)),
                },
                ty,
            },
            ExprKind::Binary { op, lhs, rhs } => HirExpr {
                kind: HirExprKind::Binary {
                    op: *op,
                    lhs: Box::new(self.lower_expr(lhs)),
                    rhs: Box::new(self.lower_expr(rhs)),
                },
                ty,
            },
            ExprKind::Assign { target, value } => HirExpr {
                kind: HirExprKind::Assign {
                    target: Box::new(self.lower_assign_target(target)),
                    value: Box::new(self.lower_expr(value)),
                },
                ty,
            },
            ExprKind::Call { callee, args, .. } => self.lower_call(expr.id, callee, args, ty),
            ExprKind::MutArg(inner) => self.lower_expr(inner),
            ExprKind::MethodCall {
                receiver,
                method,
                args,
                ..
            } => {
                if matches!(&receiver.kind, ExprKind::Path(path) if path.segments.len() == 1 && path.segments[0].name.as_str() == "timer")
                    && method.name.as_str() == "sleep"
                {
                    HirExpr {
                        kind: HirExprKind::CallBuiltin {
                            name: Symbol::new("__timer_sleep"),
                            args: args.iter().map(|arg| self.lower_expr(arg)).collect(),
                        },
                        ty,
                    }
                } else {
                    self.lower_method_call(expr.id, receiver, method, args, ty)
                }
            }
            ExprKind::Field { base, field } => self.lower_field(base, field, ty),
            ExprKind::Index { base, index } => HirExpr {
                kind: HirExprKind::Index {
                    base: Box::new(self.lower_expr(base)),
                    index: Box::new(self.lower_expr(index)),
                },
                ty,
            },
            ExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => HirExpr {
                kind: HirExprKind::If {
                    cond: Box::new(self.lower_expr(cond)),
                    then_branch: Box::new(self.lower_block_as_expr(then_branch)),
                    else_branch: else_branch.as_ref().map(|e| Box::new(self.lower_expr(e))),
                },
                ty,
            },
            ExprKind::Match { scrutinee, arms } => HirExpr {
                kind: HirExprKind::Match {
                    scrutinee: Box::new(self.lower_expr(scrutinee)),
                    arms: arms.iter().map(|a| self.lower_match_arm(a)).collect(),
                },
                ty,
            },
            ExprKind::Block(block) => self.lower_block_as_expr(block),
            ExprKind::While { cond, body } => HirExpr {
                kind: HirExprKind::While {
                    cond: Box::new(self.lower_expr(cond)),
                    body: Box::new(self.lower_block_as_expr(body)),
                },
                ty,
            },
            ExprKind::ForIn {
                pattern,
                iter,
                body,
            } => self.lower_for_in(pattern, iter, body),
            ExprKind::Loop { body } => HirExpr {
                kind: HirExprKind::Loop {
                    body: Box::new(self.lower_block_as_expr(body)),
                },
                ty,
            },
            ExprKind::Break(value) => HirExpr {
                kind: HirExprKind::Break(value.as_ref().map(|v| Box::new(self.lower_expr(v)))),
                ty,
            },
            ExprKind::Continue => HirExpr {
                kind: HirExprKind::Continue,
                ty,
            },
            ExprKind::Return(value) => HirExpr {
                kind: HirExprKind::Return(value.as_ref().map(|v| Box::new(self.lower_expr(v)))),
                ty,
            },
            ExprKind::Closure { params, body, .. } => self.lower_closure(params, body, ty),
            ExprKind::StructLit {
                path,
                fields,
                owned: _,
            } => self.lower_struct_lit(path, fields, ty),
        }
    }

    pub(super) fn lower_args(&mut self, args: &[Expr]) -> Vec<HirExpr> {
        args.iter()
            .map(|a| match &a.kind {
                ExprKind::MutArg(inner) => self.lower_expr(inner),
                _ => self.lower_expr(a),
            })
            .collect()
    }

    /// Like [`Self::lower_args`], but aware of `sig`'s trailing variadic
    /// parameter (`args: ...String`) if it has one: the fixed prefix
    /// lowers as usual, and every trailing argument (zero or more, each
    /// converted through [`Self::convert_to_string`] first when the
    /// element type is `String` — the same conversion template-string
    /// interpolation already applies) is collected into one
    /// [`HirExprKind::Array`], appended as the call's final actual
    /// argument. The callee itself only ever sees an ordinary
    /// `Array`-typed parameter — no new calling convention.
    pub(super) fn lower_variadic_aware_args(&mut self, sig: &FnSig, args: &[Expr]) -> Vec<HirExpr> {
        let Some(last) = sig.params.last().filter(|p| p.variadic) else {
            return self.lower_args(args);
        };
        let fixed_count = sig.params.len() - 1;
        let split = fixed_count.min(args.len());
        let mut lowered = self.lower_args(&args[..split]);
        let elem_ty = last.ty.clone();
        let variadic_items: Vec<HirExpr> = self
            .lower_args(&args[split..])
            .into_iter()
            .map(|arg| {
                if matches!(elem_ty, Type::String) {
                    self.convert_to_string(arg)
                } else {
                    arg
                }
            })
            .collect();
        lowered.push(HirExpr {
            kind: HirExprKind::Array(variadic_items),
            ty: Type::FixedArray(
                Box::new(elem_ty),
                Box::new(Type::Const((args.len() - split) as u128)),
            ),
        });
        lowered
    }
}
