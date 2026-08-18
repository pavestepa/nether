use super::*;

impl FnBuilder<'_> {
    pub(super) fn lower_exprs(&mut self, exprs: &[MonoExpr]) -> Vec<Operand> {
        exprs.iter().map(|e| self.lower_expr(e)).collect()
    }

    pub(super) fn lower_call_args(
        &mut self,
        args: &[MonoExpr],
        params: &[Type],
    ) -> (Vec<Operand>, Vec<Option<Operand>>, Vec<Local>) {
        let mut operands = Vec::with_capacity(args.len());
        let mut weak_sources = Vec::with_capacity(args.len());
        let mut moved_sources = Vec::new();
        for (index, arg) in args.iter().enumerate() {
            let constructs_weak = matches!(params.get(index), Some(Type::Weak(_)))
                && !matches!(&arg.ty, Type::Weak(_));
            if constructs_weak {
                // Keep a freshly-computed strong argument alive through
                // the call's full expression. The weak temporary and the
                // strong source are both cleaned up immediately after
                // the call below.
                let strong = self.lower_expr(arg);
                let weak = Operand::Local(self.materialize(
                    Rvalue::Use(strong.clone()),
                    Type::Weak(Box::new(arg.ty.clone())),
                ));
                if let Operand::Local(local) = weak {
                    self.push_instr(Instr::WeakRetain(local));
                    operands.push(Operand::Local(local));
                }
                weak_sources.push(Some(strong));
            } else {
                let operand = self.lower_expr(arg);
                if matches!(params.get(index), Some(Type::Unique(_))) {
                    if let Operand::Local(local) = &operand {
                        if is_trivial_local_alias(arg) {
                            moved_sources.push(*local);
                        }
                    }
                }
                operands.push(operand);
                weak_sources.push(None);
            }
        }
        (operands, weak_sources, moved_sources)
    }

    pub(super) fn release_call_arg_temporaries(
        &mut self,
        args: &[MonoExpr],
        operands: &[Operand],
        weak_sources: &[Option<Operand>],
    ) {
        for ((arg, operand), weak_source) in args.iter().zip(operands).zip(weak_sources) {
            if let Some(strong) = weak_source {
                if let Operand::Local(local) = operand {
                    self.push_instr(Instr::Release(*local));
                }
                self.release_temporary_value(arg, strong);
            } else {
                self.release_temporary_value(arg, operand);
            }
        }
    }

    /// Drops the initial ownership credit of a computed temporary after
    /// its consumer has taken/retained what it needs. Bare local
    /// references are not temporaries; their lexical scope remains their
    /// owner. Constructors routed through `prepare_new_binding` transfer
    /// their credit instead and deliberately do not call this helper.
    pub(super) fn release_temporary_value(&mut self, expr: &MonoExpr, operand: &Operand) {
        if is_trivial_local_alias(expr) || !self.sigs.has_managed_content(&expr.ty, self.defs) {
            return;
        }
        if let Operand::Local(local) = operand {
            self.push_instr(Instr::Release(*local));
        }
    }

    /// Like [`Self::lower_exprs`], but for operands that become a *stored*
    /// field inside a freshly constructed container (`Construct`,
    /// `ConstructVariant`, `Tuple`, `Array`) — each one goes through
    /// [`Self::prepare_new_binding`], exactly like a field/index store's
    /// incoming value (that method's own doc comment already lists this
    /// case), since the new container now independently owns whatever
    /// heap-kind value ends up in each slot, same as any other freshly
    /// bound owner. Deliberately not used for `Call`/`CallBuiltin`/
    /// `CallArrayMethod` arguments — `insert_arc`'s `call_operands` rule
    /// already wraps those in its own retain/release around the call
    /// instruction; routing them through here too would double-retain.
    pub(super) fn lower_stored_exprs(&mut self, exprs: &[MonoExpr]) -> Vec<Operand> {
        exprs.iter().map(|e| self.prepare_new_binding(e)).collect()
    }

    /// Like [`Self::lower_stored_exprs`], but for a `Construct`'s own
    /// fields specifically, since — unlike a `Tuple`/`Array`/
    /// `ConstructVariant`'s elements — a struct's fields have their own,
    /// individually declared types, and a field declared `weak T` needs
    /// [`Self::prepare_weak_binding`] instead of the ordinary heap-retain
    /// logic `prepare_new_binding` applies (which only knows how to
    /// retain a *strong* reference of the source expression's own type,
    /// not construct a fresh weak one). Tuples/enum payloads/arrays
    /// holding a `weak`-typed element directly are a documented gap —
    /// see this module's own docs.
    pub(super) fn lower_construct_fields(
        &mut self,
        ty: &Type,
        fields: &[MonoExpr],
    ) -> Vec<Operand> {
        let declared = self.field_declared_types(ty);
        fields
            .iter()
            .enumerate()
            .map(|(i, f)| match declared.get(i) {
                Some(Type::Weak(_)) => self.prepare_weak_binding(f),
                _ => self.prepare_new_binding(f),
            })
            .collect()
    }

    pub(super) fn field_declared_types(&self, ty: &Type) -> Vec<Type> {
        self.sigs.type_fields(ty).unwrap_or_default()
    }

    /// Evaluates `value` (of heap type `T`) for storage into a `weak T`
    /// -typed field — the value's own strong reference is left
    /// completely untouched (spec §13: "`weak T` never affects the
    /// retain count of its referent"); instead a fresh [`Instr::WeakRetain`]
    /// runs on a newly materialized `weak T`-typed copy, bumping the
    /// *weak* count so `runtime/arc` can safely defer freeing the header
    /// until every weak reference to it has also gone (see
    /// `runtime/arc`'s own module docs on why a weak count exists at
    /// all).
    pub(super) fn prepare_weak_binding(&mut self, value: &MonoExpr) -> Operand {
        let op = self.lower_expr(value);
        let weak_ty = Type::Weak(Box::new(value.ty.clone()));
        let local = self.materialize(Rvalue::Use(op.clone()), weak_ty);
        self.push_instr(Instr::WeakRetain(local));
        // A weak binding never consumes a strong ownership credit.
        // Named values keep their lexical owner; a freshly-computed
        // referent must drop its temporary strong +1 after the weak count
        // has been established.
        self.release_temporary_value(value, &op);
        Operand::Local(local)
    }

    pub(super) fn lower_expr(&mut self, expr: &MonoExpr) -> Operand {
        match &expr.kind {
            MonoExprKind::Literal(l) => {
                let operand = Operand::Literal(l.clone(), expr.ty.clone());
                if self.sigs.has_managed_content(&expr.ty, self.defs) {
                    Operand::Local(self.materialize(Rvalue::Use(operand), expr.ty.clone()))
                } else {
                    operand
                }
            }
            MonoExprKind::Local(id) => Operand::Local(self.local_for(*id)),
            MonoExprKind::Borrow(place) => {
                let place = self.lower_place(place);
                Operand::Local(self.materialize(Rvalue::AddressOf(place), expr.ty.clone()))
            }
            MonoExprKind::Deref(reference) => {
                let reference = self.lower_expr(reference);
                Operand::Local(self.materialize(Rvalue::Deref(reference), expr.ty.clone()))
            }
            MonoExprKind::PromoteUnique(value) => {
                let value = self.lower_expr(value);
                let source = match &value {
                    Operand::Local(local) => Some(*local),
                    _ => None,
                };
                let promoted =
                    Operand::Local(self.materialize(Rvalue::PromoteUnique(value), expr.ty.clone()));
                if let Some(source) = source {
                    self.push_instr(Instr::Clear(source));
                }
                promoted
            }
            MonoExprKind::FnRef(id) => Operand::Local(self.materialize(
                Rvalue::Closure {
                    function: *id,
                    captures: Vec::new(),
                },
                expr.ty.clone(),
            )),
            MonoExprKind::Unit => Operand::Unit,
            MonoExprKind::Tuple(items) => {
                let ops = self.lower_stored_exprs(items);
                Operand::Local(self.materialize(Rvalue::Tuple(ops), expr.ty.clone()))
            }
            MonoExprKind::Array(items) => {
                // `runtime/array_push` retains each stored element via
                // the element shim, so array literals borrow their input
                // operands here and drop only freshly-computed input
                // temporaries after construction.
                let ops = self.lower_exprs(items);
                let result =
                    Operand::Local(self.materialize(Rvalue::Array(ops.clone()), expr.ty.clone()));
                for (item, operand) in items.iter().zip(&ops) {
                    self.release_temporary_value(item, operand);
                }
                result
            }
            MonoExprKind::Concat(items) => {
                let ops = self.lower_exprs(items);
                let result =
                    Operand::Local(self.materialize(Rvalue::Concat(ops.clone()), expr.ty.clone()));
                for (item, operand) in items.iter().zip(&ops) {
                    self.release_temporary_value(item, operand);
                }
                result
            }
            MonoExprKind::ToString(inner) => {
                let op = self.lower_expr(inner);
                Operand::Local(self.materialize(Rvalue::ToString(op), expr.ty.clone()))
            }
            MonoExprKind::Unary { op, expr: inner } => {
                let inner_op = self.lower_expr(inner);
                Operand::Local(self.materialize(Rvalue::Unary(*op, inner_op), expr.ty.clone()))
            }
            MonoExprKind::Binary { op, lhs, rhs } => {
                let l = self.lower_expr(lhs);
                let r = self.lower_expr(rhs);
                Operand::Local(self.materialize(Rvalue::Binary(*op, l, r), expr.ty.clone()))
            }
            MonoExprKind::Assign { target, value } => {
                self.lower_assign(target, value);
                Operand::Unit
            }
            MonoExprKind::Call { callee, args } => {
                let callee_op = self.lower_expr(callee);
                let params = match &callee.ty {
                    Type::Function(params, _) => params.clone(),
                    _ => Vec::new(),
                };
                let (arg_ops, weak_sources, moved_sources) = self.lower_call_args(args, &params);
                let target = match callee_op {
                    Operand::Fn(id) => CallTarget::Fn(id),
                    ref other => CallTarget::Dynamic(other.clone()),
                };
                let result = Operand::Local(self.materialize(
                    Rvalue::Call {
                        target,
                        args: arg_ops.clone(),
                    },
                    expr.ty.clone(),
                ));
                self.release_temporary_value(callee, &callee_op);
                self.release_call_arg_temporaries(args, &arg_ops, &weak_sources);
                for source in moved_sources {
                    self.push_instr(Instr::Clear(source));
                }
                result
            }
            MonoExprKind::CallStatic { fn_id, args } => {
                let target = self.module.get(*fn_id);
                let mut params = Vec::with_capacity(args.len());
                if target.self_param.is_some() {
                    params.push(target.self_ty.clone().unwrap_or(Type::Error));
                }
                params.extend(target.params.iter().map(|param| param.ty.clone()));
                let (arg_ops, weak_sources, moved_sources) = self.lower_call_args(args, &params);
                let result = Operand::Local(self.materialize(
                    Rvalue::Call {
                        target: CallTarget::Fn(*fn_id),
                        args: arg_ops.clone(),
                    },
                    expr.ty.clone(),
                ));
                self.release_call_arg_temporaries(args, &arg_ops, &weak_sources);
                for source in moved_sources {
                    self.push_instr(Instr::Clear(source));
                }
                result
            }
            MonoExprKind::CallBuiltin { name, args } => {
                let arg_ops = self.lower_exprs(args);
                let result = Operand::Local(self.materialize(
                    Rvalue::CallBuiltin {
                        name: name.clone(),
                        args: arg_ops.clone(),
                    },
                    expr.ty.clone(),
                ));
                for (arg, operand) in args.iter().zip(&arg_ops) {
                    self.release_temporary_value(arg, operand);
                }
                result
            }
            MonoExprKind::CallArrayMethod {
                receiver,
                method,
                args,
            } => {
                let recv_op = self.lower_expr(receiver);
                let arg_ops = self.lower_exprs(args);
                let result = Operand::Local(self.materialize(
                    Rvalue::CallArrayMethod {
                        receiver: recv_op.clone(),
                        method: method.clone(),
                        args: arg_ops.clone(),
                    },
                    expr.ty.clone(),
                ));
                self.release_temporary_value(receiver, &recv_op);
                for (arg, operand) in args.iter().zip(&arg_ops) {
                    self.release_temporary_value(arg, operand);
                }
                result
            }
            MonoExprKind::Field { base, index } => {
                let base_op = self.lower_expr(base);
                let result = Operand::Local(self.materialize(
                    Rvalue::Field {
                        base: base_op.clone(),
                        index: *index,
                    },
                    expr.ty.clone(),
                ));
                self.release_temporary_value(base, &base_op);
                result
            }
            MonoExprKind::Index { base, index } => {
                let base_op = self.lower_expr(base);
                let index_op = self.lower_expr(index);
                let result = Operand::Local(self.materialize(
                    Rvalue::Index {
                        base: base_op.clone(),
                        index: index_op.clone(),
                    },
                    expr.ty.clone(),
                ));
                self.release_temporary_value(base, &base_op);
                self.release_temporary_value(index, &index_op);
                result
            }
            MonoExprKind::Construct { ty, fields } => {
                let ops = self.lower_construct_fields(&expr.ty, fields);
                Operand::Local(self.materialize(
                    Rvalue::Construct {
                        ty: *ty,
                        fields: ops,
                    },
                    expr.ty.clone(),
                ))
            }
            MonoExprKind::ConstructVariant {
                enum_id,
                variant,
                payload,
            } => {
                let ops = self.lower_stored_exprs(payload);
                Operand::Local(self.materialize(
                    Rvalue::ConstructVariant {
                        enum_id: *enum_id,
                        variant: *variant,
                        payload: ops,
                    },
                    expr.ty.clone(),
                ))
            }
            MonoExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => self.lower_if(expr, cond, then_branch, else_branch.as_deref()),
            MonoExprKind::Match { scrutinee, arms } => self.lower_match(expr, scrutinee, arms),
            MonoExprKind::Block(stmts, tail) => self.lower_block(stmts, tail.as_deref()),
            MonoExprKind::While { cond, body } => self.lower_while(cond, body),
            MonoExprKind::Loop { body } => self.lower_loop(body),
            MonoExprKind::Break(value) => {
                if let Some(v) = value {
                    let value = self.lower_expr(v);
                    self.release_temporary_value(v, &value);
                }
                let ctx_depth = self
                    .loop_stack
                    .last()
                    .expect("`break` outside a loop — typecheck rejects this")
                    .scope_depth;
                let break_block = self.loop_stack.last().unwrap().break_block;
                self.release_scopes(ctx_depth, None);
                self.terminate_current(Terminator::Goto(break_block));
                Operand::Unit
            }
            MonoExprKind::Continue => {
                let ctx_depth = self
                    .loop_stack
                    .last()
                    .expect("`continue` outside a loop — typecheck rejects this")
                    .scope_depth;
                let continue_block = self.loop_stack.last().unwrap().continue_block;
                self.release_scopes(ctx_depth, None);
                self.terminate_current(Terminator::Goto(continue_block));
                Operand::Unit
            }
            MonoExprKind::Return(value) => {
                let op = match value {
                    Some(v) => self.lower_escaping_value(v),
                    None => Operand::Unit,
                };
                self.release_scopes(0, escaping_local(&op));
                self.terminate_current(Terminator::Return(op));
                Operand::Unit
            }
            MonoExprKind::Closure { function, captures } => {
                let captures = self.lower_stored_exprs(captures);
                Operand::Local(self.materialize(
                    Rvalue::Closure {
                        function: *function,
                        captures,
                    },
                    expr.ty.clone(),
                ))
            }
        }
    }

    /// Lowers `target = value` — whether `target` is a plain, already-
    /// bound `mut` variable or a field/index store (`self.name = x;`).
    /// Neither is one of `arc-model.md`'s literal worked examples (which
    /// only covers fresh `let` bindings), but the same §3.1 reasoning
    /// extends symmetrically to mutation: whatever `target` held loses an
    /// owner here and needs releasing, while the incoming value gains one
    /// (via [`Self::prepare_new_binding`]) — see this crate's module docs
    /// for why this counts as build_mir's job, not insert_arc's.
    pub(super) fn lower_assign(&mut self, target: &MonoExpr, value: &MonoExpr) {
        let place = self.lower_place(target);
        if matches!(&target.ty, Type::Weak(_)) {
            self.lower_weak_assign(place, value);
            return;
        }
        let has_projection = !place.projection.is_empty();
        let value_ty = value.ty.clone();
        let old = if self.sigs.has_managed_content(&value_ty, self.defs) {
            Some(if place.projection.is_empty() {
                // Snapshot the old bits before overwriting the slot.
                // This is an ownership move, not an aliasing copy, so it
                // deliberately receives no retain; the release below
                // consumes the slot's previous credit.
                Operand::Local(
                    self.materialize(Rvalue::Use(Operand::Local(place.local)), value_ty.clone()),
                )
            } else {
                self.read_place(&place, value_ty.clone())
            })
        } else {
            None
        };
        let val_op = self.prepare_new_binding(value);
        self.push_instr(Instr::Assign(place, Rvalue::Use(val_op)));
        if let Some(Operand::Local(old_local)) = old {
            self.push_instr(Instr::Release(old_local));
            if has_projection {
                // For a plain `x = value;` reassignment, `read_place`
                // just names `x`'s own existing storage directly (no new
                // instruction, no incidental retain), so the one release
                // above already correctly drops its one original claim.
                // For a field/index store, `read_place` instead had to
                // *materialize* the old value via its own fresh Field/
                // Index rvalue — which `insert_arc` retains
                // unconditionally, exactly like any other such read (it
                // can't tell this one apart from a normal one). This
                // second release cancels that incidental retain, so the
                // first one above is the only one that nets against the
                // place's actual original reference — see
                // `Self::read_place`.
                self.push_instr(Instr::Release(old_local));
            }
        }
    }

    /// [`Self::lower_assign`]'s branch for a `target` declared `weak T` —
    /// a distinct, simpler path from the general heap case: the old value
    /// (if any — a field/index projection's own previous pointer) is
    /// released via [`Instr::WeakRelease`], never [`Instr::Release`], and
    /// the incoming value is bound via [`Self::prepare_weak_binding`]
    /// rather than `prepare_new_binding`'s ordinary heap-retain logic
    /// (which only knows how to retain a *strong* reference of the
    /// source's own type, not construct a fresh weak one). No incidental
    /// -retain-cancelling second release is needed here the way the heap
    /// case needs one: `insert_arc`'s bind-time-retain rule only ever
    /// fires for a *heap-kind* destination, so materializing the old
    /// value through a fresh `Field`/`Index` read never gets one to begin
    /// with.
    pub(super) fn lower_weak_assign(&mut self, place: Place, value: &MonoExpr) {
        let has_projection = !place.projection.is_empty();
        let weak_ty = Type::Weak(Box::new(value.ty.clone()));
        let old_op = if place.projection.is_empty() {
            Operand::Local(self.materialize(Rvalue::Use(Operand::Local(place.local)), weak_ty))
        } else {
            self.read_place(&place, weak_ty)
        };
        let val_op = self.prepare_weak_binding(value);
        self.push_instr(Instr::Assign(place, Rvalue::Use(val_op)));
        if let Operand::Local(old_local) = old_op {
            self.push_instr(Instr::WeakRelease(old_local));
            if has_projection {
                // The projected read received an incidental weak retain
                // from the generic aliasing-read rule; cancel that in
                // addition to dropping the field's original weak credit.
                self.push_instr(Instr::WeakRelease(old_local));
            }
        }
    }
}
