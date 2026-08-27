use super::*;
use crate::check::checker::count_local_uses_in_expr;

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
            ExprKind::FixedArray(elems) => self.check_fixed_array(elems, expected, expr.span),
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
            ExprKind::RawBorrow { mutable, place } => self.check_raw_borrow(*mutable, place),
            ExprKind::RawDeref(pointer) => self.check_raw_deref(pointer, expr.span),
            ExprKind::Unary { op, expr: inner } => self.check_unary(*op, inner),
            ExprKind::Binary { op, lhs, rhs } => self.check_binary(*op, lhs, rhs),
            ExprKind::Assign { target, value } => self.check_assign(target, value),
            ExprKind::Call {
                callee,
                generic_args,
                args,
            } => {
                if let ExprKind::Path(path) = &callee.kind {
                    if path.segments.len() == 2
                        && path.segments[0].name.as_str() == "task"
                        && path.segments[1].name.as_str() == "spawn"
                    {
                        if !self.in_async {
                            self.err(
                                expr.span,
                                "`task.spawn` is only allowed inside an `async fn`",
                            );
                        }
                        if !generic_args.is_empty() {
                            self.err(path.segments[1].span, "`task.spawn` is not generic");
                        }
                        if args.len() != 1 {
                            self.err(
                                expr.span,
                                format!("`task.spawn` expects 1 argument, found {}", args.len()),
                            );
                            return Type::Error;
                        }
                        return match self.check_expr(&args[0]) {
                            task @ Type::Task(_) => {
                                self.check_spawn_capture_borrows(&args[0], "task");
                                task
                            }
                            Type::Error => Type::Error,
                            other => {
                                let found = self.describe(&other);
                                self.err(
                                    args[0].span,
                                    format!("`task.spawn` expects a task, found `{found}`"),
                                );
                                Type::Error
                            }
                        };
                    }
                    if path.segments.len() == 2
                        && path.segments[0].name.as_str() == "thread"
                        && path.segments[1].name.as_str() == "spawn"
                    {
                        if !generic_args.is_empty() {
                            self.err(path.segments[1].span, "`thread.spawn` is not generic");
                        }
                        if args.len() != 1 {
                            self.err(
                                expr.span,
                                format!(
                                    "`thread.spawn` expects 1 argument, found {}",
                                    args.len()
                                ),
                            );
                            return Type::Error;
                        }
                        return self.check_thread_spawn(&args[0]);
                    }
                    if path.segments.len() == 2
                        && path.segments[0].name.as_str() == "timer"
                        && path.segments[1].name.as_str() == "sleep"
                    {
                        if !generic_args.is_empty() {
                            self.err(path.segments[1].span, "`timer.sleep` is not generic");
                        }
                        if args.len() != 1 {
                            self.err(
                                expr.span,
                                format!("`timer.sleep` expects 1 argument, found {}", args.len()),
                            );
                        }
                        for argument in args {
                            let ty = self.check_expr(argument);
                            if !matches!(ty, Type::Primitive(kind) if kind.is_integer()) {
                                self.err(
                                    argument.span,
                                    "`timer.sleep` expects integer milliseconds",
                                );
                            }
                        }
                        return Type::Task(Box::new(Type::unit()));
                    }
                }
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
                if let ExprKind::Path(path) = &receiver.kind {
                    if path.segments.len() == 1
                        && path.segments[0].name.as_str() == "timer"
                        && method.name.as_str() == "sleep"
                    {
                        if !generic_args.is_empty() {
                            self.err(method.span, "`timer.sleep` is not generic");
                        }
                        if args.len() != 1 {
                            self.err(
                                expr.span,
                                format!("`timer.sleep` expects 1 argument, found {}", args.len()),
                            );
                        }
                        for argument in args {
                            let ty = self.check_expr(argument);
                            if !matches!(ty, Type::Primitive(kind) if kind.is_integer()) {
                                self.err(
                                    argument.span,
                                    "`timer.sleep` expects integer milliseconds",
                                );
                            }
                        }
                        return Type::Task(Box::new(Type::unit()));
                    }
                }
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
            ExprKind::Unsafe(block) => self.check_unsafe_block(block, expected),
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
                mut_capture,
                params,
                body,
            } => self.check_closure(
                expr.id,
                *move_capture,
                *mut_capture,
                params,
                body,
                expected,
            ),
            ExprKind::StructLit {
                path,
                fields,
                owned,
            } => self.check_struct_lit(path, fields, *owned, expected),
        }
    }

    /// `[1, 2, 3]` (language-spec §2.4, Stage 7) — always `Type::Array`
    /// (growable, heap-backed). Before Stage 7 this also coerced into
    /// `Type::FixedArray` whenever the expected type called for one;
    /// that context-driven ambiguity is gone — `{1, 2, 3}` ([`Self::check_fixed_array`])
    /// is now the only way to construct a fixed-size literal, so a
    /// `[...]` literal used where a `{T, N}` is expected is now an
    /// ordinary type mismatch, exactly like any other wrong-type literal.
    pub(super) fn check_array(
        &mut self,
        elems: &[Expr],
        expected: Option<&Type>,
        span: Span,
    ) -> Type {
        if elems.is_empty() {
            return match expected {
                Some(Type::Array(inner)) => Type::Array(inner.clone()),
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
            _ => None,
        };
        let first = self.check_array_elems(elems, elem_expected.as_ref(), "array");
        Type::Array(Box::new(first))
    }

    /// `{1, 2, 3}` (language-spec §2.4, Stage 7) — always
    /// `Type::FixedArray` (inline, stack-allocated), mirroring
    /// [`Self::check_array`] except for the length check every fixed
    /// array needs (its size is part of the type).
    pub(super) fn check_fixed_array(
        &mut self,
        elems: &[Expr],
        expected: Option<&Type>,
        span: Span,
    ) -> Type {
        if elems.is_empty() {
            return match expected {
                Some(Type::FixedArray(inner, length))
                    if matches!(length.as_ref(), Type::Const(0)) =>
                {
                    Type::FixedArray(inner.clone(), length.clone())
                }
                _ => {
                    self.err(
                        span,
                        "cannot infer the element type of an empty fixed-array literal without context",
                    );
                    Type::Error
                }
            };
        }
        let elem_expected = match expected {
            Some(Type::FixedArray(inner, _)) => Some((**inner).clone()),
            _ => None,
        };
        let first = self.check_array_elems(elems, elem_expected.as_ref(), "fixed-array");
        let length = Box::new(Type::Const(elems.len() as u128));
        if let Some(Type::FixedArray(_, expected_length)) = expected {
            if let Type::Const(expected_len) = expected_length.as_ref() {
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
        }
        Type::FixedArray(Box::new(first), length)
    }

    /// Shared element-checking loop for [`Self::check_array`]/
    /// [`Self::check_fixed_array`]: the first element seeds the array's
    /// element type (using `elem_expected` as a hint), every later
    /// element must be compatible with it. `what` names the literal kind
    /// in the mismatch diagnostic only.
    fn check_array_elems(
        &mut self,
        elems: &[Expr],
        elem_expected: Option<&Type>,
        what: &str,
    ) -> Type {
        let first = self.check_expr_with_expected(&elems[0], elem_expected);
        for e in &elems[1..] {
            let t = self.check_expr_with_expected(e, Some(&first));
            if !t.compatible(&first) {
                let expected_s = self.describe(&first);
                let found_s = self.describe(&t);
                self.err(
                    e.span,
                    format!("expected `{expected_s}`, found `{found_s}` in {what} literal"),
                );
            }
        }
        first
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
        // `*p = value;` — writing through a raw pointer isn't rooted at a
        // local's own storage at all (it's rooted at whatever memory the
        // pointer happens to point to), so it bypasses every ordinary
        // assignment-target check below entirely (mutability/exclusivity
        // through a raw pointer is the programmer's own responsibility,
        // per `unsafe`'s whole premise) — handled by its own dedicated
        // path instead.
        if let ExprKind::RawDeref(pointer) = &target.kind {
            return self.check_raw_deref_assign(pointer, target.span, value);
        }
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
            // `(*p).field = value;` — a field reached *through* a raw
            // pointer deref, unlike `*p = value;` itself (handled directly
            // in `check_assign`, never reaching this function at all).
            // Accepted unconditionally for the same reason `check_assign`'s
            // own `RawDeref` special-case bypasses every other check here:
            // there is no local root to validate, by design.
            ExprKind::RawDeref(_) => true,
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

    /// `&raw const place` / `&raw mut place` (language-spec §17, Stage 5)
    /// — takes the address of an arbitrary place as a raw pointer,
    /// bypassing the unique-ownership borrow-tracking system entirely
    /// (`Type::RawConstPtr`'s own docs). Reuses
    /// [`Self::assign_target_has_local_root`] for the same "is this
    /// actually an addressable place" shape check an assignment target
    /// needs — deliberately *not* `check_assign_target_mutable`'s
    /// mutability/exclusivity checks: forming a raw pointer is always
    /// safe regardless of whether the underlying place is declared `mut`
    /// or currently borrowed (only *dereferencing* the result is
    /// unsafe — see [`Self::check_raw_deref`]).
    pub(super) fn check_raw_borrow(&mut self, mutable: bool, place: &Expr) -> Type {
        if !self.assign_target_has_local_root(place) {
            self.err(
                place.span,
                "`&raw const`/`&raw mut` requires a place expression — a local binding or one of its fields/elements/dereferences",
            );
        }
        let inner = self.check_expr(place);
        if mutable {
            Type::RawMutPtr(Box::new(inner))
        } else {
            Type::RawConstPtr(Box::new(inner))
        }
    }

    /// Prefix `*pointer` used as a *value* (a read) — language-spec §17,
    /// Stage 5. `*pointer` as an *assignment target* (`*p = value;`) is
    /// [`Self::check_raw_deref_assign`] instead, since a write additionally
    /// requires `*mut`, not merely any raw pointer.
    pub(super) fn check_raw_deref(&mut self, pointer: &Expr, span: Span) -> Type {
        if !self.in_unsafe {
            self.err(
                span,
                "dereferencing a raw pointer is only allowed inside an `unsafe` block or function",
            );
        }
        match self.check_expr(pointer) {
            Type::RawConstPtr(pointee) | Type::RawMutPtr(pointee) => *pointee,
            Type::Error => Type::Error,
            other => {
                let found = self.describe(&other);
                self.err(
                    pointer.span,
                    format!("cannot dereference `{found}`; expected a raw pointer"),
                );
                Type::Error
            }
        }
    }

    /// `*pointer = value;` — see [`Self::check_raw_deref`] for the
    /// read-position counterpart. Requires `*mut` specifically: writing
    /// through a `*const` pointer is never legal, `unsafe` or not (the
    /// same distinction C itself makes).
    fn check_raw_deref_assign(&mut self, pointer: &Expr, target_span: Span, value: &Expr) -> Type {
        if !self.in_unsafe {
            self.err(
                target_span,
                "dereferencing a raw pointer is only allowed inside an `unsafe` block or function",
            );
        }
        let pointee = match self.check_expr(pointer) {
            Type::RawMutPtr(inner) => *inner,
            Type::RawConstPtr(_) => {
                self.err(
                    target_span,
                    "cannot assign through a `*const` pointer — expected `*mut`",
                );
                Type::Error
            }
            Type::Error => Type::Error,
            other => {
                let found = self.describe(&other);
                self.err(
                    pointer.span,
                    format!("cannot dereference `{found}`; expected a raw pointer"),
                );
                Type::Error
            }
        };
        let value_ty = self.check_expr_with_expected(value, Some(&pointee));
        if !pointee.is_error() && !value_ty.compatible(&pointee) {
            let expected = self.describe(&pointee);
            let found = self.describe(&value_ty);
            self.err(
                value.span,
                format!("expected `{expected}`, found `{found}`"),
            );
        }
        Type::unit()
    }

    /// `thread.spawn(move () => { ... })` (language-spec §19, Stage 6) —
    /// requires syntactically a zero-parameter `move` closure literal
    /// (mirrors `task.spawn`'s own "argument must be a call to an async
    /// fn" shape restriction: keeps capture tracking exact rather than
    /// needing to trace an already-constructed closure value's own
    /// provenance). Every captured value must be [`crate::send_sync::is_send`]
    /// (a genuinely new OS thread, unlike `task.spawn`'s cooperative
    /// single-thread model) and every captured *reference*'s origin must
    /// not be a local owned by this function (spawned-borrow diagnostic —
    /// reuses the same origin classification already proven sound for
    /// [`Self::check_returned_reference_origin`]'s analogous
    /// return-position rule, applied at this new trigger point instead).
    ///
    /// v1 scope decision: the closure must return `()` — an arbitrary
    /// return value would need a per-call-site result-boxing wrapper
    /// function in codegen (mirroring `Task<T>`'s own frame machinery);
    /// `Thread<T>` stays generic at the `Type` level for forward
    /// compatibility, but only `Thread<()>` is ever constructed today.
    fn check_thread_spawn(&mut self, arg: &Expr) -> Type {
        let ExprKind::Closure {
            move_capture,
            params,
            body,
            ..
        } = &arg.kind
        else {
            self.err(
                arg.span,
                "`thread.spawn` requires a `move` closure literal argument, e.g. \
                 `thread.spawn(move () => { ... })`",
            );
            self.check_expr(arg);
            return Type::Error;
        };
        if !move_capture {
            self.err(arg.span, "a `thread.spawn` closure must be `move`");
        }
        if !params.is_empty() {
            self.err(arg.span, "a `thread.spawn` closure takes no parameters");
        }

        // Every outer local this closure body reads — not just
        // reference-typed ones, unlike `check_closure`'s own filtering:
        // a moved *value* capture needs a `Send` check too, computed
        // from each local's already-known outer type, before entering
        // the closure's own scope (mirrors `check_closure`'s own
        // ordering, `control.rs`).
        let captured_locals = self.check_spawn_capture_borrows(body, "thread");
        for &local in &captured_locals {
            if let Some((ty, _)) = self.locals.get(&local).cloned() {
                if !crate::send_sync::is_send(&ty, &self.resolved.definitions, self.sigs) {
                    let found = self.describe(&ty);
                    self.err(
                        arg.span,
                        format!(
                            "cannot spawn a thread capturing a value of type `{found}` — it is \
                             not `Send` (does it need `unsafe impl {found} Send {{ }}`?)"
                        ),
                    );
                }
            }
        }

        // `self.check_expr(arg)` (not `check_closure` directly) — its
        // wrapping is what records `self.expr_types[arg.id]`, which HIR's
        // own `ty_of` later depends on to lower this closure's *result*
        // correctly; calling `check_closure` alone here previously left
        // that entry missing, silently defaulting to `Type::Error` (`i1`
        // in codegen) instead of the closure's real `Type::Function`.
        let ret_ty = self.check_expr(arg);
        let Type::Function(_, inner_ret) = ret_ty else {
            unreachable!("check_closure always returns Type::Function")
        };
        if !inner_ret.is_error() && !inner_ret.compatible(&Type::unit()) {
            let found = self.describe(&inner_ret);
            self.err(
                arg.span,
                format!(
                    "a `thread.spawn` closure must return `()`, found `{found}` — \
                     returning a value from a spawned thread is not yet supported"
                ),
            );
        }
        Type::Thread(Box::new(Type::unit()))
    }

    /// Spawned-borrow diagnostic (language-spec §19, Stage 6), shared by
    /// `task.spawn`/`thread.spawn`: every outer local `expr` reads whose
    /// origin is `BorrowOrigin::Local` (a borrow of something owned *by
    /// this function*) may not outlive the spawned task/thread, since a
    /// detached task/thread's lifetime is independent of this function's
    /// own stack frame — reuses exactly the same origin-classification
    /// rule already proven sound for
    /// [`Self::check_returned_reference_origin`]'s analogous
    /// return-position case, just triggered here instead of at `return`.
    /// `BorrowOrigin::Parameter` remains allowed, by the same reasoning:
    /// a borrow received from further up the call stack is exactly as
    /// safe to hand off here as it would be to return outright. Returns
    /// every captured local found (including non-reference ones), so
    /// `thread.spawn`'s own caller can reuse the same scan for its
    /// additional `Send` check.
    fn check_spawn_capture_borrows(&mut self, expr: &Expr, kind: &str) -> Vec<LocalId> {
        let mut capture_counts = HashMap::new();
        let mut ignored_pins = HashSet::new();
        count_local_uses_in_expr(
            expr,
            self.resolved,
            &mut capture_counts,
            &mut ignored_pins,
            false,
        );
        let captured_locals: Vec<LocalId> = capture_counts
            .keys()
            .copied()
            .filter(|local| self.locals.contains_key(local))
            .collect();
        for &local in &captured_locals {
            if let Some(BorrowOrigin::Local(_)) = self.reference_origins.get(&local) {
                self.err(
                    expr.span,
                    format!(
                        "cannot spawn a {kind} capturing a borrow of a local owned by this \
                         function — it may not outlive the spawned {kind}"
                    ),
                );
            }
        }
        captured_locals
    }
}
