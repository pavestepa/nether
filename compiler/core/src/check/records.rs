use super::generics::substitute::Substitution;
use super::*;
use std::{cell::RefCell, collections::BTreeSet};
#[derive(Clone)]
pub(super) enum Argument {
    Type(Type),
    Const(u64),
}
#[derive(Default)]
pub(super) struct Nominals {
    definitions: BTreeMap<String, a::Item>,
    instances: RefCell<BTreeMap<String, Type>>,
    active: RefCell<BTreeSet<String>>,
    opaque: RefCell<BTreeMap<String, bool>>,
    origins: RefCell<BTreeMap<String, String>>,
    arguments: RefCell<BTreeMap<String, Vec<Argument>>>,
}
impl Nominals {
    pub fn is_enum(&self, name: &str) -> bool {
        matches!(
            self.definitions.get(name).map(|item| &item.kind),
            Some(a::ItemKind::Enum { .. })
        ) || matches!(self.get(name), Some(Type::Enum { .. }))
    }
    pub fn marker(&self, name: String, copy: bool) -> Type {
        let ty = Type::Record {
            copy,
            name: name.clone(),
            fields: Vec::new(),
        };
        self.opaque.borrow_mut().insert(name.clone(), copy);
        self.instances.borrow_mut().insert(name, ty.clone());
        ty
    }
    pub fn copyable(&self, ty: &Type) -> bool {
        ty.copyable()
    }

    pub fn arguments(&self, name: &str) -> Vec<Argument> {
        self.arguments
            .borrow()
            .get(name)
            .cloned()
            .unwrap_or_default()
    }
    pub fn origin(&self, name: &str) -> String {
        self.origins
            .borrow()
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_owned())
    }
    pub fn get(&self, name: &str) -> Option<Type> {
        self.instances.borrow().get(name).cloned()
    }
    pub fn contains(&self, name: &str) -> bool {
        self.definitions.contains_key(name) || self.instances.borrow().contains_key(name)
    }
    pub fn resolve(
        &self,
        name: &str,
        args: &[a::TypeArgument],
        constants: &crate::constants::Constants,
        span: Span,
    ) -> Result<Type> {
        if args.is_empty() {
            if let Some(ty) = self.get(name) {
                return Ok(ty);
            }
        }
        let item = self
            .definitions
            .get(name)
            .ok_or_else(|| error(span, "E0310", format!("unknown type '{name}'")))?;
        let generics = match &item.kind {
            a::ItemKind::Aggregate { generics, .. } | a::ItemKind::Enum { generics, .. } => {
                generics
            }
            _ => unreachable!(),
        };
        if args.len() > generics.len() {
            return Err(error(span, "E0370", "too many type arguments"));
        }
        let mut substitution = Substitution::default();
        let mut names = BTreeSet::new();
        for (index, generic) in generics.iter().enumerate() {
            if !names.insert(&generic.name) {
                return Err(error(generic.span, "E0301", "duplicate generic parameter"));
            }
            match &generic.kind {
                a::GenericKind::Type {
                    constraints,
                    default,
                } => {
                    if constraints.iter().any(|ty|!matches!(&ty.kind,a::TypeKind::Named {path,arguments} if path==&["Copy".to_owned()] && arguments.is_empty())) {return Err(error(generic.span,"E0900","interface constraints require interface checking"));}
                    let mut ty = match args.get(index) {
                        Some(a::TypeArgument::Type(ty)) => ty.clone(),
                        None => default
                            .clone()
                            .ok_or_else(|| error(span, "E0371", "missing generic type argument"))?,
                        _ => return Err(error(span, "E0370", "expected type argument")),
                    };
                    substitution.ty(&mut ty);
                    let ty = resolve_type_with(&ty, constants, self)?;
                    if !constraints.is_empty() && !self.copyable(&ty) {
                        return Err(error(
                            span,
                            "E0373",
                            "generic argument does not satisfy Copy",
                        ));
                    }
                    substitution.types.insert(generic.name.clone(), ty);
                }
                a::GenericKind::Const { default, .. } => {
                    let mut value = match args.get(index) {
                        Some(a::TypeArgument::Const(value)) => value.clone(),
                        Some(a::TypeArgument::Type(a::Type {
                            kind: a::TypeKind::Named { path, arguments },
                            ..
                        })) if path.len() == 1 && arguments.is_empty() => a::Expr {
                            kind: a::ExprKind::Name(path[0].clone()),
                            span,
                        },
                        None => default.clone().ok_or_else(|| {
                            error(span, "E0371", "missing const generic argument")
                        })?,
                        _ => return Err(error(span, "E0370", "expected const argument")),
                    };
                    substitution.expr(&mut value);
                    let Value::Integer(value) =
                        constant_value(&value, &Type::Integer(I::Usize), constants)?
                    else {
                        unreachable!()
                    };
                    substitution
                        .constants
                        .insert(generic.name.clone(), value.bits() as u64);
                }
            }
        }
        let key = if generics.is_empty() {
            name.to_owned()
        } else {
            format!(
                "{name}<{:?};{:?}>",
                substitution.types, substitution.constants
            )
        };
        if let Some(ty) = self.get(&key) {
            return Ok(ty);
        }
        {
            let mut active = self.active.borrow_mut();
            if active.len() >= 32 || !active.insert(key.clone()) {
                return Err(error(
                    span,
                    "E0350",
                    "recursive inline type or type nesting limit",
                ));
            }
        }
        let result = self.build(item, &key, &mut substitution, constants);
        self.active.borrow_mut().remove(&key);
        let ty = result?;
        if item.attributes.contains(&a::Attribute::Copy) && !self.copyable(&ty) {
            return Err(error(span, "E0373", "Copy aggregate requires Copy fields"));
        }
        crate::llvm::layout(&ty).map_err(|message| error(span, "E0318", message))?;
        let arguments = generics
            .iter()
            .map(|generic| match generic.kind {
                a::GenericKind::Type { .. } => {
                    Argument::Type(substitution.types[&generic.name].clone())
                }
                a::GenericKind::Const { .. } => {
                    Argument::Const(substitution.constants[&generic.name])
                }
            })
            .collect();
        self.arguments.borrow_mut().insert(key.clone(), arguments);
        self.origins
            .borrow_mut()
            .insert(key.clone(), name.to_owned());
        self.instances.borrow_mut().insert(key, ty.clone());
        Ok(ty)
    }
    fn build(
        &self,
        item: &a::Item,
        name: &str,
        substitution: &mut Substitution,
        constants: &crate::constants::Constants,
    ) -> Result<Type> {
        let mut resolve = |ty: &a::Type| {
            let mut ty = ty.clone();
            substitution.ty(&mut ty);
            resolve_type_with(&ty, constants, self)
        };
        match &item.kind {
            a::ItemKind::Aggregate {
                interfaces,
                members,
                ..
            } => {
                if !interfaces.is_empty() {
                    return Err(error(
                        item.span,
                        "E0900",
                        "interface implementations require interface checking",
                    ));
                }
                let mut fields = Vec::new();
                for member in members {
                    let a::Member::Field {
                        name,
                        ty,
                        private,
                        weak: false,
                        span,
                    } = member
                    else {
                        return Err(error(
                            item.span,
                            "E0900",
                            "struct methods and destructors require ownership checking",
                        ));
                    };
                    if fields.iter().any(|f: &h::RecordField| f.name == *name) {
                        return Err(error(*span, "E0301", "duplicate field"));
                    }
                    fields.push(h::RecordField {
                        name: name.clone(),
                        ty: resolve(ty)?,
                        private: *private,
                        source: span.source,
                    });
                }
                Ok(Type::Record {
                    copy: item.attributes.contains(&a::Attribute::Copy),
                    name: name.into(),
                    fields,
                })
            }
            a::ItemKind::Enum { variants, .. } => {
                let mut checked = Vec::new();
                let mut names = BTreeSet::new();
                for variant in variants {
                    if !names.insert(&variant.name) {
                        return Err(error(variant.span, "E0301", "duplicate enum variant"));
                    }
                    let (types, shape): (Vec<&a::Type>, _) = match &variant.fields {
                        a::VariantFields::Unit => (vec![], h::VariantShape::Unit),
                        a::VariantFields::Tuple(fields) => {
                            (fields.iter().collect(), h::VariantShape::Tuple)
                        }
                        a::VariantFields::Named(fields) => {
                            let mut names = BTreeSet::new();
                            for (name, _) in fields {
                                if !names.insert(name) {
                                    return Err(error(
                                        variant.span,
                                        "E0301",
                                        "duplicate variant field",
                                    ));
                                }
                            }
                            (
                                fields.iter().map(|(_, ty)| ty).collect(),
                                h::VariantShape::Named(
                                    fields.iter().map(|(name, _)| name.clone()).collect(),
                                ),
                            )
                        }
                    };
                    let fields = types
                        .into_iter()
                        .map(&mut resolve)
                        .collect::<Result<Vec<_>>>()?;
                    checked.push(h::Variant {
                        name: variant.name.clone(),
                        fields,
                        shape,
                    });
                }
                Ok(Type::Enum {
                    copy: item.attributes.contains(&a::Attribute::Copy),
                    name: name.into(),
                    variants: checked,
                })
            }
            _ => unreachable!(),
        }
    }
}
pub(super) fn collect(
    module: &a::Module,
    constants: &crate::constants::Constants,
) -> Result<Nominals> {
    let mut nominals = Nominals::default();
    let mut roots = Vec::new();
    for item in &module.items {
        if let a::ItemKind::Aggregate {
            class: false,
            name,
            generics,
            ..
        }
        | a::ItemKind::Enum { name, generics, .. } = &item.kind
        {
            if nominals
                .definitions
                .insert(name.clone(), item.clone())
                .is_some()
            {
                return Err(error(item.span, "E0301", "duplicate type declaration"));
            }
            if generics.is_empty() {
                roots.push((name.clone(), item.span));
            }
        }
    }
    for (name, span) in roots {
        nominals.resolve(&name, &[], constants, span)?;
    }
    Ok(nominals)
}
impl Checker<'_> {
    pub(super) fn record(
        &mut self,
        ty: &a::Type,
        initializers: &[(String, a::Expr)],
        expected: Option<&Type>,
        span: Span,
    ) -> Result<h::Expr> {
        if let Some(value) = self.enum_named(ty, initializers, expected, span)? {
            return Ok(value);
        }
        let contextual = match (&ty.kind, expected) {
            (a::TypeKind::Named { path, arguments }, Some(Type::Record { name, .. }))
                if arguments.is_empty() && path == &[self.nominals.origin(name)] =>
            {
                expected.cloned()
            }
            _ => None,
        };
        let ty = if let Some(ty) = contextual {
            ty
        } else {
            resolve_type_with(ty, self.constants, self.nominals)?
        };
        if let Some(expected) = expected {
            same(expected, &ty, span)?;
        }
        let Type::Record { fields, .. } = &ty else {
            return Err(error(span, "E0351", "named construction requires a struct"));
        };
        let mut values = vec![None; fields.len()];
        let mut statements = Vec::new();
        // Evaluate written order, then assemble in declaration order.
        for (name, source) in initializers {
            let index = fields
                .iter()
                .position(|f| f.name == *name)
                .ok_or_else(|| error(source.span, "E0351", format!("unknown field '{name}'")))?;
            let field = &fields[index];
            if field.private && field.source != span.source {
                return Err(error(
                    source.span,
                    "E0352",
                    "private field cannot be initialized from this module",
                ));
            }
            if values[index].is_some() {
                return Err(error(
                    source.span,
                    "E0351",
                    "field initialized more than once",
                ));
            }
            let value = self.expr(source, Some(&field.ty), true)?;
            let id = self.locals.len();
            self.locals.push(h::Local {
                view: None,
                name: format!("<field {name}>"),
                ty: field.ty.clone(),
                mutable: false,
                span: source.span,
            });
            statements.push(h::Statement::Initialize(id, value));
            values[index] = Some(h::Expr {
                kind: h::ExprKind::Local(id),
                ty: field.ty.clone(),
                span: source.span,
            });
        }
        if values.iter().any(Option::is_none) {
            return Err(error(
                span,
                "E0351",
                "every struct field must be initialized",
            ));
        }
        statements.push(h::Statement::Evaluate(h::Expr {
            kind: h::ExprKind::Tuple(values.into_iter().map(Option::unwrap).collect()),
            ty: ty.clone(),
            span,
        }));
        Ok(h::Expr {
            kind: h::ExprKind::Block(h::Block {
                storage: vec![],
                statements,
                result: ty.clone(),
                diverges: false,
                span,
            }),
            ty,
            span,
        })
    }
}
