use super::*;

impl Checker<'_> {
    /// `owned` is `true` for `:Dog { ... }` (language-spec §3, §11) — the
    /// literal's checked type is `Type::Unique(Type::Struct(..))` rather
    /// than the bare `Type::Struct(..)` an ordinary `Dog { ... }` literal
    /// produces. `expected` is unwrapped through the matching `Unique`
    /// layer first so generic inference against an owned expected type
    /// (e.g. `let d: Dog = :Dog { .. };`) still reaches the inner
    /// `Type::Struct` comparison below exactly as the unowned path does.
    pub(super) fn check_struct_lit(
        &mut self,
        path: &Path,
        fields: &[(Ident, Expr)],
        owned: bool,
        expected: Option<&Type>,
    ) -> Type {
        let Some(res) = self.resolved.path_res.get(&path.id).cloned() else {
            return Type::Error;
        };
        let Resolution::Def(id) = res.base else {
            if !matches!(res.base, Resolution::Error) {
                self.err(path.span, "struct literal path does not name a type");
            }
            return Type::Error;
        };
        let def = self.resolved.definitions.get(id);
        if def.kind != DefKind::Type {
            self.err(path.span, format!("`{}` is not a struct type", def.name));
            return Type::Error;
        }
        let name = def.name.clone();
        let Some(TypeShape::Struct(decl_fields)) = self.sigs.type_shapes.get(&id).cloned() else {
            self.err(
                path.span,
                format!("`{name}` is not a struct with named fields"),
            );
            return Type::Error;
        };
        let generic_names = self
            .sigs
            .type_generics
            .get(&id)
            .cloned()
            .unwrap_or_default();
        let mut subst: HashMap<Symbol, Type> = HashMap::new();
        let inner_expected = match (owned, expected) {
            (true, Some(Type::Unique(inner))) => Some(inner.as_ref()),
            (false, expected) => expected,
            (true, _) => None,
        };
        if let Some(Type::Struct(expected_id, args)) = inner_expected {
            if *expected_id == id && args.len() == generic_names.len() {
                subst.extend(generic_names.iter().cloned().zip(args.iter().cloned()));
            }
        }
        let mut seen = HashSet::new();
        for (fname, value) in fields {
            seen.insert(fname.name.clone());
            if let Some((_, declared)) = decl_fields.iter().find(|(n, _)| n == &fname.name) {
                let crosses_module = def.file.is_some_and(|file| file != fname.span.file);
                let is_public = self
                    .sigs
                    .field_visibility
                    .get(&(id, fname.name.clone()))
                    .is_some_and(|visibility| visibility.is_public());
                if crosses_module && !is_public {
                    self.err(fname.span, format!("field `{}` is private", fname.name));
                }
                let concrete_expected = substitute_generic(declared, &subst);
                let actual = self.check_expr_with_expected(value, Some(&concrete_expected));
                collect_generic_bindings(declared, &actual, &mut subst);
                let concrete_expected = substitute_generic(declared, &subst);
                if !actual.compatible(&concrete_expected) {
                    let expected_s = self.describe(&concrete_expected);
                    let found_s = self.describe(&actual);
                    self.err(
                        value.span,
                        format!(
                            "expected `{expected_s}`, found `{found_s}` for field `{}`",
                            fname.name
                        ),
                    );
                }
            } else {
                self.err(
                    fname.span,
                    format!("`{name}` has no field named `{}`", fname.name),
                );
                self.check_expr(value);
            }
        }
        for (field_name, _) in &decl_fields {
            if !seen.contains(field_name) {
                self.err(
                    path.span,
                    format!("missing field `{field_name}` in struct literal for `{name}`"),
                );
            }
        }
        let struct_ty = Type::Struct(
            id,
            generic_names
                .iter()
                .map(|name| subst.get(name).cloned().unwrap_or(Type::Error))
                .collect(),
        );
        if owned {
            Type::Unique(Box::new(struct_ty))
        } else {
            struct_ty
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn check_call_args(
        &mut self,
        sig: &FnSig,
        args: &[Expr],
        call_span: Span,
        expected_return: Option<&Type>,
        initial_subst: Option<HashMap<Symbol, Type>>,
        explicit_generic_args: &[Type],
        call_id: Option<NodeId>,
    ) -> HashMap<Symbol, Type> {
        // A trailing variadic parameter (`args: ...String`) collects zero
        // or more trailing call-site arguments — checked separately below
        // against its *element* type, since `sig.params.len()` no longer
        // says how many arguments the call needs.
        let variadic = sig.params.last().is_some_and(|p| p.variadic);
        let fixed_params = if variadic {
            &sig.params[..sig.params.len() - 1]
        } else {
            sig.params.as_slice()
        };
        if variadic {
            if args.len() < fixed_params.len() {
                self.err(
                    call_span,
                    format!(
                        "expected at least {} argument(s), found {}",
                        fixed_params.len(),
                        args.len()
                    ),
                );
            }
        } else if args.len() != sig.params.len() {
            self.err(
                call_span,
                format!(
                    "expected {} argument(s), found {}",
                    sig.params.len(),
                    args.len()
                ),
            );
        }
        let mut subst = initial_subst.unwrap_or_default();
        if !explicit_generic_args.is_empty() {
            let remaining: Vec<Symbol> = sig
                .generics
                .iter()
                .map(|(name, _)| name)
                .filter(|name| **name != crate::sig::variadic_len_param())
                .filter(|name| !subst.contains_key(*name))
                .cloned()
                .collect();
            if explicit_generic_args.len() != remaining.len() {
                self.err(
                    call_span,
                    format!(
                        "expected {} explicit generic argument(s), found {}",
                        remaining.len(),
                        explicit_generic_args.len()
                    ),
                );
            }
            subst.extend(
                remaining
                    .into_iter()
                    .zip(explicit_generic_args.iter().cloned()),
            );
        }
        if let Some(expected_return) = expected_return {
            collect_generic_bindings(&sig.ret, expected_return, &mut subst);
        }
        if variadic {
            subst.insert(
                crate::sig::variadic_len_param(),
                Type::Const(args.len().saturating_sub(fixed_params.len()) as u128),
            );
        }
        let fixed_arg_count = args.len().min(fixed_params.len());
        // Tracks, within *this* call's argument list only, whether each
        // borrowed local has already been borrowed mutably — the entire
        // exclusivity story this slice can honestly enforce, since
        // there's still no way for a `:&T`/`:&mut T` value to persist past
        // the call that produced it (Stage 2, slice 2).
        let mut borrowed_in_call: HashMap<LocalId, bool> = HashMap::new();
        for (param, arg) in fixed_params.iter().zip(&args[..fixed_arg_count]) {
            let expected = self
                .sigs
                .normalize_associated(&substitute_generic(&param.ty, &subst));
            if matches!(expected, Type::Ref(_) | Type::MutRef(_)) {
                if matches!(arg.kind, ExprKind::MutArg(_)) {
                    self.err(
                        arg.span,
                        "`mut` is not valid for a `:&T`/`:&mut T` argument — the parameter's own type already says whether it borrows mutably",
                    );
                }
                let actual = self.check_borrow_arg(arg, &expected, &mut borrowed_in_call);
                collect_generic_bindings(&param.ty, &actual, &mut subst);
                continue;
            }
            let (inner_expr, is_mut_arg) = match &arg.kind {
                ExprKind::MutArg(inner) => (inner.as_ref(), true),
                _ => (arg, false),
            };
            if param.mutable != is_mut_arg {
                if param.mutable {
                    self.err(arg.span, format!("parameter `{}` is `mut`; the caller must also write `mut` at the call site", param.name));
                } else {
                    self.err(
                        arg.span,
                        "`mut` is only valid for arguments passed to a `mut` parameter",
                    );
                }
            } else if param.mutable {
                self.check_mut_arg_target(inner_expr);
            }
            let expected = self
                .sigs
                .normalize_associated(&substitute_generic(&param.ty, &subst));
            let actual = self.check_expr_with_expected(inner_expr, Some(&expected));
            if is_mut_arg {
                self.expr_types.insert(arg.id, actual.clone());
            }
            collect_generic_bindings(&param.ty, &actual, &mut subst);
            let expected = self
                .sigs
                .normalize_associated(&substitute_generic(&param.ty, &subst));
            if !actual.compatible(&expected) {
                let expected_s = self.describe(&expected);
                let found_s = self.describe(&actual);
                self.err(
                    inner_expr.span,
                    format!("expected `{expected_s}`, found `{found_s}`"),
                );
            }
        }
        if variadic {
            let elem_ty = &sig.params[sig.params.len() - 1].ty;
            for arg in &args[fixed_arg_count..] {
                if matches!(arg.kind, ExprKind::MutArg(_)) {
                    self.err(arg.span, "`mut` is not valid for a variadic argument");
                    continue;
                }
                let expected = substitute_generic(elem_ty, &subst);
                let actual = self.check_expr_with_expected(arg, Some(&expected));
                if matches!(expected, Type::String) {
                    // Mirrors template-string interpolation: any
                    // `Into<String>` value is accepted here, not just a
                    // literal `String` — `nether_hir::into_string_expr`
                    // performs the matching conversion when lowering this
                    // same call.
                    self.require_into_string(&actual, arg.span);
                } else {
                    collect_generic_bindings(elem_ty, &actual, &mut subst);
                    let expected = substitute_generic(elem_ty, &subst);
                    if !actual.compatible(&expected) {
                        let expected_s = self.describe(&expected);
                        let found_s = self.describe(&actual);
                        self.err(
                            arg.span,
                            format!("expected `{expected_s}`, found `{found_s}`"),
                        );
                    }
                }
            }
        }
        for (name, bound) in &sig.generics {
            let Some(concrete) = subst.get(name) else {
                self.err(
                    call_span,
                    format!("cannot infer generic parameter `{name}` from this call's arguments"),
                );
                continue;
            };
            if sig.const_params.contains_key(name) {
                if !matches!(concrete, Type::Const(_)) {
                    self.err(
                        call_span,
                        format!("const generic parameter `{name}` requires an integer constant"),
                    );
                }
            } else if matches!(concrete, Type::Const(_)) {
                self.err(
                    call_span,
                    format!("type generic parameter `{name}` requires a type, not a constant"),
                );
            }
            for bound in bound {
                let concrete_bound = GenericBound {
                    trait_id: bound.trait_id,
                    args: bound
                        .args
                        .iter()
                        .map(|arg| substitute_generic(arg, &subst))
                        .collect(),
                };
                if !self.type_satisfies_bound(concrete, &concrete_bound) {
                    let concrete_s = self.describe(concrete);
                    let iface_name = self.describe_bound(&concrete_bound);
                    self.err(call_span, format!("`{concrete_s}` does not implement `{iface_name}`, required by generic parameter `{name}`"));
                }
            }
        }
        if let Some(call_id) = call_id {
            if !sig.generics.is_empty() {
                self.call_generic_args.insert(
                    call_id,
                    sig.generics
                        .iter()
                        .map(|(name, _)| subst.get(name).cloned().unwrap_or(Type::Error))
                        .collect(),
                );
            }
        }
        subst
    }

    pub(super) fn validate_type_bounds(&mut self, ty: &Type, span: Span) {
        match ty {
            Type::Struct(id, args) | Type::TupleStruct(id, args) | Type::Enum(id, args) => {
                let names = self
                    .sigs
                    .type_generics
                    .get(id)
                    .cloned()
                    .or_else(|| self.sigs.enum_sigs.get(id).map(|sig| sig.generics.clone()))
                    .unwrap_or_default();
                let bounds = self
                    .sigs
                    .generic_type_bounds
                    .get(id)
                    .cloned()
                    .unwrap_or_default();
                let subst: HashMap<Symbol, Type> =
                    names.iter().cloned().zip(args.iter().cloned()).collect();
                let const_params = self.sigs.const_type_params.get(id);
                for (name, argument) in names.iter().zip(args) {
                    if const_params.is_some_and(|params| params.contains_key(name)) {
                        if !matches!(argument, Type::Const(_)) {
                            self.err(
                                span,
                                format!(
                                    "const generic parameter `{name}` requires an integer constant"
                                ),
                            );
                        }
                    } else if matches!(argument, Type::Const(_)) {
                        self.err(
                            span,
                            format!(
                                "type generic parameter `{name}` requires a type, not a constant"
                            ),
                        );
                    }
                }
                for (index, bound) in bounds.into_iter().enumerate() {
                    let Some(actual) = args.get(index) else {
                        continue;
                    };
                    if actual.contains_error() {
                        continue;
                    }
                    for bound in bound {
                        let concrete_bound = GenericBound {
                            trait_id: bound.trait_id,
                            args: bound
                                .args
                                .iter()
                                .map(|arg| substitute_generic(arg, &subst))
                                .collect(),
                        };
                        if !self.type_satisfies_bound(actual, &concrete_bound) {
                            let actual = self.describe(actual);
                            let bound = self.describe_bound(&concrete_bound);
                            self.err(span, format!("`{actual}` does not implement `{bound}`, required by this generic type"));
                        }
                    }
                }
                for arg in args {
                    self.validate_type_bounds(arg, span);
                }
            }
            Type::Tuple(items) => {
                for item in items {
                    self.validate_type_bounds(item, span);
                }
            }
            Type::Array(inner) | Type::Weak(inner) => {
                self.validate_type_bounds(inner, span);
            }
            Type::Associated(owner, name) => {
                match owner.as_ref() {
                    Type::Generic(generic) => {
                        let declarations = self
                            .generics
                            .get(generic)
                            .into_iter()
                            .flatten()
                            .filter_map(|bound| {
                                self.sigs
                                    .trait_associated_types
                                    .get(&(bound.trait_id, name.clone()))
                            })
                            .collect::<Vec<_>>();
                        let count = declarations.len();
                        if count == 0 {
                            self.err(
                                span,
                                format!(
                                    "generic parameter `{generic}` has no associated type `{name}` in its bounds"
                                ),
                            );
                        } else if count > 1 {
                            self.err(
                                span,
                                format!(
                                    "associated type `{name}` is ambiguous across bounds of `{generic}`"
                                ),
                            );
                        } else if declarations[0].file != span.file
                            && !declarations[0].visibility.is_public()
                        {
                            self.err(span, format!("associated type `{name}` is private"));
                        }
                    }
                    Type::Struct(id, _) | Type::TupleStruct(id, _) | Type::Enum(id, _) => {
                        match self.sigs.associated_types.get(&(*id, name.clone())) {
                            None => self.err(span, format!("type has no associated type `{name}`")),
                            Some(signature)
                                if signature.file != span.file
                                    && !signature.visibility.is_public() =>
                            {
                                self.err(span, format!("associated type `{name}` is private"));
                            }
                            Some(_) => {}
                        }
                    }
                    _ => {}
                }
                self.validate_type_bounds(owner, span);
            }
            Type::Function(params, ret) => {
                for param in params {
                    self.validate_type_bounds(param, span);
                }
                self.validate_type_bounds(ret, span);
            }
            _ => {}
        }
    }

    pub(super) fn describe_bound(&self, bound: &GenericBound) -> String {
        let name = self
            .resolved
            .definitions
            .get(bound.trait_id)
            .name
            .to_string();
        if bound.args.is_empty() {
            name
        } else {
            format!(
                "{name}<{}>",
                bound
                    .args
                    .iter()
                    .map(|arg| self.describe(arg))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    }

    pub(super) fn is_into_string_bound(&self, bound: &GenericBound) -> bool {
        self.resolved.definitions.get(bound.trait_id).name.as_str() == "Into"
            && bound.args == [Type::String]
    }

    pub(super) fn type_satisfies_bound(&self, ty: &Type, bound: &GenericBound) -> bool {
        if self.is_into_string_bound(bound) {
            return self.is_into_string_convertible(ty);
        }
        match ty {
            Type::Struct(_, _) | Type::TupleStruct(_, _) | Type::Enum(_, _) => {
                self.sigs.satisfies(ty, bound)
            }
            Type::Generic(name) => self.generics.get(name).is_some_and(|actual| {
                actual
                    .iter()
                    .any(|actual| self.sigs.bound_satisfies(actual, bound))
            }),
            Type::Error => true,
            _ => false,
        }
    }

    pub(super) fn is_into_string_convertible(&self, ty: &Type) -> bool {
        match ty {
            Type::String | Type::Primitive(_) | Type::Error => true,
            Type::Unique(inner) => self.is_into_string_convertible(inner),
            Type::Struct(id, _) | Type::TupleStruct(id, _) | Type::Enum(id, _) => {
                let into = self.resolved.definitions.lookup(&Symbol::new("Into"));
                let bound = into.map(|trait_id| GenericBound {
                    trait_id,
                    args: vec![Type::String],
                });
                bound
                    .as_ref()
                    .is_some_and(|bound| self.sigs.satisfies(ty, bound))
                    && self
                        .sigs
                        .method(*id, &Symbol::new("into_string"), ReceiverDomain::Arc)
                        .is_some_and(|sig| sig.self_param.is_some() && sig.ret == Type::String)
            }
            Type::Generic(name) => self
                .generics
                .get(name)
                .is_some_and(|bounds| bounds.iter().any(|bound| self.is_into_string_bound(bound))),
            _ => false,
        }
    }

    pub(super) fn require_into_string(&mut self, ty: &Type, span: Span) {
        if !self.is_into_string_convertible(ty) {
            let description = self.describe(ty);
            self.err(
                span,
                format!(
                    "`{description}` cannot be converted to `String`; implement `Into<String>`"
                ),
            );
        }
    }

    pub(super) fn check_mut_arg_target(&mut self, expr: &Expr) {
        if let ExprKind::Path(path) = &expr.kind {
            if let Some(res) = self.resolved.path_res.get(&path.id) {
                if res.consumed == path.segments.len() {
                    if let Resolution::Local(id) = res.base {
                        match self.locals.get(&id) {
                            Some((_, true)) => return,
                            Some((_, false)) => {
                                self.err(expr.span, "cannot pass an immutable binding as a `mut` argument — declare it with `let mut`");
                                return;
                            }
                            None => {}
                        }
                    }
                }
            }
        }
        self.err(
            expr.span,
            "a `mut` argument must be a plain mutable local variable",
        );
    }

    /// Checks one call argument against a `Type::Ref`/`Type::MutRef`
    /// parameter (Stage 2, slice 2 — language-spec §8.1's `d: &Animal`/
    /// `e: &mut Animal` forms). There is no `&expr` operator anywhere in
    /// the grammar, so the *only* thing that can satisfy a reference
    /// parameter is a bare, already-owned (`Type::Unique`) local — the
    /// parameter's own declared type is what makes this a borrow, the
    /// same way an ordinary ARC parameter needs no caller-side marker
    /// either. `param_ty` is the already-substituted `Ref`/`MutRef`.
    /// Returns the argument's checked type for the caller's own
    /// `collect_generic_bindings` call, mirroring every other argument
    /// shape in `check_call_args`.
    pub(super) fn check_borrow_arg(
        &mut self,
        arg: &Expr,
        param_ty: &Type,
        borrowed: &mut HashMap<LocalId, bool>,
    ) -> Type {
        let is_mut = matches!(param_ty, Type::MutRef(_));
        let inner_expected = match param_ty {
            Type::Ref(inner) | Type::MutRef(inner) => inner.as_ref().clone(),
            _ => unreachable!("check_borrow_arg is only called for a Ref/MutRef param"),
        };
        let error = |this: &mut Self, arg: &Expr| {
            this.expr_types.insert(arg.id, Type::Error);
            Type::Error
        };
        let Some(id) = self.bare_local_of(arg) else {
            self.err(
                arg.span,
                "a `:&T`/`:&mut T` argument must be a plain local variable holding an owned (`:T`) value — there is no `&expr` operator to borrow anything else",
            );
            return error(self, arg);
        };
        let Some((local_ty, local_mutable)) = self.locals.get(&id).cloned() else {
            return error(self, arg);
        };
        if matches!(local_ty, Type::Ref(_) | Type::MutRef(_)) {
            let compatible = match (&local_ty, param_ty) {
                (Type::Ref(actual), Type::Ref(expected))
                | (Type::MutRef(actual), Type::Ref(expected))
                | (Type::MutRef(actual), Type::MutRef(expected)) => actual.compatible(expected),
                _ => false,
            };
            if !compatible {
                let expected = self.describe(param_ty);
                let found = self.describe(&local_ty);
                self.err(arg.span, format!("expected `{expected}`, found `{found}`"));
                return error(self, arg);
            }
            if is_mut && !local_mutable {
                self.err(arg.span, "cannot pass an immutable reference as `:&mut`");
            }
            self.expr_types.insert(arg.id, param_ty.clone());
            return param_ty.clone();
        }
        let Type::Unique(actual_inner) = &local_ty else {
            let desc = self.describe(&local_ty);
            self.err(
                arg.span,
                format!("expected an owned (`:{{Type}}`) value to borrow, found `{desc}`"),
            );
            return error(self, arg);
        };
        let actual_inner = actual_inner.as_ref().clone();
        if !actual_inner.compatible(&inner_expected) {
            let expected_s = self.describe(&inner_expected);
            let found_s = self.describe(&actual_inner);
            self.err(
                arg.span,
                format!("expected `{expected_s}`, found `{found_s}`"),
            );
        }
        if is_mut && !local_mutable {
            self.err(
                arg.span,
                "cannot borrow an immutable binding as `:&mut` — declare it with `let mut`",
            );
        }
        let (live_shared, live_mutable) =
            self.active_borrows.get(&id).copied().unwrap_or((0, None));
        if (is_mut && (live_shared > 0 || live_mutable.is_some()))
            || (!is_mut && live_mutable.is_some())
        {
            self.err(
                arg.span,
                "call borrow conflicts with an already-live stored borrow",
            );
        }
        let prev = borrowed.get(&id).copied();
        if let Some(prev_mut) = prev {
            if prev_mut || is_mut {
                self.err(
                    arg.span,
                    "cannot borrow a value as mutable more than once, or as both mutable and immutable, within the same call",
                );
            }
        }
        borrowed.insert(id, prev.unwrap_or(false) || is_mut);
        // A borrow reads the value, it doesn't consume it — same
        // treatment as a `: &self`/`: &mut self` receiver
        // (`nether_typecheck::check::method::check_method_call_on`).
        self.check_move(id, &local_ty, arg.span, false);
        let result_ty = if is_mut {
            Type::MutRef(Box::new(actual_inner))
        } else {
            Type::Ref(Box::new(actual_inner))
        };
        self.expr_types.insert(arg.id, result_ty.clone());
        result_ty
    }
}
