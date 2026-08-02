use super::*;

impl Lowerer<'_> {
    /// Desugars `for pattern in iter { body }` into an index-based loop.
    pub(super) fn lower_for_in(&mut self, pattern: &Pattern, iter: &Expr, body: &Block) -> HirExpr {
        let iter_hir = self.lower_expr(iter);
        let iter_ty = iter_hir.ty.clone();
        let elem_ty = match &iter_ty {
            Type::Array(inner) => (**inner).clone(),
            _ => Type::Error,
        };

        let iter_local = self.fresh_local();
        let idx_local = self.fresh_local();
        let elem_local = self.fresh_local();

        let iter_let = HirStmt {
            kind: HirStmtKind::Let {
                local: iter_local,
                ty: iter_ty.clone(),
                value: iter_hir,
            },
        };
        let idx_init = HirExpr {
            kind: HirExprKind::Literal(Literal::Int(0)),
            ty: Type::Primitive(PrimitiveKind::Usize),
        };
        let idx_let = HirStmt {
            kind: HirStmtKind::Let {
                local: idx_local,
                ty: Type::Primitive(PrimitiveKind::Usize),
                value: idx_init,
            },
        };

        let len_call = HirExpr {
            kind: HirExprKind::CallArrayMethod {
                receiver: Box::new(local_ref(iter_local, iter_ty.clone())),
                method: Symbol::new("len"),
                args: Vec::new(),
            },
            ty: Type::Primitive(PrimitiveKind::Usize),
        };
        let cond = HirExpr {
            kind: HirExprKind::Binary {
                op: BinaryOp::Lt,
                lhs: Box::new(local_ref(idx_local, Type::Primitive(PrimitiveKind::Usize))),
                rhs: Box::new(len_call),
            },
            ty: Type::Primitive(PrimitiveKind::Bool),
        };

        let index_expr = HirExpr {
            kind: HirExprKind::Index {
                base: Box::new(local_ref(iter_local, iter_ty.clone())),
                index: Box::new(local_ref(idx_local, Type::Primitive(PrimitiveKind::Usize))),
            },
            ty: elem_ty.clone(),
        };
        let elem_let = HirStmt {
            kind: HirStmtKind::Let {
                local: elem_local,
                ty: elem_ty.clone(),
                value: index_expr,
            },
        };

        let hir_pattern = self.lower_pattern(pattern);
        let (body_stmts, body_tail) = self.lower_block(body);
        let mut arm_stmts = body_stmts;
        if let Some(tail) = body_tail {
            arm_stmts.push(HirStmt {
                kind: HirStmtKind::Expr(*tail),
            });
        }
        let arm_body = HirExpr {
            kind: HirExprKind::Block(arm_stmts, None),
            ty: Type::unit(),
        };
        let match_expr = HirExpr {
            kind: HirExprKind::Match {
                scrutinee: Box::new(local_ref(elem_local, elem_ty)),
                arms: vec![HirMatchArm {
                    pattern: hir_pattern,
                    body: arm_body,
                }],
            },
            ty: Type::unit(),
        };

        let incremented = HirExpr {
            kind: HirExprKind::Binary {
                op: BinaryOp::Add,
                lhs: Box::new(local_ref(idx_local, Type::Primitive(PrimitiveKind::Usize))),
                rhs: Box::new(HirExpr {
                    kind: HirExprKind::Literal(Literal::Int(1)),
                    ty: Type::Primitive(PrimitiveKind::Usize),
                }),
            },
            ty: Type::Primitive(PrimitiveKind::Usize),
        };
        let increment_stmt = HirStmt {
            kind: HirStmtKind::Expr(HirExpr {
                kind: HirExprKind::Assign {
                    target: Box::new(local_ref(idx_local, Type::Primitive(PrimitiveKind::Usize))),
                    value: Box::new(incremented),
                },
                ty: Type::unit(),
            }),
        };

        let while_body = HirExpr {
            kind: HirExprKind::Block(
                vec![
                    elem_let,
                    HirStmt {
                        kind: HirStmtKind::Expr(match_expr),
                    },
                    increment_stmt,
                ],
                None,
            ),
            ty: Type::unit(),
        };
        let while_expr = HirExpr {
            kind: HirExprKind::While {
                cond: Box::new(cond),
                body: Box::new(while_body),
            },
            ty: Type::unit(),
        };

        HirExpr {
            kind: HirExprKind::Block(
                vec![
                    iter_let,
                    idx_let,
                    HirStmt {
                        kind: HirStmtKind::Expr(while_expr),
                    },
                ],
                None,
            ),
            ty: Type::unit(),
        }
    }

    pub(super) fn lower_closure(
        &mut self,
        params: &[Param],
        body: &Expr,
        result_ty: Type,
    ) -> HirExpr {
        let mut hir_params = Vec::new();
        for p in params {
            let (local, ty) = match self.resolved.locals.get(&p.id) {
                Some(orig) => (self.local_for(*orig), self.local_ty(*orig)),
                None => (self.fresh_local(), Type::Error),
            };
            hir_params.push(HirParam {
                local,
                name: p.name.name.clone(),
                mutable: p.mutable,
                ty,
            });
        }
        let body_hir = self.lower_expr(body);
        let captures = closure_captures(&body_hir, &hir_params);
        HirExpr {
            kind: HirExprKind::Closure {
                params: hir_params,
                captures,
                body: Box::new(body_hir),
            },
            ty: result_ty,
        }
    }
}
