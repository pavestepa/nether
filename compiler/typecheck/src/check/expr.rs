use super::*;

impl Checker<'_> {
    pub(super) fn check_expr(&mut self, expr: &Expr) -> Type {
        self.check_expr_with_expected(expr, None)
    }

    pub(super) fn check_expr_with_expected(
        &mut self,
        expr: &Expr,
        expected: Option<&Type>,
    ) -> Type {
        let mut synthesized = self
            .sigs
            .normalize_associated(&self.synth_expr(expr, expected));
        let normalized_expected = expected.map(|ty| self.sigs.normalize_associated(ty));
        if let Some(expected @ (Type::Any(trait_id, args) | Type::Some(trait_id, args))) =
            normalized_expected.as_ref()
        {
            if &synthesized != expected {
                let bound = GenericBound {
                    trait_id: *trait_id,
                    args: args.clone(),
                };
                if self.type_satisfies_bound(&synthesized, &bound) {
                    if matches!(expected, Type::Some(_, _)) {
                        match &self.opaque_witness {
                            Some(existing) if existing != &synthesized => self.err(
                                expr.span,
                                format!(
                                    "all returns of an opaque `some` type must use one concrete type; found `{}` after `{}`",
                                    self.describe(&synthesized),
                                    self.describe(existing)
                                ),
                            ),
                            None => self.opaque_witness = Some(synthesized.clone()),
                            _ => {}
                        }
                    }
                    self.existential_coercions
                        .insert(expr.id, synthesized.clone());
                    synthesized = expected.clone();
                } else if !synthesized.is_error() {
                    self.err(
                        expr.span,
                        format!(
                            "`{}` does not implement `{}`",
                            self.describe(&synthesized),
                            self.describe_bound(&bound)
                        ),
                    );
                    synthesized = Type::Error;
                }
            }
        }
        let ty = normalized_expected
            .as_ref()
            .map(|expected| contextualize_unknowns(&synthesized, expected))
            .unwrap_or(synthesized);
        self.validate_type_bounds(&ty, expr.span);
        self.expr_types.insert(expr.id, ty.clone());
        ty
    }

    pub(super) fn synth_expr(&mut self, expr: &Expr, expected: Option<&Type>) -> Type {
        match &expr.kind {
            // A bare integer/float literal carries no ownership qualifier
            // of its own — `:i32`'s only construction syntax is an
            // ordinary literal in an owned-typed position (language-spec
            // §3/§4.4's own `let count: i32 = 10;` example), not a
            // separate `:10` form. So an owned-inline *expected* type
            // (`Type::Unique(Primitive(_))`) is honored here directly,
            // the same way an ordinary expected primitive type already
            // picks the literal's concrete `PrimitiveKind`.
            ExprKind::Literal(Literal::Int(_)) => match expected {
                Some(Type::Primitive(p)) if p.is_integer() => Type::Primitive(*p),
                Some(Type::Unique(inner)) if matches!(inner.as_ref(), Type::Primitive(p) if p.is_integer()) => {
                    Type::Unique(inner.clone())
                }
                _ => Type::Primitive(PrimitiveKind::I32),
            },
            ExprKind::Literal(Literal::Float(_)) => match expected {
                Some(Type::Primitive(p)) if p.is_float() => Type::Primitive(*p),
                Some(Type::Unique(inner)) if matches!(inner.as_ref(), Type::Primitive(p) if p.is_float()) => {
                    Type::Unique(inner.clone())
                }
                _ => Type::Primitive(PrimitiveKind::F64),
            },
            ExprKind::Literal(Literal::Bool(_)) => match expected {
                Some(Type::Unique(inner))
                    if matches!(inner.as_ref(), Type::Primitive(PrimitiveKind::Bool)) =>
                {
                    Type::Unique(inner.clone())
                }
                _ => Type::Primitive(PrimitiveKind::Bool),
            },
            ExprKind::Literal(Literal::Char(_)) => match expected {
                Some(Type::Unique(inner))
                    if matches!(inner.as_ref(), Type::Primitive(PrimitiveKind::Char)) =>
                {
                    Type::Unique(inner.clone())
                }
                _ => Type::Primitive(PrimitiveKind::Char),
            },
            ExprKind::Literal(Literal::Str(_)) => match expected {
                Some(Type::Unique(inner)) if matches!(inner.as_ref(), Type::String) => {
                    Type::Unique(inner.clone())
                }
                _ => Type::String,
            },
            ExprKind::Path(path) => self.check_value_path(path, None, expected, None, &[]),
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
            ExprKind::Await(inner) => {
                if !self.in_async {
                    self.err(expr.span, "`await` is only allowed inside an `async fn`");
                }
                match self.check_expr(inner) {
                    Type::Task(output) => *output,
                    Type::Error => Type::Error,
                    other => {
                        let found = self.describe(&other);
                        self.err(
                            inner.span,
                            format!("cannot await `{found}`; expected a task"),
                        );
                        Type::Error
                    }
                }
            }
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
            ExprKind::Call {
                callee,
                generic_args,
                args,
            } => {
                let generic_args = self.lower_call_generic_args(generic_args);
                self.check_call(expr.id, callee, &generic_args, args, expected)
            }
            ExprKind::MutArg(inner) => self.check_expr(inner),
            ExprKind::MethodCall {
                receiver,
                method,
                generic_args,
                args,
            } => {
                let generic_args = self.lower_call_generic_args(generic_args);
                let receiver_ty = self.check_expr(receiver);
                let receiver_mutable = self.place_root_mutable(receiver);
                let receiver_local = self.bare_local_of(receiver);
                self.check_method_call_on(
                    &receiver_ty,
                    method,
                    &generic_args,
                    args,
                    method.span,
                    Some(expr.id),
                    receiver_mutable,
                    receiver_local,
                )
            }
            ExprKind::Field { base, field } => {
                let base_ty = self.check_expr(base);
                self.check_field_access(&base_ty, field)
            }
            ExprKind::Index { base, index } => self.check_index(base, index),
            ExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => self.check_if(cond, then_branch, else_branch, expected),
            ExprKind::Match { scrutinee, arms } => {
                self.check_match(scrutinee, arms, expr.span, expected)
            }
            ExprKind::Block(block) => self.check_block_with_expected(block, expected),
            ExprKind::While { cond, body } => {
                let cond_ty = self.check_expr(cond);
                self.require_bool(&cond_ty, cond.span, "`while` condition");
                self.loop_depth += 1;
                self.check_loop_body_with_fixpoint(|this| {
                    this.check_block(body);
                });
                self.loop_depth -= 1;
                Type::unit()
            }
            ExprKind::ForIn {
                pattern,
                iter,
                body,
            } => self.check_for_in(pattern, iter, body),
            ExprKind::Loop { body } => {
                self.loop_depth += 1;
                self.check_loop_body_with_fixpoint(|this| {
                    this.check_block(body);
                });
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
                    self.err(expr.span, "`continue` is only valid inside a loop");
                }
                Type::Never
            }
            ExprKind::Return(value) => self.check_return(value, expr.span),
            ExprKind::Closure {
                move_capture,
                params,
                body,
            } => self.check_closure(expr.id, *move_capture, params, body, expected),
            ExprKind::StructLit {
                path,
                fields,
                owned,
            } => self.check_struct_lit(path, fields, *owned, expected),
        }
    }

    pub(super) fn check_array(
        &mut self,
        elems: &[Expr],
        expected: Option<&Type>,
        span: Span,
    ) -> Type {
        if elems.is_empty() {
            return match expected {
                Some(Type::Array(inner)) => Type::Array(inner.clone()),
                Some(Type::FixedArray(inner, length))
                    if matches!(length.as_ref(), Type::Const(0)) =>
                {
                    Type::FixedArray(inner.clone(), length.clone())
                }
                _ => {
                    self.err(
                        span,
                        "cannot infer the element type of an empty array literal without context",
                    );
                    Type::Error
                }
            };
        }
        let elem_expected = match expected {
            Some(Type::Array(inner)) => Some((**inner).clone()),
            Some(Type::FixedArray(inner, _)) => Some((**inner).clone()),
            _ => None,
        };
        let first = self.check_expr_with_expected(&elems[0], elem_expected.as_ref());
        for e in &elems[1..] {
            let t = self.check_expr_with_expected(e, Some(&first));
            if !t.compatible(&first) {
                let expected_s = self.describe(&first);
                let found_s = self.describe(&t);
                self.err(
                    e.span,
                    format!("expected `{expected_s}`, found `{found_s}` in array literal"),
                );
            }
        }
        match expected {
            Some(Type::FixedArray(_, length)) => {
                if let Type::Const(expected_len) = length.as_ref() {
                    if *expected_len != elems.len() as u128 {
                        self.err(
                            span,
                            format!(
                                "fixed array expects {expected_len} element(s), found {}",
                                elems.len()
                            ),
                        );
                    }
                }
                Type::FixedArray(Box::new(first), length.clone())
            }
            _ => Type::Array(Box::new(first)),
        }
    }

    pub(super) fn require_bool(&mut self, ty: &Type, span: Span, what: &str) {
        if !matches!(ty, Type::Primitive(PrimitiveKind::Bool)) && !ty.is_error() {
            self.err(span, format!("{what} must be `bool`"));
        }
    }

    pub(super) fn check_unary(&mut self, op: UnaryOp, inner: &Expr) -> Type {
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

    pub(super) fn check_binary(&mut self, op: BinaryOp, lhs: &Expr, rhs: &Expr) -> Type {
        let lhs_ty = self.check_expr(lhs);
        let rhs_ty = self.check_expr_with_expected(rhs, Some(&lhs_ty));
        match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => {
                if !matches!(&lhs_ty, Type::Primitive(p) if p.is_numeric()) && !lhs_ty.is_error() {
                    self.err(lhs.span, "arithmetic operators require numeric operands");
                    return Type::Error;
                }
                if !lhs_ty.compatible(&rhs_ty) {
                    self.err(
                        rhs.span,
                        "both operands of an arithmetic operator must have the same type",
                    );
                    return Type::Error;
                }
                lhs_ty
            }
            BinaryOp::Eq | BinaryOp::Ne => {
                if !lhs_ty.compatible(&rhs_ty) {
                    self.err(
                        rhs.span,
                        "both operands of `==`/`!=` must have the same type",
                    );
                } else if self.sigs.declares_eq(&lhs_ty, &self.resolved.definitions)
                    && !self.sigs.can_derive_eq(&lhs_ty, &self.resolved.definitions)
                {
                    self.err(
                        lhs.span,
                        "cannot derive `Eq` for this type — every field must be primitive or another finite `Eq` struct",
                    );
                }
                Type::Primitive(PrimitiveKind::Bool)
            }
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                if !matches!(&lhs_ty, Type::Primitive(p) if p.is_numeric()) && !lhs_ty.is_error() {
                    self.err(lhs.span, "comparison operators require numeric operands");
                } else if !lhs_ty.compatible(&rhs_ty) {
                    self.err(
                        rhs.span,
                        "both operands of a comparison must have the same type",
                    );
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

    pub(super) fn check_assign(&mut self, target: &Expr, value: &Expr) -> Type {
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
            self.err(
                value.span,
                format!("expected `{expected}`, found `{found}`"),
            );
        }
        self.check_assign_target_mutable(target);
        // A fresh value now lives in `target` — a bare-local target's own
        // prior move (if any) no longer applies (mirrors Rust: overwriting
        // a moved-from binding makes it live again). Only meaningful for a
        // bare local, not `dog.field = ...`: fields are never themselves
        // `Type::Unique` (language-spec §4.2), so `dog` itself was never
        // marked moved by writing through it in the first place.
        if let ExprKind::Path(path) = &target.kind {
            if let Some(res) = self.resolved.path_res.get(&path.id) {
                if path.segments.len() == 1 {
                    if let Resolution::Local(id) = res.base {
                        self.moved.remove(&id);
                    }
                }
            }
        }
        Type::unit()
    }

    pub(super) fn assign_target_has_local_root(&self, target: &Expr) -> bool {
        match &target.kind {
            ExprKind::Path(path) => self
                .resolved
                .path_res
                .get(&path.id)
                .is_some_and(|resolution| matches!(resolution.base, Resolution::Local(_))),
            ExprKind::Field { base, .. } | ExprKind::Index { base, .. } => {
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
    pub(super) fn check_assign_target_type(&mut self, target: &Expr) -> Type {
        match &target.kind {
            // `self.field`/`x.field` is one multi-segment `Path` node, not
            // `ExprKind::Field` (spec §10 — module/static/member access
            // all share `.`, so the parser only ever produces a dotted
            // -path node for a bare identifier chain like this).
            ExprKind::Path(path) => self.check_assign_path_type(path),
            ExprKind::Field {
                base,
                field: nether_ast::FieldAccessor::Named(ident),
            } => {
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
    pub(super) fn check_assign_path_type(&mut self, path: &Path) -> Type {
        let Some(res) = self.resolved.path_res.get(&path.id).cloned() else {
            return Type::Error;
        };
        let total = path.segments.len();
        let mut current_ty = match res.base {
            Resolution::Local(id) => self
                .locals
                .get(&id)
                .map(|(t, _)| t.clone())
                .unwrap_or(Type::Error),
            // No other resolution kind can ever name a `weak`-typed place
            // (only locals/fields can be declared `weak T`), so falling
            // back to the normal, upgrading path is safe here.
            _ => return self.check_value_path(path, None, None, None, &[]),
        };
        for i in res.consumed..total {
            let seg = &path.segments[i];
            current_ty = if i + 1 == total {
                self.field_type_named(&current_ty, seg)
            } else {
                self.check_field_access_named(&current_ty, seg)
            };
        }
        if res.consumed == total {
            if let Type::MutRef(inner) = current_ty {
                if crate::alloc::alloc_kind(&inner, &self.resolved.definitions)
                    == crate::alloc::AllocKind::Stack
                {
                    return *inner;
                }
                return Type::MutRef(inner);
            }
        }
        current_ty
    }

    /// `Some(id)` when `expr` is *exactly* a bare single-segment path
    /// resolving to a local (`dog`, not `dog.name` or `holder.dog`) — the
    /// only shape a method-call receiver can be for the call to be a move
    /// candidate at all (see `Checker::check_move`'s docs). Normally
    /// unreachable from `ExprKind::MethodCall` (the parser folds a bare-
    /// local-then-call into a multi-segment `Path`, handled directly in
    /// `check_value_path`), kept here defensively for whichever shapes do
    /// still reach this node.
    pub(super) fn bare_local_of(&self, expr: &Expr) -> Option<LocalId> {
        let ExprKind::Path(path) = &expr.kind else {
            return None;
        };
        if path.segments.len() != 1 {
            return None;
        }
        match self.resolved.path_res.get(&path.id)?.base {
            Resolution::Local(id) => Some(id),
            _ => None,
        }
    }

    /// The mutability of `expr`'s root local, or `None` if `expr` isn't a
    /// traceable place (a dotted Path/Field/Index chain rooted at a local)
    /// — e.g. a temporary (struct literal, call result), which can't
    /// satisfy a mutation requirement either way.
    ///
    /// `self.field`/`x.a.b.c` is one multi-segment `Path` node, not
    /// nested `ExprKind::Field`s (doc comment on
    /// [`Self::check_assign_target_type`]), so the `Path` arm below
    /// deliberately ignores `PathResolution::consumed`: `res.base`
    /// already identifies the root local regardless of how many trailing
    /// field segments follow it, and mutability of everything reachable
    /// through that root — a direct field write, or a `mut self` method
    /// call anywhere along the chain — is governed by that one root,
    /// exactly like Rust's own field-mutation rule (`holder.child.name =
    /// x` requires `holder` itself to be `mut`, transitively).
    pub(super) fn place_root_mutable(&self, expr: &Expr) -> Option<bool> {
        match &expr.kind {
            ExprKind::Path(path) => {
                let res = self.resolved.path_res.get(&path.id)?;
                match res.base {
                    Resolution::Local(id) => self.locals.get(&id).map(|(_, m)| *m),
                    _ => None,
                }
            }
            ExprKind::Field { base, .. } | ExprKind::Index { base, .. } => {
                self.place_root_mutable(base)
            }
            _ => None,
        }
    }

    pub(super) fn place_root_local(&self, expr: &Expr) -> Option<LocalId> {
        match &expr.kind {
            ExprKind::Path(path) => match self.resolved.path_res.get(&path.id)?.base {
                Resolution::Local(id) => Some(id),
                _ => None,
            },
            ExprKind::Field { base, .. } | ExprKind::Index { base, .. } => {
                self.place_root_local(base)
            }
            _ => None,
        }
    }

    pub(super) fn check_assign_target_mutable(&mut self, target: &Expr) {
        if self
            .place_root_local(target)
            .is_some_and(|id| self.active_borrows.contains_key(&id))
        {
            self.err(target.span, "cannot mutate a value while it is borrowed");
        }
        if let Some(false) = self.place_root_mutable(target) {
            self.err(
                target.span,
                "cannot assign to an immutable binding — declare it with `let mut`",
            );
        }
    }
}
