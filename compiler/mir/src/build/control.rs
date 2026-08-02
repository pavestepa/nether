use super::*;

impl FnBuilder<'_> {
    pub(super) fn lower_if(
        &mut self,
        expr: &MonoExpr,
        cond: &MonoExpr,
        then_branch: &MonoExpr,
        else_branch: Option<&MonoExpr>,
    ) -> Operand {
        let cond_op = self.lower_expr(cond);
        let then_block = self.new_block();
        let else_block = self.new_block();
        let merge_block = self.new_block();
        self.terminate_current(Terminator::Branch {
            cond: cond_op,
            then_block,
            else_block,
        });

        let result = self.declare_local(expr.ty.clone(), false);

        self.current = then_block;
        let then_op = self.prepare_new_binding(then_branch);
        self.push_instr(Instr::Assign(Place::local(result), Rvalue::Use(then_op)));
        self.terminate_current_to(merge_block);

        self.current = else_block;
        let else_op = match else_branch {
            Some(e) => self.prepare_new_binding(e),
            None => Operand::Unit,
        };
        self.push_instr(Instr::Assign(Place::local(result), Rvalue::Use(else_op)));
        self.terminate_current_to(merge_block);

        self.current = merge_block;
        Operand::Local(result)
    }

    /// Like [`Self::terminate_current`], but joins into an
    /// *already-created* block (an `if`/`match` merge point) instead of a
    /// fresh dead one.
    pub(super) fn terminate_current_to(&mut self, target: BlockId) {
        let cur = self.current;
        self.blocks[cur.0 as usize].terminator = Some(Terminator::Goto(target));
        self.current = self.new_block();
        // This fresh block is unreachable (the real successor is
        // `target`) — left for `build`'s `Unreachable` default, same as
        // `terminate_current`.
    }

    pub(super) fn lower_block(
        &mut self,
        stmts: &[nether_monomorphization::MonoStmt],
        tail: Option<&MonoExpr>,
    ) -> Operand {
        self.scopes.push(Vec::new());
        for stmt in stmts {
            match &stmt.kind {
                MonoStmtKind::Let { local, ty, value } => {
                    let value_ty = ty.clone();
                    let val_op = if matches!(ty, Type::Weak(_)) {
                        self.prepare_weak_binding(value)
                    } else {
                        self.prepare_new_binding(value)
                    };
                    let mir_local = self.as_local(val_op, value_ty.clone());
                    self.bind_hir_local(*local, mir_local);
                    if self.sigs.has_managed_content(&value_ty, self.defs) {
                        self.scopes.last_mut().unwrap().push(mir_local);
                    }
                }
                MonoStmtKind::Expr(e) => {
                    let operand = self.lower_expr(e);
                    self.release_temporary_value(e, &operand);
                }
            }
        }
        let result = match tail {
            Some(t) => self.lower_escaping_value(t),
            None => Operand::Unit,
        };
        let depth = self.scopes.len() - 1;
        self.release_scopes(depth, escaping_local(&result));
        self.scopes.pop();
        result
    }

    pub(super) fn lower_while(&mut self, cond: &MonoExpr, body: &MonoExpr) -> Operand {
        let header = self.new_block();
        self.terminate_current_to(header);
        self.current = header;
        let cond_op = self.lower_expr(cond);
        let body_block = self.new_block();
        let exit_block = self.new_block();
        self.terminate_current(Terminator::Branch {
            cond: cond_op,
            then_block: body_block,
            else_block: exit_block,
        });

        self.loop_stack.push(LoopCtx {
            break_block: exit_block,
            continue_block: header,
            scope_depth: self.scopes.len(),
        });
        self.current = body_block;
        self.lower_expr(body);
        self.terminate_current_to(header);
        self.loop_stack.pop();

        self.current = exit_block;
        Operand::Unit
    }

    pub(super) fn lower_loop(&mut self, body: &MonoExpr) -> Operand {
        let header = self.new_block();
        self.terminate_current_to(header);
        let exit_block = self.new_block();

        self.loop_stack.push(LoopCtx {
            break_block: exit_block,
            continue_block: header,
            scope_depth: self.scopes.len(),
        });
        self.current = header;
        self.lower_expr(body);
        self.terminate_current_to(header);
        self.loop_stack.pop();

        self.current = exit_block;
        // `loop`'s own type is always unit regardless of `break value` —
        // `nether_typecheck`'s own documented simplification, carried
        // forward unchanged (see this crate's module docs).
        Operand::Unit
    }
}
