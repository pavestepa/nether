use super::*;

impl Checker<'_> {
    pub(super) fn resolve_type_value(
        &mut self,
        id: DefId,
        name: &Symbol,
        span: Span,
        call_args: Option<&[Expr]>,
        expected: Option<&Type>,
    ) -> Type {
        let generic_names = self
            .sigs
            .type_generics
            .get(&id)
            .cloned()
            .unwrap_or_default();
        let mut subst: HashMap<Symbol, Type> = HashMap::new();
        match expected {
            Some(Type::Struct(expected_id, args)) | Some(Type::TupleStruct(expected_id, args))
                if *expected_id == id && args.len() == generic_names.len() =>
            {
                subst.extend(generic_names.iter().cloned().zip(args.iter().cloned()));
            }
            _ => {}
        }
        match (self.sigs.type_shapes.get(&id).cloned(), call_args) {
            (Some(TypeShape::Unit), None) => Type::Struct(
                id,
                generic_names
                    .iter()
                    .map(|name| subst.get(name).cloned().unwrap_or(Type::Error))
                    .collect(),
            ),
            (Some(TypeShape::TupleStruct(field_tys)), Some(args)) => {
                if args.len() != field_tys.len() {
                    self.err(
                        span,
                        format!(
                            "expected {} argument(s), found {}",
                            field_tys.len(),
                            args.len()
                        ),
                    );
                }
                for (a, declared) in args.iter().zip(field_tys.iter()) {
                    let concrete_expected = substitute_generic(declared, &subst);
                    let actual = self.check_expr_with_expected(a, Some(&concrete_expected));
                    collect_generic_bindings(declared, &actual, &mut subst);
                    let concrete_expected = substitute_generic(declared, &subst);
                    if !actual.compatible(&concrete_expected) {
                        let expected_s = self.describe(&concrete_expected);
                        let found_s = self.describe(&actual);
                        self.err(
                            a.span,
                            format!("expected `{expected_s}`, found `{found_s}`"),
                        );
                    }
                }
                Type::TupleStruct(
                    id,
                    generic_names
                        .iter()
                        .map(|name| subst.get(name).cloned().unwrap_or(Type::Error))
                        .collect(),
                )
            }
            _ => {
                self.err(
                    span,
                    format!(
                        "`{name}` cannot be used this way — construct it with `{{ .. }}` (struct) or `(..)` \
                         (tuple-struct) syntax matching its declaration"
                    ),
                );
                Type::Error
            }
        }
    }

    pub(super) fn enum_variant_value_type(
        &mut self,
        enum_id: DefId,
        idx: u32,
        span: Span,
        call_args: Option<&[Expr]>,
        expected: Option<&Type>,
    ) -> Type {
        let Some(sig) = self.sigs.enum_sigs.get(&enum_id) else {
            self.err(
                span,
                "internal type information for this enum is unavailable",
            );
            return Type::Error;
        };
        let Some((_, payload)) = sig.variants.get(idx as usize) else {
            self.err(span, "unknown enum variant");
            return Type::Error;
        };
        let payload = payload.clone();
        let generic_names = sig.generics.clone();
        let mut subst = HashMap::new();
        if let Some(Type::Enum(expected_id, args)) = expected {
            if *expected_id == enum_id && args.len() == generic_names.len() {
                subst.extend(generic_names.iter().cloned().zip(args.iter().cloned()));
            }
        }
        if !payload.is_empty() {
            match call_args {
                Some(args) => {
                    if args.len() != payload.len() {
                        self.err(
                            span,
                            format!(
                                "variant expects {} argument(s), found {}",
                                payload.len(),
                                args.len()
                            ),
                        );
                    }
                    for (a, declared) in args.iter().zip(payload.iter()) {
                        let concrete_expected = substitute_generic(declared, &subst);
                        let actual = self.check_expr_with_expected(a, Some(&concrete_expected));
                        collect_generic_bindings(declared, &actual, &mut subst);
                        let concrete_expected = substitute_generic(declared, &subst);
                        if !actual.compatible(&concrete_expected) {
                            let expected_s = self.describe(&concrete_expected);
                            let found_s = self.describe(&actual);
                            self.err(
                                a.span,
                                format!("expected `{expected_s}`, found `{found_s}`"),
                            );
                        }
                    }
                }
                None => self.err(span, "this variant requires payload arguments"),
            }
        }
        Type::Enum(
            enum_id,
            generic_names
                .iter()
                .map(|name| subst.get(name).cloned().unwrap_or(Type::Error))
                .collect(),
        )
    }

    pub(super) fn static_member_call_or_value(
        &mut self,
        owner_id: DefId,
        idx: u32,
        span: Span,
        call_args: Option<&[Expr]>,
        call_id: Option<NodeId>,
        generic_args: &[Type],
    ) -> Type {
        let Some(name) = self
            .resolved
            .definitions
            .get(owner_id)
            .methods
            .get(idx as usize)
            .cloned()
        else {
            return Type::Error;
        };
        // Any domain: this is a call through the *type* (`Dog.new(...)`),
        // which is only ever valid for a `Static` signature — but we still
        // want to find an `Arc`/`Owned` instance method here too, purely
        // to give the precise "instance method" diagnostic below instead
        // of a generic "no method" one.
        let Some(sig) = self.sigs.method_any_domain(owner_id, &name).cloned() else {
            return Type::Error;
        };
        if sig.file != span.file && !sig.visibility.is_public() {
            self.err(span, format!("method `{name}` is private"));
            return Type::Error;
        }
        if sig.self_param.is_some() {
            self.err(
                span,
                format!("`{name}` is an instance method and cannot be called as a static member"),
            );
            return Type::Error;
        }
        match call_args {
            Some(args) => {
                let subst =
                    self.check_call_args(&sig, args, span, None, None, generic_args, call_id);
                substitute_generic(&sig.ret, &subst)
            }
            None => Type::Function(
                sig.params.iter().map(|p| p.ty.clone()).collect(),
                Box::new(sig.ret),
            ),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn check_method_call_on(
        &mut self,
        base_ty: &Type,
        method: &Ident,
        generic_args: &[Type],
        args: &[Expr],
        span: Span,
        call_id: Option<NodeId>,
        receiver_mutable: Option<bool>,
        receiver_local: Option<LocalId>,
    ) -> Type {
        // Captured from the *un-stripped* type — an owned (`:Dog`) receiver
        // or a reference to one (`:&Dog`/`:&mut Dog`) must resolve against
        // the owner's `Owned`-domain method set, not its ARC-domain one
        // (language-spec §8.4, §3.1).
        let receiver_domain = ReceiverDomain::of_receiver_ty(base_ty);
        let unstripped_base_ty = base_ty.clone();
        // `strip_indirection`, not just `strip_unique` — a `:&T`/`:&mut T`
        // receiver's owner/field resolution below needs to see through
        // the reference the same way it already sees through `Unique`
        // (Stage 2, slice 3).
        let base_ty = base_ty.strip_indirection();
        if let Type::Generic(name) = base_ty {
            return self.check_generic_method_call(name, method, generic_args, args, span, call_id);
        }
        if let Type::Array(elem_ty) = base_ty {
            if matches!(method.name.as_str(), "len" | "push" | "pop") {
                if !generic_args.is_empty() {
                    self.err(method.span, "array methods are not generic");
                }
                if matches!(method.name.as_str(), "push" | "pop") && receiver_mutable != Some(true)
                {
                    self.err(method.span, "cannot call a `mut self` method through an immutable receiver — declare it with `let mut`");
                }
                return self.check_array_method_call(elem_ty, method, args, span);
            }
        }
        let owner_id = match base_ty {
            Type::Struct(id, _) | Type::TupleStruct(id, _) => Some(*id),
            Type::Enum(id, _) => Some(*id),
            Type::Array(_) => self.resolved.definitions.lookup(&Symbol::new("Array")),
            _ => None,
        };
        let Some(owner_id) = owner_id else {
            if !base_ty.is_error() {
                let desc = self.describe(base_ty);
                self.err(
                    method.span,
                    format!("`{desc}` has no method named `{}`", method.name),
                );
            }
            return Type::Error;
        };
        // Specialization-aware: an exact-match concrete override
        // (`impl Option<i32> { ... }`) wins over the generic fallback —
        // see `MethodSet::for_args`. `receiver_args` still containing an
        // unsubstituted `Type::Generic` (this call site is itself inside
        // another still-generic function) can never exactly match a
        // specialization, so it naturally resolves to the generic sig
        // here; `monomorphization` re-resolves the same way once the
        // enclosing function is instantiated for a concrete type and picks
        // up the override then (`docs/generics.md` § "Methods on generic
        // types").
        let receiver_args: &[Type] = match base_ty {
            Type::Struct(_, args) | Type::TupleStruct(_, args) | Type::Enum(_, args) => args,
            Type::Array(elem) => std::slice::from_ref(elem.as_ref()),
            _ => &[],
        };
        // Try the receiver's own domain first; fall back to `Static` so a
        // static method can still be called through a value
        // (`static_methods_can_be_called_through_values`) — `Static` can
        // never coexist with `Arc`/`Owned` for the same name, so at most
        // one of these two lookups ever succeeds.
        let method_set = self
            .sigs
            .methods
            .get(&(owner_id, method.name.clone(), receiver_domain))
            .or_else(|| {
                self.sigs
                    .methods
                    .get(&(owner_id, method.name.clone(), ReceiverDomain::Static))
            });
        let Some(method_set) = method_set else {
            let desc = self.describe(base_ty);
            self.err(
                method.span,
                format!("`{desc}` has no method named `{}`", method.name),
            );
            return Type::Error;
        };
        let is_specialized = method_set
            .specializations
            .iter()
            .any(|(args, _)| args.as_slice() == receiver_args);
        let Some(sig) = method_set.for_args(receiver_args).cloned() else {
            let desc = self.describe(base_ty);
            self.err(
                method.span,
                format!("`{desc}` has no method named `{}`", method.name),
            );
            return Type::Error;
        };
        if sig.file != method.span.file && !sig.visibility.is_public() {
            self.err(method.span, format!("method `{}` is private", method.name));
            return Type::Error;
        }
        // `ByMutRef` (`mut self`) and `OwnedMutRef` (`: &mut self`) both
        // need an exclusive/mutable receiver — `receiver_mutable` already
        // answers this correctly for both a `let mut`-bound owned local
        // and a `:&mut T` parameter, since a `Type::MutRef` parameter's
        // own binding already counts as mutable
        // (`Checker::check_fn_decl`, Stage 2 slice 2).
        if matches!(
            sig.self_param,
            Some(SelfParam::ByMutRef | SelfParam::OwnedMutRef)
        ) && receiver_mutable != Some(true)
        {
            self.err(method.span, "cannot call a `mut self` method (or `: &mut self`) through an immutable receiver — declare it with `let mut`");
        }
        // A `: self` (consuming) method can never be called through a mere
        // reference — the receiver's *unstripped* type must genuinely be
        // `Type::Unique`, not `Type::Ref`/`Type::MutRef` (Stage 2, slice
        // 3) — otherwise `ReceiverDomain::of_receiver_ty` folding
        // references into the same `Owned` bucket as owned values (so
        // `: &self`/`: &mut self` methods are reachable at all through a
        // `:&T`/`:&mut T` parameter) would unsoundly let this consume a
        // value the caller only lent out.
        if sig.self_param == Some(SelfParam::Owned)
            && matches!(unstripped_base_ty, Type::Ref(_) | Type::MutRef(_))
        {
            self.err(
                method.span,
                "cannot call a consuming (`: self`) method through a borrowed reference",
            );
        }
        // A `: self` receiver consumes the owned value it's called
        // through; `: &self`/`: &mut self` only borrow it (validity-check
        // only, same as a field read) — language-spec §8.4. `receiver_local`
        // is `None` whenever the receiver isn't literally a bare local
        // (e.g. `container.field.method()`), which this pass doesn't
        // track moves for at all (language-spec §4.2: a field is never
        // itself `Type::Unique`, so there is nothing to consume there).
        if let Some(id) = receiver_local {
            let consumes = sig.self_param == Some(SelfParam::Owned);
            self.check_move(id, &unstripped_base_ty, span, consumes);
        }
        // Build a placeholder `Owner<T, ...>` to structurally match against
        // `base_ty`'s concrete arguments below, binding each owner
        // parameter from the receiver. The names must be `sig`'s own — not
        // re-read from a declaration — since a builtin owner such as
        // `Option` has none; `sig.generics`' first `owner_arity` entries
        // are exactly the owner's parameters, in the receiver's positional
        // order, however this particular method's `impl` block happened to
        // name them (`build_impl_methods` always splices them in first). A
        // specialized `sig` has no owner placeholders at all — everything
        // in it is already concrete — so there is nothing to bind.
        let owner_arity = if is_specialized {
            0
        } else {
            receiver_args.len()
        };
        let owner_names = sig
            .generics
            .iter()
            .take(owner_arity)
            .map(|(name, _)| Type::Generic(name.clone()));
        let owner_pattern = match base_ty {
            Type::Struct(id, _) => Type::Struct(*id, owner_names.collect()),
            Type::TupleStruct(id, _) => Type::TupleStruct(*id, owner_names.collect()),
            Type::Enum(id, _) => Type::Enum(*id, owner_names.collect()),
            Type::Array(_) => Type::Array(Box::new(
                owner_names.into_iter().next().unwrap_or(Type::Error),
            )),
            _ => unreachable!("owner_id was only set for Struct/TupleStruct/Enum/Array above"),
        };
        let mut owner_subst = HashMap::new();
        collect_generic_bindings(&owner_pattern, base_ty, &mut owner_subst);
        let subst = self.check_call_args(
            &sig,
            args,
            span,
            None,
            Some(owner_subst),
            generic_args,
            call_id,
        );
        substitute_generic(&sig.ret, &subst)
    }

    pub(super) fn check_generic_method_call(
        &mut self,
        name: &Symbol,
        method: &Ident,
        generic_args: &[Type],
        args: &[Expr],
        span: Span,
        call_id: Option<NodeId>,
    ) -> Type {
        let bound = self.generics.get(name).cloned().flatten();
        if let Some(bound) = bound {
            let bound_name = self.resolved.definitions.get(bound.trait_id).name.as_str();
            if bound_name == "Into"
                && bound.args == [Type::String]
                && method.name.as_str() == "into_string"
            {
                if !generic_args.is_empty() {
                    self.err(method.span, "`into_string` is not generic");
                }
                if !args.is_empty() {
                    self.err(
                        span,
                        format!("expected 0 argument(s), found {}", args.len()),
                    );
                }
                return Type::String;
            }
            if let Some(raw_sig) = self
                .sigs
                .trait_methods
                .get(&(bound.trait_id, method.name.clone()))
                .cloned()
            {
                let trait_subst: HashMap<Symbol, Type> = self
                    .sigs
                    .trait_generics
                    .get(&bound.trait_id)
                    .into_iter()
                    .flatten()
                    .cloned()
                    .zip(bound.args)
                    .collect();
                let sig = specialize_fn_sig(&raw_sig, &trait_subst);
                let subst =
                    self.check_call_args(&sig, args, span, None, None, generic_args, call_id);
                return substitute_generic(&sig.ret, &subst);
            }
        }
        self.err(
            method.span,
            format!(
                "no method named `{}` found for generic type `{name}`",
                method.name
            ),
        );
        Type::Error
    }

    /// `Array<T>`'s runtime methods (language-spec §3.5: "Runtime methods:
    /// push, pop, len") — provided by `runtime/array`, not by any `impl`
    /// block a user could write, so there is no [`crate::sig::FnSig`] for
    /// them in [`Signatures::methods`] to look up; special-cased here the
    /// same way `println`/`print` are special-cased in
    /// [`Checker::resolve_fn_value`].
    pub(super) fn check_array_method_call(
        &mut self,
        elem_ty: &Type,
        method: &Ident,
        args: &[Expr],
        span: Span,
    ) -> Type {
        match method.name.as_str() {
            "len" => {
                if !args.is_empty() {
                    self.err(
                        span,
                        format!("expected 0 argument(s), found {}", args.len()),
                    );
                }
                Type::Primitive(PrimitiveKind::Usize)
            }
            "push" => {
                if args.len() != 1 {
                    self.err(
                        span,
                        format!("expected 1 argument(s), found {}", args.len()),
                    );
                }
                if let Some(arg) = args.first() {
                    let actual = self.check_expr_with_expected(arg, Some(elem_ty));
                    if !actual.compatible(elem_ty) {
                        let expected_s = self.describe(elem_ty);
                        let found_s = self.describe(&actual);
                        self.err(
                            arg.span,
                            format!("expected `{expected_s}`, found `{found_s}`"),
                        );
                    }
                }
                Type::unit()
            }
            "pop" => {
                if !args.is_empty() {
                    self.err(
                        span,
                        format!("expected 0 argument(s), found {}", args.len()),
                    );
                }
                match self.resolved.definitions.lookup(&Symbol::new("Option")) {
                    Some(option_id) => Type::Enum(option_id, vec![elem_ty.clone()]),
                    None => Type::Error,
                }
            }
            other => {
                self.err(
                    method.span,
                    format!("`Array` has no method named `{other}`"),
                );
                Type::Error
            }
        }
    }
}
