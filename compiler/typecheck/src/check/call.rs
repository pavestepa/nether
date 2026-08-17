use super::*;

impl Checker<'_> {
    pub(super) fn check_call(
        &mut self,
        call_id: NodeId,
        callee: &Expr,
        generic_args: &[Type],
        args: &[Expr],
        expected: Option<&Type>,
    ) -> Type {
        if let ExprKind::Path(path) = &callee.kind {
            let ty = self.check_value_path(path, Some(args), expected, Some(call_id), generic_args);
            self.expr_types.insert(callee.id, ty.clone());
            ty
        } else {
            if !generic_args.is_empty() {
                self.err(
                    callee.span,
                    "explicit generic arguments require a named function or method",
                );
            }
            let callee_ty = self.check_expr(callee);
            self.check_call_value(&callee_ty, args, callee.span)
        }
    }

    pub(super) fn check_call_value(&mut self, ty: &Type, args: &[Expr], span: Span) -> Type {
        match ty {
            Type::Function(params, ret) => {
                if args.len() != params.len() {
                    self.err(
                        span,
                        format!(
                            "expected {} argument(s), found {}",
                            params.len(),
                            args.len()
                        ),
                    );
                }
                for (p, a) in params.iter().zip(args.iter()) {
                    let actual = self.check_expr_with_expected(a, Some(p));
                    if !actual.compatible(p) {
                        let expected = self.describe(p);
                        let found = self.describe(&actual);
                        self.err(a.span, format!("expected `{expected}`, found `{found}`"));
                    }
                }
                (**ret).clone()
            }
            Type::Error => Type::Error,
            other => {
                let desc = self.describe(other);
                self.err(span, format!("`{desc}` is not callable"));
                Type::Error
            }
        }
    }

    /// The heart of language-spec §10's disambiguation: `path.segments`
    /// beyond what `resolver` already consumed are walked here as a
    /// field/method access chain now that types are known — see
    /// `nether_resolver::PathResolution` docs.
    pub(super) fn check_value_path(
        &mut self,
        path: &Path,
        call_args: Option<&[Expr]>,
        expected: Option<&Type>,
        call_id: Option<NodeId>,
        generic_args: &[Type],
    ) -> Type {
        let Some(res) = self.resolved.path_res.get(&path.id).cloned() else {
            return Type::Error;
        };
        let total = path.segments.len();
        let direct_call_args = if res.consumed == total {
            call_args
        } else {
            None
        };

        let mut current_ty = match res.base {
            Resolution::Local(id) => {
                let ty = self
                    .locals
                    .get(&id)
                    .map(|(t, _)| t.clone())
                    .unwrap_or(Type::Error);
                // Whole-value read (nothing left in the path after this
                // local) vs. read-through (a field/method segment
                // follows) — see `Checker::check_move`'s docs. A trailing
                // *consuming* method call overrides the read-through
                // verdict separately, below, once the chosen overload's
                // `self_param` is known.
                self.check_move(id, &ty, path.span, res.consumed == total);
                if res.consumed == total {
                    if let Some(args) = call_args {
                        if !generic_args.is_empty() {
                            self.err(
                                path.span,
                                "explicit generic arguments cannot be applied to a function value",
                            );
                        }
                        return self.check_call_value(&ty, args, path.span);
                    }
                }
                self.upgrade_weak(ty)
            }
            Resolution::Def(id) => self.resolve_def_value(
                id,
                path.span,
                direct_call_args,
                expected,
                call_id,
                generic_args,
            ),
            Resolution::EnumVariant(enum_id, idx) => {
                if !generic_args.is_empty() {
                    self.err(
                        path.span,
                        "enum variants do not accept function generic arguments",
                    );
                }
                self.enum_variant_value_type(enum_id, idx, path.span, direct_call_args, expected)
            }
            Resolution::StaticMember(owner_id, idx) => self.static_member_call_or_value(
                owner_id,
                idx,
                path.span,
                direct_call_args,
                call_id,
                generic_args,
            ),
            Resolution::GenericParam | Resolution::Error => Type::Error,
        };

        for i in res.consumed..total {
            let seg = &path.segments[i];
            let is_last = i + 1 == total;
            if is_last {
                if let Some(args) = call_args {
                    let receiver_mutable = match res.base {
                        Resolution::Local(id) => self.locals.get(&id).map(|(_, m)| *m),
                        _ => None,
                    };
                    // Only a bare local *directly* followed by this one
                    // trailing call segment is a move candidate — e.g.
                    // `dog.greet()`, not `holder.dog.greet()` (there,
                    // `holder.dog`'s field read already went through the
                    // read-through check above, and its own type is
                    // never `Type::Unique` per language-spec §4.2, so
                    // there's nothing further to consume here).
                    let receiver_local = match res.base {
                        Resolution::Local(id) if total - res.consumed == 1 => Some(id),
                        _ => None,
                    };
                    return self.check_method_call_on(
                        &current_ty,
                        seg,
                        generic_args,
                        args,
                        seg.span,
                        call_id,
                        receiver_mutable,
                        receiver_local,
                    );
                }
            }
            current_ty = self.check_field_access_named(&current_ty, seg);
        }
        current_ty
    }

    pub(super) fn resolve_def_value(
        &mut self,
        id: DefId,
        span: Span,
        call_args: Option<&[Expr]>,
        expected: Option<&Type>,
        call_id: Option<NodeId>,
        generic_args: &[Type],
    ) -> Type {
        let def = self.resolved.definitions.get(id);
        let name = def.name.clone();
        match def.kind {
            DefKind::Primitive | DefKind::TypeAlias => {
                self.err(span, format!("`{name}` is a type, not a value"));
                Type::Error
            }
            DefKind::Trait => {
                self.err(span, format!("`{name}` is a trait and has no value form"));
                Type::Error
            }
            DefKind::Imported => Type::Error,
            DefKind::Enum => {
                self.err(
                    span,
                    format!("`{name}` is an enum type, not a value — use one of its variants"),
                );
                Type::Error
            }
            DefKind::Fn => {
                self.resolve_fn_value(id, &name, span, call_args, expected, call_id, generic_args)
            }
            DefKind::Type => {
                if !generic_args.is_empty() {
                    self.err(
                        span,
                        "type constructors do not accept function generic arguments",
                    );
                }
                self.resolve_type_value(id, &name, span, call_args, expected)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn resolve_fn_value(
        &mut self,
        id: DefId,
        name: &Symbol,
        span: Span,
        call_args: Option<&[Expr]>,
        expected: Option<&Type>,
        call_id: Option<NodeId>,
        generic_args: &[Type],
    ) -> Type {
        if name.as_str() == "println" || name.as_str() == "print" {
            if !generic_args.is_empty() {
                self.err(span, format!("builtin function `{name}` is not generic"));
            }
            return match call_args {
                Some(args) => {
                    for a in args {
                        let ty = self.check_expr(a);
                        self.require_into_string(&ty, a.span);
                    }
                    Type::unit()
                }
                None => {
                    self.err(span, format!("`{name}` must be called"));
                    Type::Error
                }
            };
        }
        if name.as_str() == "to" {
            return self.check_to_conversion(span, call_args, expected, generic_args);
        }
        match self.sigs.fns.get(&id).cloned() {
            Some(sig) => match call_args {
                Some(args) => {
                    let subst = self.check_call_args(
                        &sig,
                        args,
                        span,
                        expected,
                        None,
                        generic_args,
                        call_id,
                    );
                    substitute_generic(&sig.ret, &subst)
                }
                None if sig.generics.is_empty() => Type::Function(
                    sig.params.iter().map(|p| p.ty.clone()).collect(),
                    Box::new(sig.ret),
                ),
                None => {
                    self.err(
                        span,
                        format!(
                            "generic function `{name}` cannot be used as a value because its type arguments cannot be inferred"
                        ),
                    );
                    Type::Error
                }
            },
            None => Type::Error,
        }
    }

    /// `to(value)` — the universal ownership-domain conversion
    /// (language-spec §9). Only the target-inferred-from-context form is
    /// implemented (Stage 2, slice 4); `to<T>(value)`'s explicit form is a
    /// smaller, deferred follow-up.
    ///
    /// Three of the four transitions (`:T -> T`, `:t -> t`, `t -> :t`) are
    /// pure type-system relabeling — `:T`/`T` already share identical
    /// runtime representation (§3.2), and an inline value is an
    /// independent bit-copy already, so promoting it to `:t` aliases
    /// nothing. `T -> :T` is different: the source may have other live
    /// ARC aliases, so relabeling it `:T` without an actual deep copy
    /// would produce a "uniquely owned" value that isn't — that direction
    /// stays rejected until `Clone` exists (spec §10, Stage 3).
    fn check_to_conversion(
        &mut self,
        span: Span,
        call_args: Option<&[Expr]>,
        expected: Option<&Type>,
        generic_args: &[Type],
    ) -> Type {
        if !generic_args.is_empty() {
            self.err(
                span,
                "`to<T>(value)`'s explicit target form is not yet implemented — write `to(value)` and let the target be inferred from context",
            );
        }
        let Some(args) = call_args else {
            self.err(span, "`to` must be called");
            return Type::Error;
        };
        if args.len() != 1 {
            self.err(
                span,
                format!("`to` takes exactly 1 argument, found {}", args.len()),
            );
            for a in args {
                self.check_expr(a);
            }
            return Type::Error;
        }
        let Some(target) = expected.cloned() else {
            self.err(
                span,
                "cannot infer the target type of `to(value)`; add a type annotation",
            );
            self.check_expr(&args[0]);
            return Type::Error;
        };
        let arg_ty = self.check_expr_with_expected(&args[0], Some(&target));
        match (&arg_ty, &target) {
            (Type::Unique(source_inner), _) if !matches!(target, Type::Unique(_)) => {
                if !source_inner.compatible(&target) {
                    let expected_s = self.describe(&target);
                    let found_s = self.describe(source_inner);
                    self.err(
                        args[0].span,
                        format!("expected `{expected_s}`, found `{found_s}`"),
                    );
                }
            }
            (_, Type::Unique(target_inner)) if !matches!(arg_ty, Type::Unique(_)) => {
                if !arg_ty.compatible(target_inner) {
                    let expected_s = self.describe(target_inner);
                    let found_s = self.describe(&arg_ty);
                    self.err(
                        args[0].span,
                        format!("expected `{expected_s}`, found `{found_s}`"),
                    );
                } else if crate::alloc::alloc_kind(&arg_ty, &self.resolved.definitions)
                    == crate::alloc::AllocKind::Heap
                {
                    self.err(
                        args[0].span,
                        format!(
                            "`to()` cannot convert `{}` to its owned form yet — this direction requires `Clone`, not yet implemented (language-spec §10, Stage 3)",
                            self.describe(&arg_ty)
                        ),
                    );
                }
            }
            _ if arg_ty.is_error() || target.is_error() => {}
            _ => {
                self.err(
                    args[0].span,
                    "`to(value)` must convert between the owned and ordinary form of the same type — one side must be `:T`/`:t` and the other its plain counterpart",
                );
            }
        }
        target
    }
}
