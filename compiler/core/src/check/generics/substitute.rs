use super::*;
#[derive(Default)]
pub(in crate::check) struct Substitution {
    pub types: BTreeMap<String, Type>,
    pub constants: BTreeMap<String, u64>,
    shadowed: Vec<std::collections::BTreeSet<String>>,
}
impl Substitution {
    pub fn ty(&mut self, ty: &mut a::Type) {
        if let a::TypeKind::Named { path, arguments } = &ty.kind {
            if path.len() == 1 && arguments.is_empty() {
                if let Some(value) = self.types.get(&path[0]) {
                    *ty = ast_type(value, ty.span);
                    return;
                }
            }
        }
        match &mut ty.kind {
            a::TypeKind::Named { arguments, .. } => self.arguments(arguments),
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
    fn arguments(&mut self, args: &mut [a::TypeArgument]) {
        for arg in args {
            if let a::TypeArgument::Type(a::Type {
                kind: a::TypeKind::Named { path, arguments },
                span,
            }) = arg
            {
                if path.len() == 1 && arguments.is_empty() {
                    if let Some(value) = self.constants.get(&path[0]) {
                        *arg = a::TypeArgument::Const(number(*value, *span));
                        continue;
                    }
                }
            }
            match arg {
                a::TypeArgument::Type(ty) => self.ty(ty),
                a::TypeArgument::Const(value) => self.expr(value),
            }
        }
    }
    pub fn function(&mut self, function: &mut a::Function) {
        self.shadowed.push(Default::default());
        for param in &mut function.parameters {
            if let Some(ty) = &mut param.ty {
                self.ty(ty);
            }
            self.pattern(&mut param.pattern);
        }
        self.ty(&mut function.result);
        if let Some(body) = &mut function.body {
            self.block(body);
        }
        self.shadowed.pop();
    }
    fn pattern(&mut self, pattern: &mut a::Pattern) {
        match &mut pattern.kind {
            a::PatternKind::Binding { name, .. } => {
                if let Some(scope) = self.shadowed.last_mut() {
                    scope.insert(name.clone());
                }
            }
            a::PatternKind::Tuple(fields) | a::PatternKind::Array(fields) => {
                for field in fields {
                    self.pattern(field);
                }
            }
            a::PatternKind::Variant { path, fields } => {
                if let Some(name) = path.first_mut() {
                    if let Some(
                        Type::Record { name: actual, .. } | Type::Enum { name: actual, .. },
                    ) = self.types.get(name)
                    {
                        *name = actual.clone();
                    }
                }
                match fields {
                    a::PatternFields::Tuple(fields) => {
                        for p in fields {
                            self.pattern(p);
                        }
                    }
                    a::PatternFields::Named(fields, _) => {
                        for (_, p) in fields {
                            self.pattern(p);
                        }
                    }
                    _ => (),
                }
            }
            _ => (),
        }
    }
    fn block(&mut self, block: &mut a::Block) {
        self.shadowed.push(Default::default());
        for statement in &mut block.statements {
            match &mut statement.kind {
                a::StatementKind::Binding {
                    pattern, ty, value, ..
                } => {
                    if let Some(ty) = ty {
                        self.ty(ty);
                    }
                    self.expr(value);
                    self.pattern(pattern);
                }
                a::StatementKind::Expression(value) | a::StatementKind::Return(Some(value)) => {
                    self.expr(value)
                }
                a::StatementKind::Unsafe(block) => self.block(block),
                _ => (),
            }
        }
        self.shadowed.pop();
    }
    pub fn expr(&mut self, expr: &mut a::Expr) {
        if let a::ExprKind::Name(name) = &expr.kind {
            if !self.shadowed.iter().any(|scope| scope.contains(name)) {
                if let Some(value) = self.constants.get(name) {
                    *expr = number(*value, expr.span);
                    return;
                }
            }
        }
        match &mut expr.kind {
            a::ExprKind::Specialize { value, arguments } => {
                self.expr(value);
                self.arguments(arguments);
            }
            a::ExprKind::Group(value)
            | a::ExprKind::Unary { value, .. }
            | a::ExprKind::Member { value, .. } => self.expr(value),
            a::ExprKind::Tuple(values) | a::ExprKind::Array(values) => {
                for value in values {
                    self.expr(value);
                }
            }
            a::ExprKind::Construct { ty, fields } => {
                self.ty(ty);
                for (_, value) in fields {
                    self.expr(value);
                }
            }
            a::ExprKind::New { ty, arguments } => {
                self.ty(ty);
                for arg in arguments {
                    self.expr(arg);
                }
            }
            a::ExprKind::Call {
                callee,
                types,
                arguments,
            } => {
                self.expr(callee);
                self.arguments(types);
                for arg in arguments {
                    self.expr(arg);
                }
            }
            a::ExprKind::Index { value, index } => {
                self.expr(value);
                self.expr(index);
            }
            a::ExprKind::Binary { left, right, .. }
            | a::ExprKind::Assign {
                place: left,
                value: right,
                ..
            } => {
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
                if let Some(branch) = else_branch {
                    self.expr(branch);
                }
            }
            a::ExprKind::While { condition, body } => {
                self.expr(condition);
                self.block(body);
            }
            a::ExprKind::Branch(block) => self.block(block),
            a::ExprKind::Match { value, arms } => {
                self.expr(value);
                for arm in arms {
                    self.shadowed.push(Default::default());
                    self.pattern(&mut arm.pattern);
                    if let Some(guard) = &mut arm.guard {
                        self.expr(guard);
                    }
                    self.expr(&mut arm.value);
                    self.shadowed.pop();
                }
            }
            _ => (),
        }
    }
}
fn number(value: u64, span: Span) -> a::Expr {
    a::Expr {
        kind: a::ExprKind::Literal(a::Literal::Integer(value.to_string())),
        span,
    }
}
pub(crate) fn ast_type(ty: &Type, span: Span) -> a::Type {
    let name = match ty {
        Type::Unit => {
            return a::Type {
                kind: a::TypeKind::Tuple(vec![]),
                span,
            }
        }
        Type::Tuple(fields) => {
            return a::Type {
                kind: a::TypeKind::Tuple(fields.iter().map(|t| ast_type(t, span)).collect()),
                span,
            }
        }
        Type::Array(element, count) => {
            return a::Type {
                kind: a::TypeKind::Array {
                    element: Box::new(ast_type(element, span)),
                    length: Box::new(number(*count, span)),
                },
                span,
            }
        }
        Type::Record { name, .. } | Type::Enum { name, .. } => name.clone(),
        Type::Integer(i) => format!("{i:?}").to_lowercase(),
        other => format!("{other:?}").to_lowercase(),
    };
    a::Type {
        kind: a::TypeKind::Named {
            path: vec![name],
            arguments: vec![],
        },
        span,
    }
}
