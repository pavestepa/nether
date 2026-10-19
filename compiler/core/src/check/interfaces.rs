//! Nominal interface obligations and abstract generic method surfaces. Concrete
//! calls use the existing entry specializer; no interface value or vtable is made.
use super::*;
use generics::substitute::Substitution;
use std::{cell::RefCell, collections::BTreeSet};
mod signatures;
use signatures::{compatible, signature};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Bound {
    name: String,
    arguments: Vec<records::Argument>,
}
#[derive(Default)]
pub(super) struct Registry {
    definitions: BTreeMap<String, a::Item>,
    methods: BTreeMap<String, (a::Function, bool)>,
    markers: RefCell<BTreeMap<String, Vec<Bound>>>,
    active: RefCell<BTreeSet<String>>,
}
pub(super) fn copy(ty: &a::Type) -> bool {
    matches!(&ty.kind,a::TypeKind::Named {path,arguments} if path==&["Copy".to_owned()] && arguments.is_empty())
}
impl Registry {
    pub(super) fn collect(module: &a::Module) -> Result<Self> {
        let mut result = Self::default();
        let mut names = BTreeSet::new();
        for item in &module.items {
            match &item.kind {
                a::ItemKind::Interface { name, methods, .. } => {
                    if !names.insert(name.clone())
                        || result
                            .definitions
                            .insert(name.clone(), item.clone())
                            .is_some()
                    {
                        return Err(error(
                            item.span,
                            "E0400",
                            "duplicate interface/type declaration",
                        ));
                    }
                    let mut seen = BTreeSet::new();
                    for method in methods {
                        if !seen.insert(&method.name) {
                            return Err(error(method.span, "E0400", "duplicate interface method"));
                        }
                        if method.body.is_some() {
                            return Err(error(
                                method.span,
                                "E0400",
                                "interface methods must be declarations",
                            ));
                        }
                    }
                }
                a::ItemKind::Aggregate { name, .. } | a::ItemKind::Enum { name, .. } => {
                    if !names.insert(name.clone()) {
                        return Err(error(
                            item.span,
                            "E0400",
                            "duplicate interface/type declaration",
                        ));
                    }
                }
                a::ItemKind::Function(f) => {
                    result
                        .methods
                        .insert(f.name.clone(), (f.clone(), item.private));
                }
                _ => {}
            }
        }
        Ok(result)
    }
    pub(super) fn contains(&self, name: &str) -> bool {
        self.definitions.contains_key(name)
    }
    pub(super) fn validate_declarations(
        &self,
        constants: &crate::constants::Constants,
        nominals: &records::Nominals,
    ) -> Result<()> {
        for (name, item) in &self.definitions {
            let a::ItemKind::Interface { generics, .. } = &item.kind else {
                unreachable!()
            };
            let mut substitution = Substitution::default();
            let mut seen = BTreeSet::new();
            for generic in generics {
                if !seen.insert(&generic.name) {
                    return Err(error(generic.span, "E0301", "duplicate generic parameter"));
                }
                match &generic.kind {
                    a::GenericKind::Type { constraints, .. } => {
                        substitution.types.insert(
                            generic.name.clone(),
                            nominals.marker(
                                format!("<interface {name}:{}>", generic.name),
                                constraints.iter().any(copy),
                            ),
                        );
                    }
                    a::GenericKind::Const { ty, .. } => {
                        if resolve_type_with(ty, constants, nominals)? != Type::Integer(I::Usize) {
                            return Err(error(
                                generic.span,
                                "E0370",
                                "const interface parameter must be usize",
                            ));
                        }
                        substitution.constants.insert(generic.name.clone(), 2);
                    }
                }
            }
            self.prepare_markers(generics, &substitution, constants, nominals)?;
            let receiver = nominals.marker(format!("<interface receiver {name}>"), false);
            let bound = Bound {
                name: name.clone(),
                arguments: generics
                    .iter()
                    .map(|g| match g.kind {
                        a::GenericKind::Type { .. } => {
                            records::Argument::Type(substitution.types[&g.name].clone())
                        }
                        a::GenericKind::Const { .. } => {
                            records::Argument::Const(substitution.constants[&g.name])
                        }
                    })
                    .collect(),
            };
            for method in self.surface(&bound, &receiver, constants, nominals)? {
                let mut seen = BTreeSet::new();
                for generic in &method.generics {
                    if !seen.insert(&generic.name) {
                        return Err(error(
                            generic.span,
                            "E0301",
                            "duplicate method generic parameter",
                        ));
                    }
                }
                compatible(&method, &method, constants, nominals)?;
            }
            // Defaults see preceding parameters, even when no instance is used.
            let mut prefix = Substitution::default();
            for generic in generics {
                match &generic.kind {
                    a::GenericKind::Type {
                        default: Some(ty),
                        constraints,
                    } => {
                        let mut ty = ty.clone();
                        prefix.ty(&mut ty);
                        let ty = resolve_type_with(&ty, constants, nominals)?;
                        let mut scope = substitution.clone();
                        scope.types.insert(generic.name.clone(), ty.clone());
                        for constraint in constraints {
                            self.require(&ty, constraint, &scope, constants, nominals)?;
                        }
                    }
                    a::GenericKind::Const {
                        default: Some(value),
                        ..
                    } => {
                        let mut value = value.clone();
                        prefix.expr(&mut value);
                        constant_value(&value, &Type::Integer(I::Usize), constants)?;
                    }
                    _ => {}
                }
                match generic.kind {
                    a::GenericKind::Type { .. } => {
                        prefix.types.insert(
                            generic.name.clone(),
                            substitution.types[&generic.name].clone(),
                        );
                    }
                    a::GenericKind::Const { .. } => {
                        prefix
                            .constants
                            .insert(generic.name.clone(), substitution.constants[&generic.name]);
                    }
                }
            }
        }
        Ok(())
    }
    fn instance(
        &self,
        source: &a::Type,
        outer: &Substitution,
        constants: &crate::constants::Constants,
        nominals: &records::Nominals,
    ) -> Result<(Bound, Substitution)> {
        let mut source = source.clone();
        outer.clone().ty(&mut source);
        let a::TypeKind::Named { path, arguments } = &source.kind else {
            return Err(error(
                source.span,
                "E0400",
                "constraint must name an interface or Copy",
            ));
        };
        let name = path.join("::");
        let Some(item) = self.definitions.get(&name) else {
            return Err(error(
                source.span,
                "E0400",
                format!("unknown interface '{name}'"),
            ));
        };
        let a::ItemKind::Interface { generics, .. } = &item.kind else {
            unreachable!()
        };
        if arguments.len() > generics.len() {
            return Err(error(source.span, "E0370", "too many interface arguments"));
        }
        let mut substitution = Substitution::default();
        let mut actual = Vec::new();
        let mut names = BTreeSet::new();
        for (index, generic) in generics.iter().enumerate() {
            if !names.insert(&generic.name) {
                return Err(error(generic.span, "E0301", "duplicate generic parameter"));
            }
            match &generic.kind {
                a::GenericKind::Type { default, .. } => {
                    let mut ty = match arguments.get(index) {
                        Some(a::TypeArgument::Type(ty)) => ty.clone(),
                        None => default.clone().ok_or_else(|| {
                            error(source.span, "E0371", "missing interface type argument")
                        })?,
                        _ => {
                            return Err(error(
                                source.span,
                                "E0370",
                                "expected interface type argument",
                            ))
                        }
                    };
                    substitution.ty(&mut ty);
                    let ty = resolve_type_with(&ty, constants, nominals)?;
                    actual.push(records::Argument::Type(ty.clone()));
                    substitution.types.insert(generic.name.clone(), ty);
                }
                a::GenericKind::Const { default, .. } => {
                    let mut value = match arguments.get(index) {
                        Some(a::TypeArgument::Const(value)) => value.clone(),
                        Some(a::TypeArgument::Type(a::Type {
                            kind: a::TypeKind::Named { path, arguments },
                            span,
                        })) if path.len() == 1 && arguments.is_empty() => a::Expr {
                            kind: a::ExprKind::Name(path[0].clone()),
                            span: *span,
                        },
                        None => default.clone().ok_or_else(|| {
                            error(source.span, "E0371", "missing interface const argument")
                        })?,
                        _ => {
                            return Err(error(
                                source.span,
                                "E0370",
                                "expected interface const argument",
                            ))
                        }
                    };
                    substitution.expr(&mut value);
                    let Value::Integer(value) =
                        constant_value(&value, &Type::Integer(I::Usize), constants)?
                    else {
                        unreachable!()
                    };
                    let value = value.bits() as u64;
                    actual.push(records::Argument::Const(value));
                    substitution.constants.insert(generic.name.clone(), value);
                }
            }
        }
        Ok((
            Bound {
                name,
                arguments: actual,
            },
            substitution,
        ))
    }
    pub(super) fn require(
        &self,
        ty: &Type,
        source: &a::Type,
        outer: &Substitution,
        constants: &crate::constants::Constants,
        nominals: &records::Nominals,
    ) -> Result<()> {
        if copy(source) {
            return if ty.copyable() {
                Ok(())
            } else {
                Err(error(
                    source.span,
                    "E0373",
                    "generic argument does not satisfy Copy",
                ))
            };
        }
        let (bound, substitution) = self.instance(source, outer, constants, nominals)?;
        let key = format!("{:?}:{bound:?}", ty.logical());
        if self.active.borrow().contains(&key) {
            return Ok(());
        }
        if self.active.borrow().len() >= 32 {
            return Err(error(
                source.span,
                "E0372",
                "interface obligation nesting limit exceeded",
            ));
        }
        self.active.borrow_mut().insert(key.clone());
        let result = (|| {
            let a::ItemKind::Interface { generics, .. } = &self.definitions[&bound.name].kind
            else {
                unreachable!()
            };
            for generic in generics {
                if let a::GenericKind::Type { constraints, .. } = &generic.kind {
                    for constraint in constraints {
                        self.require(
                            &substitution.types[&generic.name],
                            constraint,
                            &substitution,
                            constants,
                            nominals,
                        )?;
                    }
                }
            }
            let claims = match ty {
                Type::Record { name, .. } | Type::Class { name, .. } => {
                    if let Some(bounds) = self.markers.borrow().get(name) {
                        bounds.clone()
                    } else {
                        nominals.interface_claims(ty, constants)?
                    }
                }
                _ => Vec::new(),
            };
            if !claims.contains(&bound) {
                return Err(error(
                    source.span,
                    "E0402",
                    format!(
                        "type does not explicitly implement interface '{}'",
                        bound.name
                    ),
                ));
            }
            Ok(())
        })();
        self.active.borrow_mut().remove(&key);
        result
    }
    pub(super) fn claims(
        &self,
        sources: &[a::Type],
        substitution: &Substitution,
        constants: &crate::constants::Constants,
        nominals: &records::Nominals,
    ) -> Result<Vec<Bound>> {
        let mut result = Vec::new();
        for source in sources {
            let (bound, _) = self.instance(source, substitution, constants, nominals)?;
            if result.contains(&bound) {
                return Err(error(
                    source.span,
                    "E0401",
                    "duplicate interface implementation",
                ));
            }
            result.push(bound);
        }
        Ok(result)
    }
    fn surface(
        &self,
        bound: &Bound,
        receiver: &Type,
        constants: &crate::constants::Constants,
        nominals: &records::Nominals,
    ) -> Result<Vec<a::Function>> {
        let item = &self.definitions[&bound.name];
        let a::ItemKind::Interface {
            generics, methods, ..
        } = &item.kind
        else {
            unreachable!()
        };
        let mut substitution = Substitution::default();
        for (generic, arg) in generics.iter().zip(&bound.arguments) {
            match arg {
                records::Argument::Type(ty) => {
                    substitution.types.insert(generic.name.clone(), ty.clone());
                }
                records::Argument::Const(n) => {
                    substitution.constants.insert(generic.name.clone(), *n);
                }
            }
        }
        let mut result = Vec::new();
        for method in methods {
            let mut method = method.clone();
            // Method generics shadow interface generics of the same name.
            let mut scope = substitution.clone();
            for generic in &method.generics {
                scope.types.remove(&generic.name);
                scope.constants.remove(&generic.name);
            }
            scope.function(&mut method);
            apply_generics(&mut scope, &mut method.generics);
            for p in &mut method.parameters {
                if p.receiver {
                    p.ty = Some(generics::substitute::ast_type(receiver, p.span));
                }
            }
            // Resolve nongeneric signatures now; generic method signatures are
            // resolved against fresh markers during compatibility checking.
            if method.generics.is_empty() {
                signature(&method, constants, nominals)?;
            }
            result.push(method);
        }
        Ok(result)
    }
    pub(super) fn validate_implementation(
        &self,
        ty: &Type,
        sources: &[a::Type],
        outer: &Substitution,
        constants: &crate::constants::Constants,
        nominals: &records::Nominals,
    ) -> Result<()> {
        let (Type::Record { name, .. } | Type::Class { name, .. }) = ty else {
            unreachable!()
        };
        let owner = nominals.origin(name);
        for source in sources {
            let (bound, _) = self.instance(source, outer, constants, nominals)?;
            self.require(ty, source, outer, constants, nominals)?;
            for required in self.surface(&bound, &ty.logical(), constants, nominals)? {
                let candidate = format!("{owner}.{}", required.name);
                let (actual, private) = self.methods.get(&candidate).ok_or_else(|| {
                    error(
                        source.span,
                        "E0401",
                        format!("missing interface method '{}'", required.name),
                    )
                    .with_note(required.span, "required by this interface declaration")
                })?;
                if *private {
                    return Err(error(
                        actual.span,
                        "E0401",
                        "interface implementation method must be public",
                    ));
                }
                let mut actual = actual.clone();
                outer.clone().function(&mut actual);
                let outer_count = nominals.arguments(name).len();
                actual.generics.drain(..outer_count);
                apply_generics(&mut outer.clone(), &mut actual.generics);
                compatible(&required, &actual, constants, nominals)?;
            }
        }
        Ok(())
    }
    pub(super) fn prepare_markers(
        &self,
        generics: &[a::Generic],
        outer: &Substitution,
        constants: &crate::constants::Constants,
        nominals: &records::Nominals,
    ) -> Result<()> {
        let mut pending = Vec::new();
        for generic in generics {
            if let a::GenericKind::Type { constraints, .. } = &generic.kind {
                let marker = &outer.types[&generic.name];
                let Type::Record { name, .. } = marker else {
                    unreachable!()
                };
                self.markers.borrow_mut().insert(name.clone(), Vec::new());
                for constraint in constraints.iter().filter(|ty| !copy(ty)) {
                    pending.push((name.clone(), constraint.clone()));
                }
            }
        }
        // A bound's type arguments can depend on a later parameter's bounds.
        // Publish independently resolved claims before retrying those obligations.
        while !pending.is_empty() {
            let previous = pending.len();
            let mut next = Vec::new();
            let mut diagnostic = None;
            for (name, constraint) in pending {
                match self.instance(&constraint, outer, constants, nominals) {
                    Ok((bound, _)) => {
                        let mut markers = self.markers.borrow_mut();
                        let claims = markers.get_mut(&name).unwrap();
                        if !claims.contains(&bound) {
                            claims.push(bound)
                        }
                    }
                    Err(error) => {
                        diagnostic = Some(error);
                        next.push((name, constraint));
                    }
                }
            }
            if next.len() == previous {
                return Err(diagnostic.unwrap());
            }
            pending = next;
        }
        for generic in generics {
            if let a::GenericKind::Type { constraints, .. } = &generic.kind {
                for constraint in constraints {
                    self.require(
                        &outer.types[&generic.name],
                        constraint,
                        outer,
                        constants,
                        nominals,
                    )?;
                }
            }
        }
        Ok(())
    }
    pub(super) fn marker_methods(
        &self,
        marker: &Type,
        constants: &crate::constants::Constants,
        nominals: &records::Nominals,
    ) -> Result<Vec<a::Function>> {
        let Type::Record { name, .. } = marker else {
            unreachable!()
        };
        let bounds = self.markers.borrow().get(name).cloned().unwrap_or_default();
        let mut functions = BTreeMap::<String, a::Function>::new();
        for bound in bounds {
            for mut method in self.surface(&bound, marker, constants, nominals)? {
                let short = method.name.clone();
                method.name = format!("{name}.{short}");
                if let Some(old) = functions.get(&short) {
                    compatible(old, &method, constants, nominals)?;
                } else {
                    functions.insert(short, method);
                }
            }
        }
        Ok(functions.into_values().collect())
    }
}
fn apply_generics(substitution: &mut Substitution, generics: &mut [a::Generic]) {
    for generic in generics {
        match &mut generic.kind {
            a::GenericKind::Type {
                constraints,
                default,
            } => {
                for ty in constraints {
                    substitution.ty(ty)
                }
                if let Some(ty) = default {
                    substitution.ty(ty)
                }
            }
            a::GenericKind::Const { ty, default } => {
                substitution.ty(ty);
                if let Some(value) = default {
                    substitution.expr(value)
                }
            }
        }
    }
}
