use super::generics::substitute::Substitution;
use super::*;
use std::{cell::RefCell, collections::BTreeSet};
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Argument {
    Type(Type),
    Const(u64),
}
#[derive(Default)]
pub(super) struct Nominals {
    pub(super) interfaces: interfaces::Registry,
    definitions: BTreeMap<String, a::Item>,
    classes: RefCell<h::Classes>,
    instances: RefCell<BTreeMap<String, Type>>,
    active: RefCell<BTreeSet<String>>,
    pending_classes: RefCell<Vec<(a::Item, String, Substitution, usize)>>,
    class_depth: std::cell::Cell<usize>,
    opaque: RefCell<BTreeMap<String, bool>>,
    origins: RefCell<BTreeMap<String, String>>,
    arguments: RefCell<BTreeMap<String, Vec<Argument>>>,
}
impl Nominals {
    pub fn register_storage(&self, ty: &Type) {
        match ty {
            Type::Class {
                storage: Some(fields),
                copy,
                ..
            } => {
                self.classes.borrow_mut().insert(
                    ty.class_key(),
                    h::ClassDefinition {
                        fields: fields.clone(),
                        copy: *copy,
                    },
                );
                for field in fields {
                    self.register_storage(&field.ty)
                }
            }
            Type::Record { fields, .. } => {
                for field in fields {
                    self.register_storage(&field.ty)
                }
            }
            Type::Tuple(fields) => {
                for field in fields {
                    self.register_storage(field)
                }
            }
            Type::Array(ty, _) | Type::StoredView(ty) => self.register_storage(ty),
            Type::Enum { variants, .. } => {
                for field in variants.iter().flat_map(|v| &v.fields) {
                    self.register_storage(field)
                }
            }
            Type::Function {
                parameters, result, ..
            } => {
                for (_, ty) in parameters {
                    self.register_storage(ty)
                }
                self.register_storage(result)
            }
            _ => {}
        }
    }
    pub(super) fn interface_claims(
        &self,
        ty: &Type,
        constants: &crate::constants::Constants,
    ) -> Result<Vec<interfaces::Bound>> {
        let (Type::Record { name, .. } | Type::Class { name, .. }) = ty else {
            return Ok(Vec::new());
        };
        let Some(item) = self.definitions.get(&self.origin(name)) else {
            return Ok(Vec::new());
        };
        let a::ItemKind::Aggregate {
            generics,
            interfaces,
            ..
        } = &item.kind
        else {
            return Ok(Vec::new());
        };
        let mut outer = Substitution::default();
        for (generic, arg) in generics.iter().zip(self.arguments(name)) {
            match arg {
                Argument::Type(ty) => {
                    outer.types.insert(generic.name.clone(), ty);
                }
                Argument::Const(n) => {
                    outer.constants.insert(generic.name.clone(), n);
                }
            }
        }
        self.interfaces.claims(interfaces, &outer, constants, self)
    }
    pub(super) fn field_declaration(&self, owner: &str, name: &str) -> Option<Span> {
        let a::ItemKind::Aggregate { members, .. } = &self.definitions.get(owner)?.kind else {
            return None;
        };
        members.iter().find_map(|member| match member {
            a::Member::Field {
                name: field, span, ..
            } if field == name => Some(*span),
            _ => None,
        })
    }
    pub fn classes(&self) -> h::Classes {
        self.classes.borrow().clone()
    }
    pub fn fields(&self, ty: &Type) -> Option<Vec<h::RecordField>> {
        match ty {
            Type::Record { fields, .. } => Some(fields.clone()),
            Type::Class {
                storage: Some(fields),
                ..
            } => Some(fields.clone()),
            Type::Class { name, .. } => self.classes.borrow().get(name).map(|c| c.fields.clone()),
            _ => None,
        }
    }

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
                    constraints: _,
                    default,
                } => {
                    let mut ty = match args.get(index) {
                        Some(a::TypeArgument::Type(ty)) => ty.clone(),
                        None => default
                            .clone()
                            .ok_or_else(|| error(span, "E0371", "missing generic type argument"))?,
                        _ => return Err(error(span, "E0370", "expected type argument")),
                    };
                    substitution.ty(&mut ty);
                    let ty = resolve_type_with(&ty, constants, self)?;
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
        for generic in generics {
            if let a::GenericKind::Type { constraints, .. } = &generic.kind {
                for constraint in constraints {
                    self.interfaces.require(
                        &substitution.types[&generic.name],
                        constraint,
                        &substitution,
                        constants,
                        self,
                    )?;
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
        let is_class = matches!(item.kind, a::ItemKind::Aggregate { class: true, .. });
        if is_class {
            // Register the identity handle before resolving its payload. A class
            // edge breaks inline layout recursion without leaking Rc cycles.
            self.instances.borrow_mut().insert(
                key.clone(),
                Type::Class {
                    name: key.clone(),
                    storage: None,
                    copy: item.attributes.contains(&a::Attribute::Copy),
                },
            );
        }
        let defer_class = is_class && !self.active.borrow().is_empty();
        let depth = self.active.borrow().len() + self.class_depth.get();
        if depth >= 32 {
            return Err(error(
                span,
                "E0350",
                "recursive inline type or type nesting limit",
            ));
        }
        let result = if defer_class {
            // Class handles break layout recursion even when resolution started
            // from an inline enum/struct. Complete this payload after that inline
            // layout is available, independent of declaration order.
            self.pending_classes.borrow_mut().push((
                item.clone(),
                key.clone(),
                substitution.clone(),
                depth,
            ));
            Ok(self.get(&key).unwrap())
        } else {
            {
                let mut active = self.active.borrow_mut();
                if !active.insert(key.clone()) {
                    return Err(error(
                        span,
                        "E0350",
                        "recursive inline type or type nesting limit",
                    ));
                }
            }
            let result = self.build(item, &key, &mut substitution, constants);
            self.active.borrow_mut().remove(&key);
            result
        };
        let ty = match result {
            Ok(ty) => ty,
            Err(error) => {
                if is_class {
                    self.instances.borrow_mut().remove(&key);
                }
                return Err(error);
            }
        };
        if item.attributes.contains(&a::Attribute::Copy) && !self.copyable(&ty) {
            return Err(error(span, "E0373", "Copy aggregate requires Copy fields"));
        }
        crate::layout::layout(&ty).map_err(|message| error(span, "E0318", message))?;
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
        if let a::ItemKind::Aggregate { interfaces, .. } = &item.kind {
            self.interfaces.validate_implementation(
                &ty,
                interfaces,
                &substitution,
                constants,
                self,
            )?;
        }
        if self.active.borrow().is_empty() {
            loop {
                let pending = self.pending_classes.borrow_mut().pop();
                let Some((item, key, mut substitution, depth)) = pending else {
                    break;
                };
                self.active.borrow_mut().insert(key.clone());
                let previous = self.class_depth.replace(depth);
                let result = self.build(&item, &key, &mut substitution, constants);
                self.class_depth.set(previous);
                self.active.borrow_mut().remove(&key);
                result?;
            }
        }
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
                class,
                interfaces,
                members,
                ..
            } => {
                for interface in interfaces {
                    if !matches!(&interface.kind,a::TypeKind::Named {path,..} if self.interfaces.contains(&path.join("::")))
                    {
                        return Err(error(interface.span,"E0900","embedded base construction and inherited method lowering require the remaining C4.2 work"));
                    }
                }
                if *class {
                    super::constructors::validate(members)?;
                }
                let mut fields = Vec::new();
                for member in members {
                    if *class && !matches!(member, a::Member::Field { .. }) {
                        continue;
                    }
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
                let copy = item.attributes.contains(&a::Attribute::Copy);
                if *class {
                    if copy && fields.iter().any(|f| !f.ty.copyable()) {
                        return Err(error(item.span, "E0373", "Copy class requires Copy fields"));
                    }
                    // Validate the payload separately: the handle itself always
                    // has pointer layout, even when a payload field is recursive.
                    crate::layout::layout(&Type::Tuple(
                        fields.iter().map(|f| f.ty.clone()).collect(),
                    ))
                    .map_err(|message| error(item.span, "E0318", message))?;
                    self.classes
                        .borrow_mut()
                        .insert(name.into(), h::ClassDefinition { copy, fields });
                    return Ok(Type::Class {
                        name: name.into(),
                        copy,
                        storage: None,
                    });
                }
                Ok(Type::Record {
                    copy,
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
    let mut nominals = Nominals {
        interfaces: interfaces::Registry::collect(module)?,
        ..Default::default()
    };
    let mut roots = Vec::new();
    for item in &module.items {
        if let a::ItemKind::Aggregate { name, generics, .. }
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
    nominals
        .interfaces
        .validate_declarations(constants, &nominals)?;
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
        let mut ty = expected
            .filter(|expected| {
                matches!(expected, Type::Record { .. }) && expected.logical() == ty.logical()
            })
            .cloned()
            .unwrap_or(ty);
        let owner = ty.clone();
        let Type::Record { fields, .. } = &mut ty else {
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
            let field = &mut fields[index];
            self.check_field_access(&owner, field, source.span)?;
            if values[index].is_some() {
                return Err(error(
                    source.span,
                    "E0351",
                    "field initialized more than once",
                ));
            }
            let value = self.storage_value(source, Some(&field.ty))?;
            field.ty = value.ty.clone();
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
        let construction: Vec<_> = values
            .iter()
            .map(|v| {
                let h::ExprKind::Local(id) = v.as_ref().unwrap().kind else {
                    unreachable!()
                };
                id
            })
            .collect();
        statements.push(h::Statement::Evaluate(h::Expr {
            kind: h::ExprKind::Tuple(values.into_iter().map(Option::unwrap).collect()),
            ty: ty.clone(),
            span,
        }));
        Ok(h::Expr {
            kind: h::ExprKind::Block(h::Block {
                construction: construction.clone(),
                storage: construction,
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

#[cfg(test)]
mod class_tests {
    use super::*;
    use nether_frontend::{parser::parse, source::SourceId};
    fn registry(source: &str) -> Result<Nominals> {
        let parsed = parse(SourceId(0), source);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        collect(&parsed.module.unwrap(), &BTreeMap::new())
    }
    #[test]
    fn class_indirection_breaks_recursive_payload_layout() {
        let nominals=registry("class Node {next:Link;value:i32} enum Link {End,More(Node)} class Left {right:Right} class Right {left:Left}").unwrap();
        let node = nominals.get("Node").unwrap();
        assert!(matches!(node, Type::Class { copy: false, .. }));
        assert_eq!(
            crate::layout::layout(&node).unwrap(),
            nether_semantics::Layout::POINTER
        );
        let classes = nominals.classes.borrow();
        assert_eq!(classes["Node"].fields.len(), 2);
        assert!(matches!(&classes["Left"].fields[0].ty,Type::Class {name,..} if name=="Right"));
        assert!(matches!(&classes["Right"].fields[0].ty,Type::Class {name,..} if name=="Left"));
        assert!(registry("struct Node {next:Node}").is_err());
        let reversed =
            registry("enum Link {End,More(Node)} class Node {next:Link;value:i32}").unwrap();
        assert_eq!(reversed.classes()["Node"].fields.len(), 2);
        let generic=registry("enum Link<T> {End,More(Node<T>)} class Node<T> {next:Link<T>;value:T} struct Root {link:Link<i32>}").unwrap();
        assert_eq!(generic.classes().len(), 1);
        assert!(registry("struct Root {node:Node<i32>} class Node<T> {next:Node<{T;1}>}").is_err());
    }
    #[test]
    fn generic_class_instances_keep_distinct_payload_definitions() {
        let nominals =
            registry("class Cell<T> {value:T} struct Holder {small:Cell<i32>;large:Cell<i64>}")
                .unwrap();
        let classes = nominals.classes.borrow();
        assert_eq!(classes.len(), 2);
        let types = classes
            .values()
            .map(|c| c.fields[0].ty.clone())
            .collect::<Vec<_>>();
        assert!(types.contains(&Type::Integer(I::I32)));
        assert!(types.contains(&Type::Integer(I::I64)));
    }
    #[test]
    fn copy_class_requires_copy_payload_and_still_has_pointer_layout() {
        assert!(registry("class Owned {} #[Copy] class Bad {value:Owned}").is_err());
        let nominals = registry("#[Copy] class Number {value:i32}").unwrap();
        let number = nominals.get("Number").unwrap();
        assert!(number.copyable());
        assert_eq!(
            crate::layout::layout(&number).unwrap(),
            nether_semantics::Layout::POINTER
        );
    }
}
