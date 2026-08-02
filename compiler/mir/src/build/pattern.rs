use super::*;

impl FnBuilder<'_> {
    pub(super) fn lower_match(
        &mut self,
        expr: &MonoExpr,
        scrutinee: &MonoExpr,
        arms: &[MonoMatchArm],
    ) -> Operand {
        let scrutinee_ty = scrutinee.ty.clone();
        // Goes through `prepare_new_binding`, not the cheaper
        // `lower_expr_to_local`, for exactly the same reason a `let`
        // does: a bare-parameter/bare-local scrutinee needs its own
        // fresh credit before every arm binding (including a top-level
        // `HirPattern::Binding` catch-all) can safely reuse it directly.
        let scrutinee_op = self.prepare_new_binding(scrutinee);
        let scrutinee_local = self.as_local(scrutinee_op, scrutinee_ty.clone());
        let match_scope_depth = self.scopes.len();
        let mut match_scope = Vec::new();
        if self.sigs.has_managed_content(&scrutinee_ty, self.defs) {
            match_scope.push(scrutinee_local);
        }
        self.scopes.push(match_scope);
        let result = self.declare_local(expr.ty.clone(), false);
        let merge_block = self.new_block();

        for arm in arms {
            let body_block = self.new_block();
            let next_block = self.new_block();
            self.lower_pattern_branch(
                Operand::Local(scrutinee_local),
                &scrutinee_ty,
                &arm.pattern,
                body_block,
                next_block,
            );

            self.current = body_block;
            // Each arm's own pattern bindings form their own scope,
            // released once the arm's body has produced its value —
            // exactly the same shape as a `let` inside a `Block` (a
            // heap-kind binding here, e.g. `Boxed(d) => ...`'s `d`, would
            // otherwise never be released at all: nothing else tracks
            // it, and `insert_arc` already gave it a bind-time retain via
            // the `VariantField`/`Field` rvalue
            // `lower_pattern_bindings` read it from).
            self.scopes.push(Vec::new());
            let mut bindings = Vec::new();
            self.lower_pattern_bindings(
                Operand::Local(scrutinee_local),
                &scrutinee_ty,
                &arm.pattern,
                &mut bindings,
            );
            for (id, op, ty) in bindings {
                let local = self.as_local(op, ty.clone());
                self.bind_hir_local(id, local);
                if self.sigs.has_managed_content(&ty, self.defs) {
                    self.scopes.last_mut().unwrap().push(local);
                }
            }
            // `arm.body` may be a bare expression (`Circle(x) => x`, no
            // braces — language-spec match arms don't require a block),
            // unlike `if`/`while`/`loop` bodies, which are always
            // `Block`s and so always self-credit via their own
            // `lower_block`/`escape`; `prepare_new_binding` is what
            // catches a bare `Circle(_) => self`-style arm here.
            let body_op = self.prepare_new_binding(&arm.body);
            let body_escaping = escaping_local(&body_op);
            self.push_instr(Instr::Assign(Place::local(result), Rvalue::Use(body_op)));
            self.release_scopes(match_scope_depth, body_escaping);
            self.scopes.pop();
            self.terminate_current_to(merge_block);

            self.current = next_block;
        }
        // Every arm chain is exhaustive (`typecheck`'s own invariant) —
        // this final fallthrough block is provably never reached.
        self.terminate_current(Terminator::Unreachable);

        self.scopes.pop();
        self.current = merge_block;
        Operand::Local(result)
    }

    /// Emits the control-flow test for one pattern. Variant payloads are
    /// deliberately visited only from the block reached after their tag
    /// has matched: unused enum payload slots are not initialized, and
    /// eagerly reading a managed payload from (for example) `Option.None`
    /// would make ARC retain an arbitrary pointer.
    pub(super) fn lower_pattern_branch(
        &mut self,
        scrutinee: Operand,
        scrutinee_ty: &Type,
        pattern: &HirPattern,
        success: BlockId,
        failure: BlockId,
    ) {
        let bool_ty = Type::Primitive(PrimitiveKind::Bool);
        match pattern {
            HirPattern::Wildcard | HirPattern::Binding(_) => {
                self.terminate_current_to(success);
            }
            HirPattern::Literal(lit) => {
                let cond = Operand::Local(self.materialize(
                    Rvalue::Binary(
                        nether_ast::BinaryOp::Eq,
                        scrutinee,
                        Operand::Literal(lit.clone(), scrutinee_ty.clone()),
                    ),
                    bool_ty,
                ));
                self.terminate_current(Terminator::Branch {
                    cond,
                    then_block: success,
                    else_block: failure,
                });
            }
            HirPattern::Tuple(subs) => {
                let elem_tys = match scrutinee_ty {
                    Type::Tuple(tys) => tys.clone(),
                    _ => vec![Type::Error; subs.len()],
                };
                for (i, sub) in subs.iter().enumerate() {
                    let elem_ty = elem_tys.get(i).cloned().unwrap_or(Type::Error);
                    let next = if i + 1 == subs.len() {
                        success
                    } else {
                        self.new_block()
                    };
                    if matches!(sub, HirPattern::Wildcard | HirPattern::Binding(_)) {
                        self.terminate_current_to(next);
                    } else {
                        let field_val = Operand::Local(self.materialize(
                            Rvalue::Field {
                                base: scrutinee.clone(),
                                index: i as u32,
                            },
                            elem_ty.clone(),
                        ));
                        self.lower_pattern_branch(field_val, &elem_ty, sub, next, failure);
                    }
                    if next != success {
                        self.current = next;
                    }
                }
                if subs.is_empty() {
                    self.terminate_current_to(success);
                }
            }
            HirPattern::Variant {
                enum_id: _,
                variant,
                payload,
            } => {
                let usize_ty = Type::Primitive(PrimitiveKind::Usize);
                let tag = Operand::Local(
                    self.materialize(Rvalue::Discriminant(scrutinee.clone()), usize_ty.clone()),
                );
                let tag_matches = Operand::Local(self.materialize(
                    Rvalue::Binary(
                        nether_ast::BinaryOp::Eq,
                        tag,
                        Operand::Literal(nether_ast::Literal::Int(u128::from(*variant)), usize_ty),
                    ),
                    bool_ty.clone(),
                ));
                let payload_block = if payload.is_empty() {
                    success
                } else {
                    self.new_block()
                };
                self.terminate_current(Terminator::Branch {
                    cond: tag_matches,
                    then_block: payload_block,
                    else_block: failure,
                });
                if payload.is_empty() {
                    return;
                }
                self.current = payload_block;
                let payload_tys = self
                    .sigs
                    .enum_payload(scrutinee_ty, *variant)
                    .unwrap_or_default();
                for (i, sub) in payload.iter().enumerate() {
                    let elem_ty = payload_tys.get(i).cloned().unwrap_or(Type::Error);
                    let next = if i + 1 == payload.len() {
                        success
                    } else {
                        self.new_block()
                    };
                    if matches!(sub, HirPattern::Wildcard | HirPattern::Binding(_)) {
                        self.terminate_current_to(next);
                    } else {
                        let field_val = Operand::Local(self.materialize(
                            Rvalue::VariantField {
                                base: scrutinee.clone(),
                                variant: *variant,
                                index: i as u32,
                            },
                            elem_ty.clone(),
                        ));
                        self.lower_pattern_branch(field_val, &elem_ty, sub, next, failure);
                    }
                    if next != success {
                        self.current = next;
                    }
                }
            }
        }
    }

    /// Collects bindings after [`Self::lower_pattern_branch`] has proved
    /// the complete pattern. Every variant payload read here is therefore
    /// active and safe to retain.
    pub(super) fn lower_pattern_bindings(
        &mut self,
        scrutinee: Operand,
        scrutinee_ty: &Type,
        pattern: &HirPattern,
        bindings: &mut Vec<(HirLocalId, Operand, Type)>,
    ) {
        match pattern {
            HirPattern::Wildcard | HirPattern::Literal(_) => {}
            HirPattern::Binding(id) => {
                bindings.push((*id, scrutinee, scrutinee_ty.clone()));
            }
            HirPattern::Tuple(subs) => {
                let elem_tys = match scrutinee_ty {
                    Type::Tuple(tys) => tys.clone(),
                    _ => vec![Type::Error; subs.len()],
                };
                for (i, sub) in subs.iter().enumerate() {
                    if !pattern_has_bindings(sub) {
                        continue;
                    }
                    let elem_ty = elem_tys.get(i).cloned().unwrap_or(Type::Error);
                    let field = Operand::Local(self.materialize(
                        Rvalue::Field {
                            base: scrutinee.clone(),
                            index: i as u32,
                        },
                        elem_ty.clone(),
                    ));
                    self.lower_pattern_bindings(field, &elem_ty, sub, bindings);
                }
            }
            HirPattern::Variant {
                variant, payload, ..
            } => {
                let payload_tys = self
                    .sigs
                    .enum_payload(scrutinee_ty, *variant)
                    .unwrap_or_default();
                for (i, sub) in payload.iter().enumerate() {
                    if !pattern_has_bindings(sub) {
                        continue;
                    }
                    let elem_ty = payload_tys.get(i).cloned().unwrap_or(Type::Error);
                    let field = Operand::Local(self.materialize(
                        Rvalue::VariantField {
                            base: scrutinee.clone(),
                            variant: *variant,
                            index: i as u32,
                        },
                        elem_ty.clone(),
                    ));
                    self.lower_pattern_bindings(field, &elem_ty, sub, bindings);
                }
            }
        }
    }
}
