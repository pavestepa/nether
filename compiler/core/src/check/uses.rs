//! Conservative source-use hint for choosing pattern move/view lowering.
use super::*;
pub(super) fn names(body: &a::Block) -> BTreeMap<String, Vec<Span>> {
    let mut result = BTreeMap::new();
    block(body, &mut result);
    result
}
fn block(body: &a::Block, names: &mut BTreeMap<String, Vec<Span>>) {
    for statement in &body.statements {
        match &statement.kind {
            a::StatementKind::Binding { value, .. }
            | a::StatementKind::Expression(value)
            | a::StatementKind::Return(Some(value)) => expr(value, names),
            a::StatementKind::Unsafe(body) => block(body, names),
            _ => (),
        }
    }
}
fn expr(value: &a::Expr, names: &mut BTreeMap<String, Vec<Span>>) {
    match &value.kind {
        a::ExprKind::Name(name) => names.entry(name.clone()).or_default().push(value.span),
        a::ExprKind::Group(value)
        | a::ExprKind::Unary { value, .. }
        | a::ExprKind::Member { value, .. }
        | a::ExprKind::Specialize { value, .. }
        | a::ExprKind::Cast { value, .. } => expr(value, names),
        a::ExprKind::Tuple(values) | a::ExprKind::Array(values) => {
            for value in values {
                expr(value, names);
            }
        }
        a::ExprKind::Construct { fields, .. } => {
            for (_, value) in fields {
                expr(value, names);
            }
        }
        a::ExprKind::New { arguments, .. } => {
            for value in arguments {
                expr(value, names);
            }
        }
        a::ExprKind::Call {
            callee, arguments, ..
        } => {
            expr(callee, names);
            for value in arguments {
                expr(value, names);
            }
        }
        a::ExprKind::Binary { left, right, .. }
        | a::ExprKind::Assign {
            place: left,
            value: right,
            ..
        }
        | a::ExprKind::Index {
            value: left,
            index: right,
        } => {
            expr(left, names);
            expr(right, names);
        }
        a::ExprKind::If {
            condition,
            then_block,
            else_branch,
        } => {
            expr(condition, names);
            block(then_block, names);
            if let Some(value) = else_branch {
                expr(value, names);
            }
        }
        a::ExprKind::While { condition, body } => {
            expr(condition, names);
            block(body, names);
        }
        a::ExprKind::Branch(body) => block(body, names),
        a::ExprKind::Match { value, arms } => {
            expr(value, names);
            for arm in arms {
                if let Some(value) = &arm.guard {
                    expr(value, names);
                }
                expr(&arm.value, names);
            }
        }
        a::ExprKind::Literal(_) => (),
    }
}
pub(super) fn binds(pattern: &a::Pattern) -> bool {
    match &pattern.kind {
        a::PatternKind::Binding { .. } => true,
        a::PatternKind::Tuple(fields) | a::PatternKind::Array(fields) => fields.iter().any(binds),
        a::PatternKind::Variant { fields, .. } => match fields {
            a::PatternFields::Tuple(fields) => fields.iter().any(binds),
            a::PatternFields::Named(fields, _) => fields.iter().any(|(_, p)| binds(p)),
            _ => false,
        },
        _ => false,
    }
}
impl Checker<'_> {
    pub(super) fn needs_pattern_view(&self, value: &a::Expr) -> bool {
        fn root(value: &a::Expr) -> Option<&str> {
            match &value.kind {
                a::ExprKind::Name(name) => Some(name),
                a::ExprKind::Group(value)
                | a::ExprKind::Member { value, .. }
                | a::ExprKind::Index { value, .. } => root(value),
                _ => None,
            }
        }
        if let Some(name) = root(value) {
            if self.lookup(name).is_some()
                && (self.loops > 0
                    || self
                        .lookup(name)
                        .is_some_and(|id| self.locals[id].view.is_some())
                    || self.value_uses.get(name).is_some_and(|uses| {
                        uses.iter().any(|span| {
                            span.source == value.span.source && span.start >= value.span.end
                        })
                    }))
            {
                return true;
            }
        }
        false
    }
}
