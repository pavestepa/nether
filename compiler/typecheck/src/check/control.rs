use super::*;

impl Checker<'_> {
    /// Reading a `weak T`-typed place *as a value* yields `Option<T>`,
    /// forcing every use to handle a value that may already be gone.
    pub(super) fn upgrade_weak(&self, ty: Type) -> Type {
        match ty {
            Type::Weak(inner) => match self.resolved.definitions.lookup(&Symbol::new("Option")) {
                Some(option_id) => Type::Enum(option_id, vec![*inner]),
                None => Type::Error,
            },
            other => other,
        }
    }

    pub(super) fn check_field_access(
        &mut self,
        base_ty: &Type,
        field: &nether_ast::FieldAccessor,
    ) -> Type {
        match field {
            nether_ast::FieldAccessor::Named(ident) => {
                self.check_field_access_named(base_ty, ident)
            }
            nether_ast::FieldAccessor::Index(idx, span) => {
                self.check_tuple_index(base_ty, *idx, *span)
            }
        }
    }

    pub(super) fn check_field_access_named(&mut self, base_ty: &Type, ident: &Ident) -> Type {
        let ty = self.field_type_named(base_ty, ident);
        self.upgrade_weak(ty)
    }

    /// The field's raw declared type — shared by [`Self::check_field_access_named`]
    /// (a normal read, which upgrades a `weak` result) and
    /// [`Self::check_assign_target_type`] (an assignment target, which
    /// must not: the new value needs checking against the real `weak T`
    /// storage type, not the `Option<T>` reading it back out would give).
    pub(super) fn field_type_named(&mut self, base_ty: &Type, ident: &Ident) -> Type {
        // `strip_indirection` (not just `strip_unique`) so a field is
        // reachable through a `:&T`/`:&mut T` parameter the same way it
        // already is through `T`/`:T` (Stage 2, slice 2).
        let base_ty = base_ty.strip_indirection();
        if let Type::Struct(_, _) = base_ty {
            if let Some(fields) = self.sigs.named_type_fields(base_ty) {
                if let Some((_, ty)) = fields.into_iter().find(|(n, _)| n == &ident.name) {
                    return ty.clone();
                }
            }
        }
        if base_ty.is_error() {
            return Type::Error;
        }
        let desc = self.describe(base_ty);
        self.err(
            ident.span,
            format!("`{desc}` has no field named `{}`", ident.name),
        );
        Type::Error
    }

    pub(super) fn check_tuple_index(&mut self, base_ty: &Type, idx: u32, span: Span) -> Type {
        let elems: Option<Vec<Type>> = match base_ty {
            Type::Tuple(elems) => Some(elems.clone()),
            Type::TupleStruct(_, _) => self.sigs.type_fields(base_ty),
            _ => None,
        };
        match elems.and_then(|e| e.get(idx as usize).cloned()) {
            Some(ty) => ty,
            None => {
                if !base_ty.is_error() {
                    let desc = self.describe(base_ty);
                    self.err(span, format!("`{desc}` has no field `.{idx}`"));
                }
                Type::Error
            }
        }
    }

    pub(super) fn check_index(&mut self, base: &Expr, index: &Expr) -> Type {
        let base_ty = self.check_expr(base);
        let index_ty = self.check_expr(index);
        if !matches!(&index_ty, Type::Primitive(p) if p.is_integer()) && !index_ty.is_error() {
            self.err(index.span, "array index must be an integer type");
        }
        match base_ty {
            Type::Array(inner) => *inner,
            Type::Error => Type::Error,
            other => {
                let desc = self.describe(&other);
                self.err(base.span, format!("`{desc}` cannot be indexed"));
                Type::Error
            }
        }
    }

    pub(super) fn check_if(
        &mut self,
        cond: &Expr,
        then_branch: &Block,
        else_branch: &Option<Box<Expr>>,
        expected: Option<&Type>,
    ) -> Type {
        let cond_ty = self.check_expr(cond);
        self.require_bool(&cond_ty, cond.span, "`if` condition");
        // `then`/`else` are mutually exclusive at runtime, so each is
        // move-checked from the *same* pre-branch state
        // (`Checker::moved_snapshot`), then merged back
        // (`Checker::merge_branches`) — a value moved on only one live-
        // reaching arm counts as moved after the `if`.
        let (then_ty, then_moved) =
            self.moved_snapshot(|this| this.check_block_with_expected(then_branch, expected));
        let then_diverges = matches!(then_ty, Type::Never);
        match else_branch {
            Some(e) => {
                let else_expected = expected.or(Some(&then_ty));
                let (else_ty, else_moved) =
                    self.moved_snapshot(|this| this.check_expr_with_expected(e, else_expected));
                let else_diverges = matches!(else_ty, Type::Never);
                self.merge_branches(vec![
                    (then_diverges, then_moved),
                    (else_diverges, else_moved),
                ]);
                if !then_ty.compatible(&else_ty) {
                    let then_s = self.describe(&then_ty);
                    let else_s = self.describe(&else_ty);
                    self.err(e.span, format!("`if`/`else` branches have incompatible types: `{then_s}` vs `{else_s}`"));
                }
                let result = if matches!(then_ty, Type::Never) {
                    else_ty
                } else {
                    prefer_concrete_type(then_ty, else_ty)
                };
                if let Some(tail) = &then_branch.tail {
                    if self
                        .expr_types
                        .get(&tail.id)
                        .is_some_and(|ty| ty.compatible(&result) && ty.contains_error())
                    {
                        self.expr_types.insert(tail.id, result.clone());
                    }
                }
                result
            }
            None => {
                // A missing `else` is exactly "the condition was false" —
                // an implicit branch that moves nothing.
                self.merge_branches(vec![(then_diverges, then_moved)]);
                Type::unit()
            }
        }
    }

    pub(super) fn check_match(
        &mut self,
        scrutinee: &Expr,
        arms: &[nether_ast::MatchArm],
        span: Span,
        expected: Option<&Type>,
    ) -> Type {
        let scrutinee_ty = self.check_expr(scrutinee);
        let mut result_ty: Option<Type> = None;
        let mut covered: HashSet<u32> = HashSet::new();
        let mut has_catch_all = false;
        // Every arm is move-checked from the same pre-`match` state and
        // merged back after the loop — mutually exclusive at runtime, same
        // reasoning as `check_if`'s `then`/`else`.
        let mut arm_moves = Vec::with_capacity(arms.len());
        for arm in arms {
            let (body_ty, moved) = self.moved_snapshot(|this| {
                this.check_pattern(
                    &arm.pattern,
                    &scrutinee_ty,
                    &mut covered,
                    &mut has_catch_all,
                );
                let arm_expected = expected.or(result_ty.as_ref());
                this.check_expr_with_expected(&arm.body, arm_expected)
            });
            arm_moves.push((matches!(body_ty, Type::Never), moved));
            result_ty = Some(match result_ty {
                None => body_ty,
                Some(prev) => {
                    if !prev.compatible(&body_ty) {
                        let prev_s = self.describe(&prev);
                        let body_s = self.describe(&body_ty);
                        self.err(
                            arm.body.span,
                            format!(
                                "`match` arms have incompatible types: `{prev_s}` vs `{body_s}`"
                            ),
                        );
                    }
                    if matches!(prev, Type::Never) {
                        body_ty
                    } else {
                        prefer_concrete_type(prev, body_ty)
                    }
                }
            });
        }
        self.merge_branches(arm_moves);
        if let Type::Enum(enum_id, _) = &scrutinee_ty {
            if !has_catch_all {
                if let Some(enum_sig) = self.sigs.enum_sigs.get(enum_id) {
                    let missing: Vec<&str> = enum_sig
                        .variants
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| !covered.contains(&(*i as u32)))
                        .map(|(_, (name, _))| name.as_str())
                        .collect();
                    if !missing.is_empty() {
                        self.err(
                            span,
                            format!(
                                "match is not exhaustive: missing variant(s) {}",
                                missing.join(", ")
                            ),
                        );
                    }
                }
            }
        }
        let result = result_ty.unwrap_or_else(Type::unit);
        // Context-free unit variants such as `Option.None` initially
        // carry unknown generic arguments. Once all arms determine the
        // match result, give those direct arm expressions the concrete
        // enum type so HIR/MIR/codegen use one consistent layout.
        for arm in arms {
            if let Some(arm_ty) = self.expr_types.get(&arm.body.id) {
                if arm_ty.compatible(&result)
                    && matches!(arm_ty, Type::Enum(_, args) if args.iter().any(Type::is_error))
                {
                    self.expr_types.insert(arm.body.id, result.clone());
                }
            }
        }
        result
    }

    pub(super) fn check_pattern(
        &mut self,
        pattern: &Pattern,
        scrutinee_ty: &Type,
        covered: &mut HashSet<u32>,
        has_catch_all: &mut bool,
    ) {
        self.check_pattern_inner(pattern, scrutinee_ty, covered, has_catch_all, true);
    }

    pub(super) fn check_pattern_inner(
        &mut self,
        pattern: &Pattern,
        scrutinee_ty: &Type,
        covered: &mut HashSet<u32>,
        has_catch_all: &mut bool,
        top_level: bool,
    ) {
        match pattern {
            Pattern::Wildcard(_) => {
                if top_level {
                    *has_catch_all = true;
                }
            }
            Pattern::Binding(id, _ident) => {
                if self.resolved.locals.contains_key(id) {
                    self.bind_local(*id, scrutinee_ty.clone(), false);
                    if top_level {
                        *has_catch_all = true;
                    }
                } else if let Some(res) = self.resolved.path_res.get(id) {
                    if let Resolution::EnumVariant(enum_id, idx) = res.base {
                        self.check_variant_pattern(
                            pattern.span(),
                            enum_id,
                            idx,
                            &[],
                            scrutinee_ty,
                            covered,
                            has_catch_all,
                            top_level,
                        );
                    }
                }
            }
            Pattern::Literal(literal, span) => {
                let compatible = match literal {
                    Literal::Int(_) => {
                        matches!(scrutinee_ty, Type::Primitive(kind) if kind.is_integer())
                    }
                    Literal::Float(_) => {
                        matches!(scrutinee_ty, Type::Primitive(kind) if kind.is_float())
                    }
                    Literal::Bool(_) => {
                        matches!(scrutinee_ty, Type::Primitive(PrimitiveKind::Bool))
                    }
                    Literal::Char(_) => {
                        matches!(scrutinee_ty, Type::Primitive(PrimitiveKind::Char))
                    }
                    Literal::Str(_) => matches!(scrutinee_ty, Type::String),
                };
                if !compatible && !scrutinee_ty.is_error() {
                    let found = self.describe(scrutinee_ty);
                    self.err(
                        *span,
                        format!("literal pattern is incompatible with `{found}`"),
                    );
                }
            }
            Pattern::Tuple(elems, span) => {
                let elem_tys: Vec<Type> = match scrutinee_ty {
                    Type::Tuple(tys) => {
                        if tys.len() != elems.len() {
                            self.err(
                                *span,
                                format!(
                                    "tuple pattern has {} element(s), but the scrutinee has {}",
                                    elems.len(),
                                    tys.len()
                                ),
                            );
                        }
                        tys.clone()
                    }
                    Type::Error => vec![Type::Error; elems.len()],
                    other => {
                        let found = self.describe(other);
                        self.err(
                            *span,
                            format!("tuple pattern requires a tuple, found `{found}`"),
                        );
                        vec![Type::Error; elems.len()]
                    }
                };
                for (index, pattern) in elems.iter().enumerate() {
                    let ty = elem_tys.get(index).cloned().unwrap_or(Type::Error);
                    self.check_pattern_inner(pattern, &ty, covered, has_catch_all, false);
                }
            }
            Pattern::Variant { path, payload, .. } => {
                if let Some(res) = self.resolved.path_res.get(&path.id) {
                    if let Resolution::EnumVariant(enum_id, idx) = res.base {
                        self.check_variant_pattern(
                            pattern.span(),
                            enum_id,
                            idx,
                            payload,
                            scrutinee_ty,
                            covered,
                            has_catch_all,
                            top_level,
                        );
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn check_variant_pattern(
        &mut self,
        span: Span,
        enum_id: DefId,
        variant: u32,
        payload: &[Pattern],
        scrutinee_ty: &Type,
        covered: &mut HashSet<u32>,
        has_catch_all: &mut bool,
        top_level: bool,
    ) {
        let same_enum =
            matches!(scrutinee_ty, Type::Enum(scrutinee_id, _) if *scrutinee_id == enum_id);
        if !same_enum {
            if !scrutinee_ty.is_error() {
                let pattern_enum = self.resolved.definitions.get(enum_id).name.to_string();
                let found = self.describe(scrutinee_ty);
                self.err(
                    span,
                    format!("variant pattern from enum `{pattern_enum}` cannot match `{found}`"),
                );
            }
        } else if top_level {
            covered.insert(variant);
        }

        let declared_arity = self
            .sigs
            .enum_sigs
            .get(&enum_id)
            .and_then(|sig| sig.variants.get(variant as usize))
            .map_or(0, |(_, fields)| fields.len());
        if payload.len() != declared_arity {
            self.err(
                span,
                format!(
                    "variant pattern expects {declared_arity} payload field(s), found {}",
                    payload.len()
                ),
            );
        }

        let payload_tys = if same_enum {
            self.sigs
                .enum_payload(scrutinee_ty, variant)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        for (index, pattern) in payload.iter().enumerate() {
            let ty = payload_tys.get(index).cloned().unwrap_or(Type::Error);
            self.check_pattern_inner(pattern, &ty, covered, has_catch_all, false);
        }
    }

    pub(super) fn check_for_in(&mut self, pattern: &Pattern, iter: &Expr, body: &Block) -> Type {
        let iter_ty = self.check_expr(iter);
        let elem_ty = match &iter_ty {
            Type::Array(inner) => (**inner).clone(),
            Type::Error => Type::Error,
            other => {
                let desc = self.describe(other);
                self.err(
                    iter.span,
                    format!("`for`-`in` requires an `Array`, found `{desc}`"),
                );
                Type::Error
            }
        };
        let mut covered = HashSet::new();
        let mut has_catch_all = false;
        self.check_pattern(pattern, &elem_ty, &mut covered, &mut has_catch_all);
        self.loop_depth += 1;
        self.check_loop_body_with_fixpoint(|this| {
            this.check_block(body);
        });
        self.loop_depth -= 1;
        Type::unit()
    }

    pub(super) fn check_return(&mut self, value: &Option<Box<Expr>>, span: Span) -> Type {
        let expected_ret = self.return_ty.clone();
        let actual = match value {
            Some(v) => self.check_expr_with_expected(v, Some(&expected_ret)),
            None => Type::unit(),
        };
        if !actual.compatible(&expected_ret) {
            let expected_s = self.describe(&expected_ret);
            let found_s = self.describe(&actual);
            self.err(
                span,
                format!("expected return type `{expected_s}`, found `{found_s}`"),
            );
        } else if matches!(expected_ret, Type::Ref(_) | Type::MutRef(_)) {
            self.check_returned_reference_origin(value.as_deref(), span);
        }
        Type::Never
    }

    fn check_returned_reference_origin(&mut self, value: Option<&Expr>, span: Span) {
        let Some(value) = value else {
            return;
        };
        let origins = self.reference_origins_of_expr(value);
        if origins.is_empty() {
            self.err(
                span,
                "cannot infer the origin of this returned reference; return a reference parameter directly",
            );
            return;
        }
        if origins
            .iter()
            .any(|origin| matches!(origin, BorrowOrigin::Local(_)))
        {
            self.err(
                span,
                "cannot return a reference borrowed from a local owned value",
            );
            return;
        }
        if origins.len() > 1 {
            self.err(
                span,
                "returned reference has ambiguous origins from multiple parameters",
            );
        }
    }

    fn reference_origins_of_expr(&self, expr: &Expr) -> HashSet<BorrowOrigin> {
        match &expr.kind {
            ExprKind::Path(_) => self
                .bare_local_of(expr)
                .and_then(|local| self.reference_origins.get(&local).copied())
                .into_iter()
                .collect(),
            ExprKind::If {
                then_branch,
                else_branch,
                ..
            } => {
                let mut origins = self.reference_origins_of_block_tail(then_branch);
                if let Some(else_branch) = else_branch {
                    origins.extend(self.reference_origins_of_expr(else_branch));
                }
                origins
            }
            ExprKind::Match { arms, .. } => arms
                .iter()
                .flat_map(|arm| self.reference_origins_of_expr(&arm.body))
                .collect(),
            ExprKind::Block(block) => self.reference_origins_of_block_tail(block),
            _ => HashSet::new(),
        }
    }

    fn reference_origins_of_block_tail(&self, block: &Block) -> HashSet<BorrowOrigin> {
        block
            .tail
            .as_deref()
            .map(|tail| self.reference_origins_of_expr(tail))
            .unwrap_or_default()
    }

    pub(super) fn check_closure(
        &mut self,
        params: &[nether_ast::Param],
        body: &Expr,
        expected: Option<&Type>,
    ) -> Type {
        let param_tys: Vec<Type> = params
            .iter()
            .map(|p| lower_type_expr(&p.ty, self.resolved, self.decls, self.diagnostics))
            .collect();
        for (p, t) in params.iter().zip(param_tys.iter()) {
            self.bind_local(p.id, t.clone(), p.mutable);
        }
        let expected_ret = match expected {
            Some(Type::Function(expected_params, expected_ret))
                if expected_params.len() == param_tys.len()
                    && expected_params
                        .iter()
                        .zip(&param_tys)
                        .all(|(expected, actual)| actual.compatible(expected)) =>
            {
                Some(expected_ret.as_ref())
            }
            _ => None,
        };
        let outer_loop_depth = std::mem::replace(&mut self.loop_depth, 0);
        let ret_ty = self.check_expr_with_expected(body, expected_ret);
        self.loop_depth = outer_loop_depth;
        Type::Function(param_tys, Box::new(ret_ty))
    }
}
