use super::*;

/// Type-checks `module` using the names `resolver` already resolved.
///
/// See this crate's module docs for the full list of checks performed and
/// the documented simplifications (notably no `break`-value unification
/// for `loop` and intentionally local generic inference).
pub fn check(module: &Module, resolved: &ResolvedNames) -> (TypedTables, Vec<Diagnostic>) {
    let mut diagnostics = Vec::new();
    let decls = index_decls(module, resolved);

    let mut sigs = Signatures::default();
    build_type_shapes(&decls, resolved, &mut sigs, &mut diagnostics);
    build_enum_sigs(module, resolved, &decls, &mut sigs, &mut diagnostics);
    build_fn_sigs(module, resolved, &decls, &mut sigs, &mut diagnostics);
    infer_return_origin_summaries(module, resolved, &mut sigs);
    let trait_methods = build_trait_method_table(&decls, resolved, &mut sigs, &mut diagnostics);
    build_impl_methods(
        module,
        resolved,
        &decls,
        &trait_methods,
        &mut sigs,
        &mut diagnostics,
    );
    build_associated_constants(module, resolved, &decls, &mut sigs, &mut diagnostics);
    build_associated_types(module, resolved, &decls, &mut sigs, &mut diagnostics);
    infer_method_origin_summaries(module, resolved, &mut sigs);
    validate_finite_value_layouts(resolved, &decls, &sigs, &mut diagnostics);
    validate_alias_casing(module, resolved, &decls, &mut diagnostics);

    let mut expr_types = HashMap::new();
    let mut local_types = HashMap::new();
    let mut call_generic_args = HashMap::new();
    let mut existential_coercions = HashMap::new();
    for item in &module.items {
        match item {
            Item::Fn(f) => {
                if let Some(id) = resolved.definitions.lookup_in(f.span.file, &f.name.name) {
                    if let Some(sig) = sigs.fns.get(&id).cloned() {
                        if f.name.name.as_str() == "main"
                            && (!sig.params.is_empty()
                                || !sig.generics.is_empty()
                                || sig.ret != Type::unit())
                        {
                            diagnostics.push(
                                Diagnostic::error("`main` must have signature `fn main()`")
                                    .with_label(f.span, "invalid entry point"),
                            );
                            continue;
                        }
                        let mut checker = Checker::new(
                            resolved,
                            &sigs,
                            &decls,
                            &mut expr_types,
                            &mut local_types,
                            &mut call_generic_args,
                            &mut existential_coercions,
                            &mut diagnostics,
                        );
                        checker.check_fn_decl(f, &sig, None);
                    }
                }
            }
            Item::Impl(b) => {
                if let Some(owner) = resolved.definitions.lookup_in(b.span.file, &b.target.name) {
                    let self_ty = owner_as_type(owner, resolved, &decls);
                    for constant in &b.associated_consts {
                        let Some(signature) = sigs
                            .associated_consts
                            .get(&(owner, constant.name.name.clone()))
                            .cloned()
                        else {
                            continue;
                        };
                        if let Some(value) = &constant.value {
                            let mut checker = Checker::new(
                                resolved,
                                &sigs,
                                &decls,
                                &mut expr_types,
                                &mut local_types,
                                &mut call_generic_args,
                                &mut existential_coercions,
                                &mut diagnostics,
                            );
                            let actual =
                                checker.check_expr_with_expected(value, Some(&signature.ty));
                            if !signature.ty.compatible(&actual) {
                                checker.err(
                                    value.span,
                                    format!(
                                        "associated constant has type `{}`, expected `{}`",
                                        describe_type(&actual, resolved),
                                        describe_type(&signature.ty, resolved)
                                    ),
                                );
                            }
                        }
                    }
                    for m in &b.methods {
                        let domain = ReceiverDomain::of_self_param(m.self_param.as_ref());
                        if let Some(sig) = sigs.method(owner, &m.name.name, domain).cloned() {
                            let mut checker = Checker::new(
                                resolved,
                                &sigs,
                                &decls,
                                &mut expr_types,
                                &mut local_types,
                                &mut call_generic_args,
                                &mut existential_coercions,
                                &mut diagnostics,
                            );
                            checker.check_fn_decl(m, &sig, Some(self_ty.clone()));
                        }
                    }
                }
            }
            Item::Trait(i) => {
                if let Some(id) = resolved.definitions.lookup_in(i.span.file, &i.name.name) {
                    for constant in &i.associated_consts {
                        let Some(value) = &constant.value else {
                            continue;
                        };
                        let Some(signature) = sigs
                            .trait_associated_consts
                            .get(&(id, constant.name.name.clone()))
                            .cloned()
                        else {
                            continue;
                        };
                        let mut checker = Checker::new(
                            resolved,
                            &sigs,
                            &decls,
                            &mut expr_types,
                            &mut local_types,
                            &mut call_generic_args,
                            &mut existential_coercions,
                            &mut diagnostics,
                        );
                        let actual = checker.check_expr_with_expected(value, Some(&signature.ty));
                        if !signature.ty.compatible(&actual) {
                            checker.err(
                                value.span,
                                format!(
                                    "associated constant has type `{}`, expected `{}`",
                                    describe_type(&actual, resolved),
                                    describe_type(&signature.ty, resolved)
                                ),
                            );
                        }
                    }
                    for m in &i.methods {
                        if m.body.is_none() {
                            continue;
                        }
                        if let Some(method) = trait_methods
                            .get(&id)
                            .and_then(|ms| ms.get(&m.name.name))
                            .cloned()
                        {
                            let mut checking_sig = method.sig;
                            let trait_generics = i.generics.iter().map(|generic| {
                                (
                                    generic.name.name.clone(),
                                    generic
                                        .bounds
                                        .iter()
                                        .filter_map(|bound| {
                                            lower_generic_bound(
                                                bound,
                                                resolved,
                                                &decls,
                                                &mut diagnostics,
                                            )
                                        })
                                        .collect(),
                                )
                            });
                            checking_sig.generics.splice(0..0, trait_generics);
                            let mut checker = Checker::new(
                                resolved,
                                &sigs,
                                &decls,
                                &mut expr_types,
                                &mut local_types,
                                &mut call_generic_args,
                                &mut existential_coercions,
                                &mut diagnostics,
                            );
                            checker.check_fn_decl(m, &checking_sig, Some(Type::Trait(id)));
                        }
                    }
                }
            }
            Item::Struct(_) | Item::Enum(_) | Item::Use(_) | Item::Mod(_) | Item::TypeAlias(_) => {}
        }
    }

    (
        TypedTables {
            expr_types,
            local_types,
            call_generic_args,
            existential_coercions,
            signatures: sigs,
        },
        diagnostics,
    )
}

pub(super) fn describe_type(ty: &Type, resolved: &ResolvedNames) -> String {
    match ty {
        Type::Const(value) => value.to_string(),
        Type::Primitive(p) => format!("{p:?}").to_lowercase(),
        Type::Struct(id, args) | Type::TupleStruct(id, args) => {
            let name = resolved.definitions.get(*id).name.to_string();
            if args.is_empty() {
                name
            } else {
                let args = args
                    .iter()
                    .map(|arg| describe_type(arg, resolved))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{name}<{args}>")
            }
        }
        Type::Enum(id, args) => {
            let name = resolved.definitions.get(*id).name.to_string();
            if args.is_empty() {
                name
            } else {
                let args_str: Vec<String> =
                    args.iter().map(|a| describe_type(a, resolved)).collect();
                format!("{name}<{}>", args_str.join(", "))
            }
        }
        Type::Tuple(elems) => {
            format!(
                "({})",
                elems
                    .iter()
                    .map(|e| describe_type(e, resolved))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
        Type::Array(inner) => format!("[{}]", describe_type(inner, resolved)),
        Type::FixedArray(element, length) => format!(
            "{{{}, {}}}",
            describe_type(element, resolved),
            describe_type(length, resolved)
        ),
        Type::String => "String".to_string(),
        Type::Function(params, ret) => format!(
            "({}) => {}",
            params
                .iter()
                .map(|p| describe_type(p, resolved))
                .collect::<Vec<_>>()
                .join(", "),
            describe_type(ret, resolved)
        ),
        Type::Trait(id) => resolved.definitions.get(*id).name.to_string(),
        Type::Any(id, args) | Type::Some(id, args) => {
            let prefix = if matches!(ty, Type::Any(_, _)) {
                "any"
            } else {
                "some"
            };
            let name = resolved.definitions.get(*id).name.to_string();
            if args.is_empty() {
                format!("{prefix} {name}")
            } else {
                format!(
                    "{prefix} {name}<{}>",
                    args.iter()
                        .map(|arg| describe_type(arg, resolved))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        }
        Type::Task(output) => format!("Task<{}>", describe_type(output, resolved)),
        Type::Generic(name) => name.to_string(),
        Type::Associated(owner, name) => {
            format!("{}.{name}", describe_type(owner, resolved))
        }
        Type::Weak(inner) => format!("weak {}", describe_type(inner, resolved)),
        Type::Unique(inner) => format!(":{}", describe_type(inner, resolved)),
        Type::Ref(inner) => format!(":&{}", describe_type(inner, resolved)),
        Type::MutRef(inner) => format!(":&mut {}", describe_type(inner, resolved)),
        Type::Never => "!".to_string(),
        Type::Error => "<error>".to_string(),
    }
}

pub(crate) fn substitute_generic(ty: &Type, subst: &HashMap<Symbol, Type>) -> Type {
    match ty {
        Type::Generic(name) => subst.get(name).cloned().unwrap_or_else(|| ty.clone()),
        Type::Struct(id, args) => Type::Struct(
            *id,
            args.iter()
                .map(|arg| substitute_generic(arg, subst))
                .collect(),
        ),
        Type::TupleStruct(id, args) => Type::TupleStruct(
            *id,
            args.iter()
                .map(|arg| substitute_generic(arg, subst))
                .collect(),
        ),
        Type::Array(inner) => Type::Array(Box::new(substitute_generic(inner, subst))),
        Type::FixedArray(element, length) => Type::FixedArray(
            Box::new(substitute_generic(element, subst)),
            Box::new(substitute_generic(length, subst)),
        ),
        Type::Associated(owner, name) => {
            Type::Associated(Box::new(substitute_generic(owner, subst)), name.clone())
        }
        Type::Weak(inner) => Type::Weak(Box::new(substitute_generic(inner, subst))),
        Type::Unique(inner) => Type::Unique(Box::new(substitute_generic(inner, subst))),
        Type::Ref(inner) => Type::Ref(Box::new(substitute_generic(inner, subst))),
        Type::MutRef(inner) => Type::MutRef(Box::new(substitute_generic(inner, subst))),
        Type::Tuple(elems) => {
            Type::Tuple(elems.iter().map(|e| substitute_generic(e, subst)).collect())
        }
        Type::Enum(id, args) => Type::Enum(
            *id,
            args.iter().map(|a| substitute_generic(a, subst)).collect(),
        ),
        Type::Any(id, args) => Type::Any(
            *id,
            args.iter()
                .map(|arg| substitute_generic(arg, subst))
                .collect(),
        ),
        Type::Some(id, args) => Type::Some(
            *id,
            args.iter()
                .map(|arg| substitute_generic(arg, subst))
                .collect(),
        ),
        Type::Task(output) => Type::Task(Box::new(substitute_generic(output, subst))),
        Type::Function(params, ret) => Type::Function(
            params
                .iter()
                .map(|p| substitute_generic(p, subst))
                .collect(),
            Box::new(substitute_generic(ret, subst)),
        ),
        other => other.clone(),
    }
}

pub(super) fn collect_generic_bindings(
    declared: &Type,
    actual: &Type,
    subst: &mut HashMap<Symbol, Type>,
) {
    match (declared, actual) {
        (Type::Generic(name), actual) if !actual.contains_error() && !actual.contains_generic() => {
            if subst
                .get(name)
                .is_none_or(|existing| existing.contains_error() || existing.contains_generic())
            {
                subst.insert(name.clone(), actual.clone());
            }
        }
        (Type::Array(a), Type::Array(b))
        | (Type::Weak(a), Type::Weak(b))
        | (Type::Unique(a), Type::Unique(b))
        | (Type::Ref(a), Type::Ref(b))
        | (Type::MutRef(a), Type::MutRef(b)) => {
            collect_generic_bindings(a, b, subst);
        }
        (Type::FixedArray(a_element, a_length), Type::FixedArray(b_element, b_length)) => {
            collect_generic_bindings(a_element, b_element, subst);
            collect_generic_bindings(a_length, b_length, subst);
        }
        (Type::Tuple(a), Type::Tuple(b))
        | (Type::Enum(_, a), Type::Enum(_, b))
        | (Type::Struct(_, a), Type::Struct(_, b))
        | (Type::TupleStruct(_, a), Type::TupleStruct(_, b)) => {
            for (declared, actual) in a.iter().zip(b) {
                collect_generic_bindings(declared, actual, subst);
            }
        }
        (Type::Function(a_params, a_ret), Type::Function(b_params, b_ret)) => {
            for (declared, actual) in a_params.iter().zip(b_params) {
                collect_generic_bindings(declared, actual, subst);
            }
            collect_generic_bindings(a_ret, b_ret, subst);
        }
        _ => {}
    }
}

/// Refines unknown generic-enum components from an enclosing expected
/// type without changing an otherwise incompatible type.  `Type::Error`
/// is also the checker's poison type, so it is replaced only with a
/// concrete expectation; diagnostics already emitted for genuine errors
/// still prevent the pipeline from continuing.
pub(super) fn contextualize_unknowns(actual: &Type, expected: &Type) -> Type {
    match (actual, expected) {
        (Type::Error, expected) if !expected.contains_error() && !expected.contains_generic() => {
            expected.clone()
        }
        (Type::Tuple(actual), Type::Tuple(expected)) if actual.len() == expected.len() => {
            Type::Tuple(
                actual
                    .iter()
                    .zip(expected)
                    .map(|(actual, expected)| contextualize_unknowns(actual, expected))
                    .collect(),
            )
        }
        (Type::Enum(actual_id, actual), Type::Enum(expected_id, expected))
            if actual_id == expected_id && actual.len() == expected.len() =>
        {
            Type::Enum(
                *actual_id,
                actual
                    .iter()
                    .zip(expected)
                    .map(|(actual, expected)| contextualize_unknowns(actual, expected))
                    .collect(),
            )
        }
        (Type::Struct(actual_id, actual), Type::Struct(expected_id, expected))
            if actual_id == expected_id && actual.len() == expected.len() =>
        {
            Type::Struct(
                *actual_id,
                actual
                    .iter()
                    .zip(expected)
                    .map(|(actual, expected)| contextualize_unknowns(actual, expected))
                    .collect(),
            )
        }
        (Type::TupleStruct(actual_id, actual), Type::TupleStruct(expected_id, expected))
            if actual_id == expected_id && actual.len() == expected.len() =>
        {
            Type::TupleStruct(
                *actual_id,
                actual
                    .iter()
                    .zip(expected)
                    .map(|(actual, expected)| contextualize_unknowns(actual, expected))
                    .collect(),
            )
        }
        (Type::Array(actual), Type::Array(expected)) => {
            Type::Array(Box::new(contextualize_unknowns(actual, expected)))
        }
        (Type::Weak(actual), Type::Weak(expected)) => {
            Type::Weak(Box::new(contextualize_unknowns(actual, expected)))
        }
        (
            Type::Function(actual_params, actual_ret),
            Type::Function(expected_params, expected_ret),
        ) if actual_params.len() == expected_params.len() => Type::Function(
            actual_params
                .iter()
                .zip(expected_params)
                .map(|(actual, expected)| contextualize_unknowns(actual, expected))
                .collect(),
            Box::new(contextualize_unknowns(actual_ret, expected_ret)),
        ),
        _ => actual.clone(),
    }
}

pub(super) fn prefer_concrete_type(first: Type, second: Type) -> Type {
    if first.contains_error() && !second.contains_error() {
        second
    } else {
        first
    }
}

// ---------------------------------------------------------------------
// Expression / statement checker
// ---------------------------------------------------------------------
