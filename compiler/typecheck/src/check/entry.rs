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
    let interface_methods =
        build_interface_method_table(&decls, resolved, &mut sigs, &mut diagnostics);
    build_impl_methods(
        module,
        resolved,
        &decls,
        &interface_methods,
        &mut sigs,
        &mut diagnostics,
    );
    validate_finite_value_layouts(resolved, &decls, &sigs, &mut diagnostics);

    let mut expr_types = HashMap::new();
    let mut local_types = HashMap::new();
    let mut call_generic_args = HashMap::new();
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
                            &mut diagnostics,
                        );
                        checker.check_fn_decl(f, &sig, None);
                    }
                }
            }
            Item::Impl(b) => {
                if let Some(owner) = resolved.definitions.lookup_in(b.span.file, &b.target.name) {
                    let self_ty = owner_as_type(owner, resolved, &decls);
                    for m in &b.methods {
                        if let Some(sig) = sigs.method(owner, &m.name.name).cloned() {
                            let mut checker = Checker::new(
                                resolved,
                                &sigs,
                                &decls,
                                &mut expr_types,
                                &mut local_types,
                                &mut call_generic_args,
                                &mut diagnostics,
                            );
                            checker.check_fn_decl(m, &sig, Some(self_ty.clone()));
                        }
                    }
                }
            }
            Item::Interface(i) => {
                if let Some(id) = resolved.definitions.lookup_in(i.span.file, &i.name.name) {
                    for m in &i.methods {
                        if m.body.is_none() {
                            continue;
                        }
                        if let Some(method) = interface_methods
                            .get(&id)
                            .and_then(|ms| ms.get(&m.name.name))
                            .cloned()
                        {
                            let mut checking_sig = method.sig;
                            let interface_generics = i.generics.iter().map(|generic| {
                                (
                                    generic.name.name.clone(),
                                    generic.bound.as_ref().and_then(|bound| {
                                        lower_generic_bound(
                                            bound,
                                            resolved,
                                            &decls,
                                            &mut diagnostics,
                                        )
                                    }),
                                )
                            });
                            checking_sig.generics.splice(0..0, interface_generics);
                            let mut checker = Checker::new(
                                resolved,
                                &sigs,
                                &decls,
                                &mut expr_types,
                                &mut local_types,
                                &mut call_generic_args,
                                &mut diagnostics,
                            );
                            checker.check_fn_decl(m, &checking_sig, Some(Type::Interface(id)));
                        }
                    }
                }
            }
            Item::Type(_) | Item::Enum(_) | Item::Use(_) | Item::Mod(_) => {}
        }
    }

    (
        TypedTables {
            expr_types,
            local_types,
            call_generic_args,
            signatures: sigs,
        },
        diagnostics,
    )
}

pub(super) fn describe_type(ty: &Type, resolved: &ResolvedNames) -> String {
    match ty {
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
        Type::Interface(id) => resolved.definitions.get(*id).name.to_string(),
        Type::Generic(name) => name.to_string(),
        Type::Weak(inner) => format!("weak {}", describe_type(inner, resolved)),
        Type::Never => "!".to_string(),
        Type::Error => "<error>".to_string(),
    }
}

pub(super) fn substitute_generic(ty: &Type, subst: &HashMap<Symbol, Type>) -> Type {
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
        Type::Weak(inner) => Type::Weak(Box::new(substitute_generic(inner, subst))),
        Type::Tuple(elems) => {
            Type::Tuple(elems.iter().map(|e| substitute_generic(e, subst)).collect())
        }
        Type::Enum(id, args) => Type::Enum(
            *id,
            args.iter().map(|a| substitute_generic(a, subst)).collect(),
        ),
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
        (Type::Array(a), Type::Array(b)) | (Type::Weak(a), Type::Weak(b)) => {
            collect_generic_bindings(a, b, subst);
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
