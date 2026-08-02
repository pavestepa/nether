use super::*;

impl Checker<'_> {
    pub(super) fn check_struct_lit(
        &mut self,
        path: &Path,
        fields: &[(Ident, Expr)],
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
        if let Some(Type::Struct(expected_id, args)) = expected {
            if *expected_id == id && args.len() == generic_names.len() {
                subst.extend(generic_names.iter().cloned().zip(args.iter().cloned()));
            }
        }
        let mut seen = HashSet::new();
        for (fname, value) in fields {
            seen.insert(fname.name.clone());
            if let Some((_, declared)) = decl_fields.iter().find(|(n, _)| n == &fname.name) {
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
        Type::Struct(
            id,
            generic_names
                .iter()
                .map(|name| subst.get(name).cloned().unwrap_or(Type::Error))
                .collect(),
        )
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
        let fixed_arg_count = args.len().min(fixed_params.len());
        for (param, arg) in fixed_params.iter().zip(&args[..fixed_arg_count]) {
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
            let expected = substitute_generic(&param.ty, &subst);
            let actual = self.check_expr_with_expected(inner_expr, Some(&expected));
            if is_mut_arg {
                self.expr_types.insert(arg.id, actual.clone());
            }
            collect_generic_bindings(&param.ty, &actual, &mut subst);
            let expected = substitute_generic(&param.ty, &subst);
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
            let Some(bound) = bound else { continue };
            let concrete_bound = GenericBound {
                interface: bound.interface,
                args: bound
                    .args
                    .iter()
                    .map(|arg| substitute_generic(arg, &subst))
                    .collect(),
            };
            let satisfies = self.type_satisfies_bound(concrete, &concrete_bound);
            if !satisfies {
                let concrete_s = self.describe(concrete);
                let iface_name = self.describe_bound(&concrete_bound);
                self.err(call_span, format!("`{concrete_s}` does not implement `{iface_name}`, required by generic parameter `{name}`"));
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
                    names.into_iter().zip(args.iter().cloned()).collect();
                for (index, bound) in bounds.into_iter().enumerate() {
                    let Some(bound) = bound else { continue };
                    let Some(actual) = args.get(index) else {
                        continue;
                    };
                    if actual.contains_error() {
                        continue;
                    }
                    let concrete_bound = GenericBound {
                        interface: bound.interface,
                        args: bound
                            .args
                            .iter()
                            .map(|arg| substitute_generic(arg, &subst))
                            .collect(),
                    };
                    if !self.type_satisfies_bound(actual, &concrete_bound) {
                        let actual = self.describe(actual);
                        let bound = self.describe_bound(&concrete_bound);
                        self.err(
                            span,
                            format!(
                                "`{actual}` does not implement `{bound}`, required by this generic type"
                            ),
                        );
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
            .get(bound.interface)
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
        self.resolved.definitions.get(bound.interface).name.as_str() == "Into"
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
            Type::Generic(name) => self
                .generics
                .get(name)
                .and_then(Option::as_ref)
                .is_some_and(|actual| self.sigs.bound_satisfies(actual, bound)),
            Type::Error => true,
            _ => false,
        }
    }

    pub(super) fn is_into_string_convertible(&self, ty: &Type) -> bool {
        match ty {
            Type::String | Type::Primitive(_) | Type::Error => true,
            Type::Struct(id, _) | Type::TupleStruct(id, _) | Type::Enum(id, _) => {
                let into = self.resolved.definitions.lookup(&Symbol::new("Into"));
                let bound = into.map(|interface| GenericBound {
                    interface,
                    args: vec![Type::String],
                });
                bound
                    .as_ref()
                    .is_some_and(|bound| self.sigs.satisfies(ty, bound))
                    && self
                        .sigs
                        .method(*id, &Symbol::new("into_string"))
                        .is_some_and(|sig| sig.self_param.is_some() && sig.ret == Type::String)
            }
            Type::Generic(name) => self
                .generics
                .get(name)
                .and_then(Option::as_ref)
                .is_some_and(|bound| self.is_into_string_bound(bound)),
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
}
