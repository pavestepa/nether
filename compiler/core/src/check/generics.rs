use super::*;
pub(super) mod substitute;
use substitute::Substitution;
#[derive(Clone, Default)]
pub(super) struct Monomorphization {
    pub result_contracts: ReturnContracts,
    pub consuming: Consuming,
    pub ownership_errors: Vec<Diagnostic>,
    pub entry_keys: BTreeMap<usize, String>,
    pub result_requests: BTreeMap<String, Type>,
    pub pending_results: std::collections::BTreeSet<usize>,
    pub bodies: BTreeMap<usize, a::Function>,
    pub entries: BTreeMap<String, Signature>,
    pub templates: BTreeMap<String, a::Function>,
    pub pending: Vec<(Signature, a::Function)>,
    pub next_id: usize,
    pub auditing: bool,
    pub audits: Vec<(Signature, a::Function)>,
    pub audit_keys: std::collections::BTreeSet<String>,
    pub instances: BTreeMap<String, Signature>,
    pub class_destructor_payloads: std::collections::BTreeSet<String>,
    pub destructor_types: std::collections::BTreeSet<String>,
}
impl Checker<'_> {
    pub(super) fn generic_call(
        &mut self,
        name: &str,
        types: &[a::TypeArgument],
        arguments: &[a::Expr],
        expected: Option<&Type>,
        span: Span,
    ) -> Result<h::Expr> {
        let template = self.mono.borrow().templates[name].clone();
        if template.parameters.len() != arguments.len() {
            return Err(error(span, "E0313", "wrong number of arguments"));
        }
        if types.len() > template.generics.len() {
            return Err(error(span, "E0370", "too many generic arguments"));
        }
        let mut substitutions = Substitution::default();
        let mut type_names = std::collections::BTreeSet::new();
        let mut const_names = std::collections::BTreeSet::new();
        for generic in &template.generics {
            match &generic.kind {
                a::GenericKind::Type { .. } => {
                    if !type_names.insert(generic.name.clone())
                        || const_names.contains(&generic.name)
                    {
                        return Err(error(generic.span, "E0301", "duplicate generic parameter"));
                    }
                }
                a::GenericKind::Const { .. } => {
                    if !const_names.insert(generic.name.clone())
                        || type_names.contains(&generic.name)
                    {
                        return Err(error(generic.span, "E0301", "duplicate generic parameter"));
                    }
                }
            }
        }
        for (generic, value) in template.generics.iter().zip(types) {
            match (&generic.kind, value) {
                (a::GenericKind::Type { .. }, a::TypeArgument::Type(ty)) => {
                    substitutions.types.insert(
                        generic.name.clone(),
                        resolve_type_with(ty, self.constants, self.nominals)?,
                    );
                }
                (a::GenericKind::Const { .. }, a::TypeArgument::Const(value)) => {
                    let Value::Integer(value) =
                        constant_value(value, &Type::Integer(I::Usize), self.constants)?
                    else {
                        unreachable!()
                    };
                    substitutions
                        .constants
                        .insert(generic.name.clone(), value.bits() as u64);
                }
                (
                    a::GenericKind::Const { .. },
                    a::TypeArgument::Type(a::Type {
                        kind: a::TypeKind::Named { path, arguments },
                        ..
                    }),
                ) if path.len() == 1 && arguments.is_empty() => {
                    let value = a::Expr {
                        kind: a::ExprKind::Name(path[0].clone()),
                        span,
                    };
                    let Value::Integer(value) =
                        constant_value(&value, &Type::Integer(I::Usize), self.constants)?
                    else {
                        unreachable!()
                    };
                    substitutions
                        .constants
                        .insert(generic.name.clone(), value.bits() as u64);
                }
                _ => return Err(error(span, "E0370", "generic argument kind mismatch")),
            }
        }
        if let Some(expected) = expected {
            infer(
                &template.result,
                &expected.logical(),
                &type_names,
                &const_names,
                &mut substitutions,
                self.nominals,
                span,
            )?;
        }
        // Use nonliteral argument information before defaulting integer literals.
        for (parameter, arg) in template.parameters.iter().zip(arguments) {
            if let Some(ty) = self.known_type(arg) {
                infer(
                    parameter.ty.as_ref().unwrap(),
                    &ty.logical(),
                    &type_names,
                    &const_names,
                    &mut substitutions,
                    self.nominals,
                    span,
                )?;
            }
        }
        let mut checked = Vec::new();
        for (parameter, arg) in template.parameters.iter().zip(arguments) {
            let source = parameter.ty.as_ref().unwrap();
            let mut ty = source.clone();
            substitutions.ty(&mut ty);
            let hint = resolve_type_with(&ty, self.constants, self.nominals).ok();
            let arg = self.expr(arg, hint.as_ref(), true)?;
            infer(
                source,
                &arg.ty.logical(),
                &type_names,
                &const_names,
                &mut substitutions,
                self.nominals,
                span,
            )?;
            checked.push(arg);
        }
        for generic in &template.generics {
            match &generic.kind {
                a::GenericKind::Type {
                    default: Some(ty), ..
                } if !substitutions.types.contains_key(&generic.name) => {
                    let mut ty = ty.clone();
                    substitutions.ty(&mut ty);
                    if let Ok(ty) = resolve_type_with(&ty, self.constants, self.nominals) {
                        substitutions.types.insert(generic.name.clone(), ty);
                    }
                }
                a::GenericKind::Const {
                    default: Some(value),
                    ..
                } if !substitutions.constants.contains_key(&generic.name) => {
                    let mut value = value.clone();
                    substitutions.expr(&mut value);
                    if let Ok(Value::Integer(value)) =
                        constant_value(&value, &Type::Integer(I::Usize), self.constants)
                    {
                        substitutions
                            .constants
                            .insert(generic.name.clone(), value.bits() as u64);
                    }
                }
                _ => (),
            }
        }
        if type_names
            .iter()
            .any(|n| !substitutions.types.contains_key(n))
            || const_names
                .iter()
                .any(|n| !substitutions.constants.contains_key(n))
        {
            return Err(error(
                span,
                "E0371",
                "generic arguments cannot be inferred; supply them explicitly",
            ));
        }
        for ty in substitutions.types.values() {
            validate_complexity(ty, span)?;
        }
        for generic in &template.generics {
            if let a::GenericKind::Type { constraints, .. } = &generic.kind {
                for constraint in constraints {
                    self.nominals
                        .interfaces
                        .require(
                            &substitutions.types[&generic.name],
                            constraint,
                            &substitutions,
                            self.constants,
                            self.nominals,
                        )
                        .map_err(|mut diagnostic| {
                            diagnostic.span = span;
                            diagnostic
                                .with_note(constraint.span, "required by this generic constraint")
                        })?;
                }
            }
        }
        if !self.mono.borrow().auditing {
            let audit_key = format!("{name}:{:?}", substitutions.constants);
            if self.mono.borrow_mut().audit_keys.insert(audit_key.clone()) {
                let mut abstract_substitution = Substitution::default();
                abstract_substitution.constants = substitutions.constants.clone();
                for generic in &template.generics {
                    if let a::GenericKind::Type { constraints, .. } = &generic.kind {
                        let marker = self.nominals.marker(
                            format!("<generic {audit_key}:{}>", generic.name),
                            constraints.iter().any(interfaces::copy),
                        );
                        abstract_substitution
                            .types
                            .insert(generic.name.clone(), marker);
                    }
                }
                self.nominals.interfaces.prepare_markers(
                    &template.generics,
                    &abstract_substitution,
                    self.constants,
                    self.nominals,
                )?;
                for marker in abstract_substitution.types.values() {
                    for method in self.nominals.interfaces.marker_methods(
                        marker,
                        self.constants,
                        self.nominals,
                    )? {
                        self.mono
                            .borrow_mut()
                            .templates
                            .insert(method.name.clone(), method);
                    }
                }
                let mut declaration = template.clone();
                abstract_substitution.function(&mut declaration);
                declaration.generics.clear();
                let parameters = declaration
                    .parameters
                    .iter()
                    .map(|p| {
                        resolve_type_with(p.ty.as_ref().unwrap(), self.constants, self.nominals)
                    })
                    .collect::<Result<Vec<_>>>()?;
                let result = resolve_type_with(&declaration.result, self.constants, self.nominals)?;
                // Audit generic bodies through the owned entry of consuming
                // parameters: the checks must hold for an owner caller.
                let mut audit_modes = methods::parameter_modes(&declaration, &parameters);
                for (index, mode) in audit_modes.iter_mut().enumerate() {
                    if self.consuming_parameter(&declaration, index) {
                        *mode = None;
                    }
                }
                self.mono.borrow_mut().audits.push((
                    Signature {
                        modes: audit_modes,
                        id: 0,
                        parameters,
                        result,
                    },
                    declaration,
                ));
            }
        }
        let key = format!(
            "{name}:{:?}:{:?}",
            substitutions.types, substitutions.constants
        );
        let mut mono = self.mono.borrow_mut();
        let signature = if let Some(signature) = mono.instances.get(&key) {
            signature.clone()
        } else {
            if mono.instances.len() >= 256 {
                return Err(error(
                    span,
                    "E0372",
                    "specialization limit exceeded; possible infinitely growing generic recursion",
                ));
            }
            let mut function = template;
            substitutions.function(&mut function);
            function.generics.clear();
            let parameters = function
                .parameters
                .iter()
                .map(|p| resolve_type_with(p.ty.as_ref().unwrap(), self.constants, self.nominals))
                .collect::<Result<Vec<_>>>()?;
            let result = resolve_type_with(&function.result, self.constants, self.nominals)?;
            let id = mono.next_id;
            mono.next_id += 1;
            function.name = format!("{name}$g{id}");
            let signature = Signature {
                modes: consume::parameter_modes(&function, &parameters, &mono.consuming),
                id,
                parameters,
                result,
            };
            mono.instances.insert(key, signature.clone());
            mono.bodies.insert(id, function.clone());
            if !mono.auditing || function.body.is_some() {
                mono.pending.push((signature.clone(), function));
            }
            signature
        };
        for (arg, ty) in checked.iter().zip(&signature.parameters) {
            same(ty, &arg.ty, arg.span)?;
        }
        if let Some(expected) = expected {
            same(expected, &signature.result, span)?;
        }
        drop(mono);
        self.call_entry(signature, arguments, checked, span)
    }
}
fn infer(
    pattern: &a::Type,
    actual: &Type,
    type_names: &std::collections::BTreeSet<String>,
    const_names: &std::collections::BTreeSet<String>,
    output: &mut Substitution,
    nominals: &records::Nominals,
    span: Span,
) -> Result<()> {
    match (&pattern.kind, actual) {
        (
            a::TypeKind::Pointer { mutable, pointee },
            Type::Pointer {
                mutable: actual_mut,
                pointee: actual_pointee,
            },
        ) => {
            if mutable != actual_mut {
                return Err(error(span, "E0371", "pointer access mismatch"));
            }
            infer(
                pointee,
                actual_pointee,
                type_names,
                const_names,
                output,
                nominals,
                span,
            )?;
        }
        (
            a::TypeKind::Function {
                unsafe_,
                parameters,
                result,
            },
            Type::Function {
                unsafe_: actual_unsafe,
                parameters: actual_parameters,
                result: actual_result,
                ..
            },
        ) => {
            if unsafe_ != actual_unsafe || parameters.len() != actual_parameters.len() {
                return Err(error(
                    span,
                    "E0371",
                    "function pointer parameter count mismatch",
                ));
            }
            for ((mutable, parameter), (actual_mutable, actual)) in
                parameters.iter().zip(actual_parameters)
            {
                if mutable != actual_mutable {
                    return Err(error(
                        span,
                        "E0371",
                        "function pointer parameter modes mismatch",
                    ));
                }
                infer(
                    parameter,
                    actual,
                    type_names,
                    const_names,
                    output,
                    nominals,
                    span,
                )?;
            }
            infer(
                result,
                actual_result,
                type_names,
                const_names,
                output,
                nominals,
                span,
            )?;
        }
        (a::TypeKind::Named { path, arguments }, ty)
            if path.len() == 1 && arguments.is_empty() && type_names.contains(&path[0]) =>
        {
            if let Some(old) = output.types.get(&path[0]) {
                same(old, ty, span)?;
            } else {
                output.types.insert(path[0].clone(), ty.clone());
            }
        }
        (
            a::TypeKind::Named { path, arguments },
            Type::Record { name, .. } | Type::Enum { name, .. } | Type::Class { name, .. },
        ) if path == &[nominals.origin(name)] => {
            let actuals = nominals.arguments(name);
            for (pattern, actual) in arguments.iter().zip(actuals) {
                match (pattern, actual) {
                    (a::TypeArgument::Type(pattern), records::Argument::Type(actual)) => infer(
                        pattern,
                        &actual,
                        type_names,
                        const_names,
                        output,
                        nominals,
                        span,
                    )?,
                    (pattern, records::Argument::Const(count)) => {
                        let name = match pattern {
                            a::TypeArgument::Const(a::Expr {
                                kind: a::ExprKind::Name(name),
                                ..
                            }) => Some(name),
                            a::TypeArgument::Type(a::Type {
                                kind: a::TypeKind::Named { path, arguments },
                                ..
                            }) if path.len() == 1 && arguments.is_empty() => Some(&path[0]),
                            _ => None,
                        };
                        if let Some(name) = name.filter(|n| const_names.contains(*n)) {
                            if output.constants.get(name).is_some_and(|old| *old != count) {
                                return Err(error(
                                    span,
                                    "E0371",
                                    "conflicting const generic arguments",
                                ));
                            }
                            output.constants.insert(name.clone(), count);
                        }
                    }
                    _ => (),
                }
            }
        }
        (a::TypeKind::Tuple(patterns), Type::Tuple(fields)) if patterns.len() == fields.len() => {
            for (pattern, actual) in patterns.iter().zip(fields) {
                infer(
                    pattern,
                    actual,
                    type_names,
                    const_names,
                    output,
                    nominals,
                    span,
                )?;
            }
        }
        (a::TypeKind::Array { element, length }, Type::Array(ty, count)) => {
            infer(element, ty, type_names, const_names, output, nominals, span)?;
            if let a::ExprKind::Name(name) = &length.kind {
                if const_names.contains(name) {
                    if output.constants.get(name).is_some_and(|old| old != count) {
                        return Err(error(span, "E0371", "conflicting const generic lengths"));
                    }
                    output.constants.insert(name.clone(), *count);
                }
            }
        }
        _ => (),
    }
    Ok(())
}

fn validate_complexity(ty: &Type, span: Span) -> Result<()> {
    let mut pending = vec![(ty, 0)];
    let mut count = 0;
    while let Some((ty, depth)) = pending.pop() {
        count += 1;
        if count > 4096 || depth > 32 {
            return Err(error(
                span,
                "E0372",
                "generic type complexity limit exceeded",
            ));
        }
        match ty {
            Type::Function {
                parameters, result, ..
            } => {
                pending.extend(parameters.iter().map(|(_, ty)| (ty, depth + 1)));
                pending.push((result, depth + 1));
            }
            Type::Tuple(fields) => pending.extend(fields.iter().map(|ty| (ty, depth + 1))),
            Type::Array(element, _)
            | Type::Pointer {
                pointee: element, ..
            } => pending.push((element, depth + 1)),
            Type::Record { fields, .. } => {
                pending.extend(fields.iter().map(|f| (&f.ty, depth + 1)))
            }
            Type::Enum { variants, .. } => pending.extend(
                variants
                    .iter()
                    .flat_map(|v| v.fields.iter())
                    .map(|ty| (ty, depth + 1)),
            ),
            _ => (),
        }
    }
    Ok(())
}
