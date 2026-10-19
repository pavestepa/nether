use super::*;
#[derive(Clone, Default)]
pub(super) struct Bindings {
    types: BTreeMap<String, a::Type>,
    constants: BTreeMap<String, a::Expr>,
}
impl Bindings {
    fn module_constants(generics: &[a::Generic], constants: &crate::constants::Constants) -> Self {
        let mut bindings = Self::default();
        for (name, (value, ty)) in constants {
            if generics.iter().any(|g| &g.name == name) {
                continue;
            }
            if *ty == Type::Integer(I::Usize) {
                let Value::Integer(value) = value else {
                    unreachable!()
                };
                bindings.constants.insert(
                    name.clone(),
                    a::Expr {
                        kind: a::ExprKind::Literal(a::Literal::Integer(value.bits().to_string())),
                        span: generics.first().map(|g| g.span).unwrap_or(Span {
                            source: nether_frontend::source::SourceId(0),
                            start: 0,
                            end: 0,
                        }),
                    },
                );
            }
        }
        bindings
    }
    pub(super) fn parent(
        generics: &[a::Generic],
        arguments: &[a::TypeArgument],
        span: Span,
        constants: &crate::constants::Constants,
    ) -> Result<Self> {
        if arguments.len() > generics.len() {
            return Err(error(span, "E0370", "too many base class arguments"));
        }
        let module = Self::module_constants(generics, constants);
        let mut result = Self::default();
        for (i, generic) in generics.iter().enumerate() {
            match &generic.kind {
                a::GenericKind::Type { default, .. } => {
                    let ty = match arguments.get(i) {
                        Some(a::TypeArgument::Type(ty)) => ty.clone(),
                        Some(_) => {
                            return Err(error(span, "E0370", "expected base class type argument"))
                        }
                        None => {
                            let mut ty = default.clone().ok_or_else(|| {
                                error(span, "E0371", "missing base class type argument")
                            })?;
                            module.ty(&mut ty);
                            result.ty(&mut ty);
                            ty
                        }
                    };
                    result.types.insert(generic.name.clone(), ty);
                }
                a::GenericKind::Const { default, .. } => {
                    let value = match arguments.get(i) {
                        Some(a::TypeArgument::Const(value)) => value.clone(),
                        Some(a::TypeArgument::Type(a::Type {
                            kind: a::TypeKind::Named { path, arguments },
                            span,
                        })) if path.len() == 1 && arguments.is_empty() => a::Expr {
                            kind: a::ExprKind::Name(path[0].clone()),
                            span: *span,
                        },
                        Some(_) => {
                            return Err(error(span, "E0370", "expected base class const argument"))
                        }
                        None => {
                            let mut value = default.clone().ok_or_else(|| {
                                error(span, "E0371", "missing base class const argument")
                            })?;
                            module.expr(&mut value);
                            result.expr(&mut value);
                            value
                        }
                    };
                    result.constants.insert(generic.name.clone(), value);
                }
            }
        }
        Ok(result)
    }
    fn argument(&self, argument: &mut a::TypeArgument) {
        if let a::TypeArgument::Type(a::Type {
            kind: a::TypeKind::Named { path, arguments },
            ..
        }) = argument
        {
            if path.len() == 1 && arguments.is_empty() {
                if let Some(value) = self.constants.get(&path[0]) {
                    *argument = a::TypeArgument::Const(value.clone());
                    return;
                }
            }
        }
        match argument {
            a::TypeArgument::Type(ty) => self.ty(ty),
            a::TypeArgument::Const(value) => self.expr(value),
        }
    }
    fn ty(&self, ty: &mut a::Type) {
        match &mut ty.kind {
            a::TypeKind::Named { path, arguments } => {
                if path.len() == 1 && arguments.is_empty() {
                    if let Some(actual) = self.types.get(&path[0]) {
                        *ty = actual.clone();
                        return;
                    }
                }
                for arg in arguments {
                    self.argument(arg);
                }
            }
            a::TypeKind::Tuple(fields) => {
                for ty in fields {
                    self.ty(ty);
                }
            }
            a::TypeKind::Array { element, length } => {
                self.ty(element);
                self.expr(length);
            }
            a::TypeKind::Pointer { pointee, .. } => self.ty(pointee),
            a::TypeKind::Function { parameters, result } => {
                for (_, ty) in parameters {
                    self.ty(ty);
                }
                self.ty(result);
            }
        }
    }
    fn expr(&self, value: &mut a::Expr) {
        match &mut value.kind {
            a::ExprKind::Name(name) => {
                if let Some(actual) = self.constants.get(name) {
                    *value = actual.clone();
                }
            }
            a::ExprKind::Group(value) | a::ExprKind::Unary { value, .. } => self.expr(value),
            a::ExprKind::Binary { left, right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            a::ExprKind::Cast { value, ty } => {
                self.expr(value);
                self.ty(ty);
            }
            a::ExprKind::If {
                condition,
                then_block,
                else_branch,
            } => {
                self.expr(condition);
                self.block(then_block);
                if let Some(value) = else_branch {
                    self.expr(value);
                }
            }
            a::ExprKind::Branch(block) => self.block(block),
            _ => {}
        }
    }
    fn block(&self, block: &mut a::Block) {
        for statement in &mut block.statements {
            if let a::StatementKind::Expression(value) = &mut statement.kind {
                self.expr(value);
            }
        }
    }
    fn signature(&self, method: &mut a::Function) {
        for parameter in &mut method.parameters {
            if let Some(ty) = &mut parameter.ty {
                self.ty(ty);
            }
        }
        self.ty(&mut method.result);
        for generic in &mut method.generics {
            match &mut generic.kind {
                a::GenericKind::Type {
                    constraints,
                    default,
                } => {
                    for ty in constraints {
                        self.ty(ty);
                    }
                    if let Some(ty) = default {
                        self.ty(ty);
                    }
                }
                a::GenericKind::Const { ty, default } => {
                    self.ty(ty);
                    if let Some(value) = default {
                        self.expr(value);
                    }
                }
            }
        }
    }
    pub(super) fn method(&self, method: &mut a::Function) {
        // Freshen method binders before substituting class arguments. Otherwise
        // A<T>.f<U>(x:T), inherited by B<U>:A<U>, would capture the class U as
        // the method's U. Declarations preserve their lexical scope across edges.
        let mut locals = Self::default();
        let mut renamed = Vec::new();
        for (index, generic) in method.generics.iter().enumerate() {
            let name = format!(
                "#local{:?}:{}:{index}",
                method.span.source, method.span.start
            );
            match generic.kind {
                a::GenericKind::Type { .. } => {
                    locals.types.insert(
                        generic.name.clone(),
                        a::Type {
                            kind: a::TypeKind::Named {
                                path: vec![name.clone()],
                                arguments: vec![],
                            },
                            span: generic.span,
                        },
                    );
                }
                a::GenericKind::Const { .. } => {
                    locals.constants.insert(
                        generic.name.clone(),
                        a::Expr {
                            kind: a::ExprKind::Name(name.clone()),
                            span: generic.span,
                        },
                    );
                }
            }
            renamed.push(name);
        }
        locals.signature(method);
        for (generic, name) in method.generics.iter_mut().zip(renamed) {
            generic.name = name;
        }
        self.signature(method);
    }
}
struct Keys<'a> {
    constants: &'a crate::constants::Constants,
}
impl Keys<'_> {
    fn expr(&self, value: &a::Expr) -> Result<String> {
        if let Ok(Value::Integer(value)) =
            constant_value(value, &Type::Integer(I::Usize), self.constants)
        {
            return Ok(value.bits().to_string());
        }
        Ok(match &value.kind {
            a::ExprKind::Name(name) => name.clone(),
            a::ExprKind::Literal(value) => format!("{value:?}"),
            a::ExprKind::Group(value) => return self.expr(value),
            a::ExprKind::Unary { operator, value } => format!("{operator}({})", self.expr(value)?),
            a::ExprKind::Binary {
                operator,
                left,
                right,
            } => format!("{operator}({},{})", self.expr(left)?, self.expr(right)?),
            a::ExprKind::Cast { value, ty } => {
                format!("cast({},{})", self.expr(value)?, self.ty(ty)?)
            }
            a::ExprKind::If {
                condition,
                then_block,
                else_branch,
            } => format!(
                "if({},{},{})",
                self.expr(condition)?,
                self.block(then_block)?,
                else_branch
                    .as_ref()
                    .map(|e| self.expr(e))
                    .transpose()?
                    .unwrap_or_default()
            ),
            a::ExprKind::Branch(block) => return self.block(block),
            _ => {
                return Err(error(
                    value.span,
                    "E0411",
                    "unsupported symbolic const expression in override signature",
                ))
            }
        })
    }
    fn block(&self, block: &a::Block) -> Result<String> {
        block
            .statements
            .iter()
            .map(|statement| match &statement.kind {
                a::StatementKind::Expression(value) => self.expr(value),
                _ => Err(error(
                    statement.span,
                    "E0411",
                    "const signature requires pure expressions",
                )),
            })
            .collect::<Result<Vec<_>>>()
            .map(|v| format!("{{{}}}", v.join(";")))
    }
    fn argument(&self, argument: &a::TypeArgument) -> Result<String> {
        match argument {
            a::TypeArgument::Const(value) => self.expr(value),
            a::TypeArgument::Type(a::Type {
                kind: a::TypeKind::Named { path, arguments },
                span,
            }) if path.len() == 1
                && arguments.is_empty()
                && (path[0].starts_with("#const") || self.constants.contains_key(&path[0])) =>
            {
                self.expr(&a::Expr {
                    kind: a::ExprKind::Name(path[0].clone()),
                    span: *span,
                })
            }
            a::TypeArgument::Type(ty) => self.ty(ty),
        }
    }
    fn ty(&self, ty: &a::Type) -> Result<String> {
        Ok(match &ty.kind {
            a::TypeKind::Named { path, arguments } => format!(
                "{}<{}>",
                path.join("::"),
                arguments
                    .iter()
                    .map(|a| self.argument(a))
                    .collect::<Result<Vec<_>>>()?
                    .join(",")
            ),
            a::TypeKind::Tuple(fields) => format!(
                "({})",
                fields
                    .iter()
                    .map(|t| self.ty(t))
                    .collect::<Result<Vec<_>>>()?
                    .join(",")
            ),
            a::TypeKind::Array { element, length } => {
                format!("{{{};{}}}", self.ty(element)?, self.expr(length)?)
            }
            a::TypeKind::Pointer { mutable, pointee } => {
                format!("ptr({mutable},{})", self.ty(pointee)?)
            }
            a::TypeKind::Function { parameters, result } => format!(
                "fn({})->{}",
                parameters
                    .iter()
                    .map(|(m, t)| Ok(format!("{m}:{}", self.ty(t)?)))
                    .collect::<Result<Vec<_>>>()?
                    .join(","),
                self.ty(result)?
            ),
        })
    }
    fn signature(&self, method: &a::Function) -> Result<String> {
        let mut method = method.clone();
        let mut alpha = Bindings::default();
        for (index, generic) in method.generics.iter().enumerate() {
            match generic.kind {
                a::GenericKind::Type { .. } => {
                    alpha.types.insert(
                        generic.name.clone(),
                        a::Type {
                            kind: a::TypeKind::Named {
                                path: vec![format!("#type{index}")],
                                arguments: vec![],
                            },
                            span: generic.span,
                        },
                    );
                }
                a::GenericKind::Const { .. } => {
                    alpha.constants.insert(
                        generic.name.clone(),
                        a::Expr {
                            kind: a::ExprKind::Name(format!("#const{index}")),
                            span: generic.span,
                        },
                    );
                }
            }
        }
        alpha.signature(&mut method);
        let generics = method
            .generics
            .iter()
            .map(|g| {
                Ok(match &g.kind {
                    a::GenericKind::Type {
                        constraints,
                        default,
                    } => format!(
                        "type({:?},{:?})",
                        constraints
                            .iter()
                            .map(|t| self.ty(t))
                            .collect::<Result<BTreeSet<_>>>()?,
                        default.as_ref().map(|t| self.ty(t)).transpose()?
                    ),
                    a::GenericKind::Const { ty, default } => format!(
                        "const({},{:?})",
                        self.ty(ty)?,
                        default.as_ref().map(|e| self.expr(e)).transpose()?
                    ),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let parameters = method
            .parameters
            .iter()
            .map(|p| {
                Ok((
                    p.receiver,
                    p.mutable,
                    p.ty.as_ref().map(|t| self.ty(t)).transpose()?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(format!(
            "{generics:?}:{parameters:?}:{}",
            self.ty(&method.result)?
        ))
    }
}
// Resolve lexical module constants before crossing an inheritance edge. A
// derived class's generic N must not capture a free module constant N in A.
pub(super) fn freeze_constants(
    method: &mut a::Function,
    generics: &[a::Generic],
    constants: &crate::constants::Constants,
) {
    let bindings = Bindings::module_constants(generics, constants);
    bindings.method(method);
    method.body = None;
}
pub(super) fn compatible(
    required: &a::Function,
    actual: &a::Function,
    generics: &[a::Generic],
    constants: &crate::constants::Constants,
) -> Result<bool> {
    let mut scoped = constants.clone();
    for generic in generics {
        scoped.remove(&generic.name);
    }
    let keys = Keys { constants: &scoped };
    Ok(keys.signature(required)? == keys.signature(actual)?)
}
