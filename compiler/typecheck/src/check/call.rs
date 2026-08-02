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
                    return self.check_method_call_on(
                        &current_ty,
                        seg,
                        generic_args,
                        args,
                        seg.span,
                        call_id,
                        receiver_mutable,
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
            DefKind::Primitive => {
                self.err(span, format!("`{name}` is a type, not a value"));
                Type::Error
            }
            DefKind::Interface => {
                self.err(
                    span,
                    format!("`{name}` is an interface and has no value form"),
                );
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
}
