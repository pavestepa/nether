//! Conservative place-sensitive future reads before definite replacement.
use super::*;
#[derive(Clone, PartialEq, Eq)]
enum Projection {
    Field(String),
    Index(Option<u128>),
}
#[derive(Clone)]
struct Place {
    name: String,
    path: Vec<Projection>,
}
fn as_place(value: &a::Expr) -> Option<Place> {
    match &value.kind {
        a::ExprKind::Name(name) => Some(Place {
            name: name.clone(),
            path: vec![],
        }),
        a::ExprKind::Group(value) => as_place(value),
        a::ExprKind::Member { value, name } => {
            let mut p = as_place(value)?;
            p.path.push(Projection::Field(name.clone()));
            Some(p)
        }
        a::ExprKind::Index { value, index } => {
            let mut p = as_place(value)?;
            let n = match &index.kind {
                a::ExprKind::Literal(a::Literal::Integer(n)) => magnitude(n, index.span).ok(),
                _ => None,
            };
            p.path.push(Projection::Index(n));
            Some(p)
        }
        _ => None,
    }
}
fn overlaps(a: &Place, b: &Place) -> bool {
    a.name == b.name
        && a.path.iter().zip(&b.path).all(|(a, b)| match (a, b) {
            (Projection::Field(a), Projection::Field(b)) => a == b,
            (Projection::Index(Some(a)), Projection::Index(Some(b))) => a == b,
            _ => true,
        })
}
fn covers(a: &Place, b: &Place) -> bool {
    a.name == b.name
        && a.path.len() <= b.path.len()
        && a.path
            .iter()
            .zip(&b.path)
            .all(|(a, b)| a == b && !matches!(a, Projection::Index(None)))
}
#[derive(Clone, Copy)]
struct Flow {
    live: bool,
    used: bool,
    stopped: bool,
}
impl Flow {
    fn join(self, other: Self) -> Self {
        Self {
            live: self.live && !self.stopped || other.live && !other.stopped,
            used: self.used || other.used,
            stopped: self.stopped && other.stopped,
        }
    }
    fn read(&mut self, value: &a::Expr, target: &Place) {
        if !self.live || self.stopped {
            return;
        }
        if let Some(p) = as_place(value) {
            if overlaps(&p, target) {
                self.used = true;
            }
            self.address(value, target);
            return;
        }
        match &value.kind {
            a::ExprKind::Assign {
                operator,
                place,
                value,
            } => {
                self.address(place, target);
                if *operator != "=" {
                    self.read(place, target);
                }
                self.read(value, target);
                if as_place(place).is_some_and(|p| covers(&p, target)) {
                    self.live = false;
                }
            }
            a::ExprKind::If {
                condition,
                then_block,
                else_branch,
            } => {
                self.read(condition, target);
                let mut yes = *self;
                yes.block(then_block, target);
                let mut no = *self;
                if let Some(no_expr) = else_branch {
                    no.read(no_expr, target);
                }
                *self = yes.join(no);
            }
            a::ExprKind::While { condition, body } => {
                self.read(condition, target);
                let mut run = *self;
                run.block(body, target);
                self.used |= run.used;
            }
            a::ExprKind::Match { value, arms } => {
                self.read(value, target);
                let mut merged = Self {
                    live: false,
                    used: self.used,
                    stopped: true,
                };
                for arm in arms {
                    let mut next = *self;
                    if let Some(g) = &arm.guard {
                        next.read(g, target);
                    }
                    next.read(&arm.value, target);
                    merged = merged.join(next);
                }
                *self = merged;
            }
            a::ExprKind::Branch(body) => self.block(body, target),
            a::ExprKind::Binary {
                operator,
                left,
                right,
            } => {
                self.read(left, target);
                if ["&&", "||"].contains(operator) {
                    let mut rhs = *self;
                    rhs.read(right, target);
                    *self = self.join(rhs);
                } else {
                    self.read(right, target);
                }
            }
            a::ExprKind::Tuple(v) | a::ExprKind::Array(v) => {
                for e in v {
                    self.read(e, target)
                }
            }
            a::ExprKind::Construct { fields, .. } => {
                for (_, e) in fields {
                    self.read(e, target)
                }
            }
            a::ExprKind::Call {
                callee, arguments, ..
            } => {
                self.read(callee, target);
                for e in arguments {
                    self.read(e, target)
                }
            }
            a::ExprKind::New { arguments, .. } => {
                for e in arguments {
                    self.read(e, target)
                }
            }
            a::ExprKind::Group(value)
            | a::ExprKind::Unary { value, .. }
            | a::ExprKind::Member { value, .. }
            | a::ExprKind::Specialize { value, .. }
            | a::ExprKind::Cast { value, .. } => self.read(value, target),
            a::ExprKind::Index { value, index } => {
                self.read(value, target);
                self.read(index, target)
            }
            _ => (),
        }
    }
    fn address(&mut self, value: &a::Expr, target: &Place) {
        match &value.kind {
            a::ExprKind::Index { value, index } => {
                self.address(value, target);
                self.read(index, target)
            }
            a::ExprKind::Group(value) | a::ExprKind::Member { value, .. } => {
                self.address(value, target)
            }
            _ => (),
        }
    }
    fn block(&mut self, body: &a::Block, target: &Place) {
        self.statements(&body.statements, target)
    }
    fn statements(&mut self, statements: &[a::Statement], target: &Place) {
        for s in statements {
            if self.stopped {
                break;
            }
            match &s.kind {
                a::StatementKind::Binding { value, .. } | a::StatementKind::Expression(value) => {
                    self.read(value, target)
                }
                a::StatementKind::Return(value) => {
                    if let Some(value) = value {
                        self.read(value, target);
                    }
                    self.stopped = true;
                }
                a::StatementKind::Unsafe(body) => self.block(body, target),
                a::StatementKind::Break | a::StatementKind::Continue => {
                    self.used |= self.live;
                    self.stopped = true;
                }
            }
        }
    }
}
impl Checker<'_> {
    pub(super) fn ownership_view_needed(&self, value: &a::Expr) -> bool {
        let Some(target) = as_place(value) else {
            return false;
        };
        if self
            .lookup(&target.name)
            .is_some_and(|id| self.locals[id].view.is_some())
        {
            return true;
        }
        if self.loops > 0 {
            return true;
        }
        // The remainder of an enclosing expression may still read the old value.
        // Treat it conservatively; the current binding's own RHS is already evaluated.
        for (body, index) in self
            .continuations
            .iter()
            .take(self.continuations.len().saturating_sub(1))
        {
            let end = body.statements[*index].span.end;
            if self.value_uses.get(&target.name).is_some_and(|uses| {
                uses.iter().any(|s| {
                    s.source == value.span.source && s.start >= value.span.end && s.start < end
                })
            }) {
                return true;
            }
        }
        let mut flow = Flow {
            live: true,
            used: false,
            stopped: false,
        };
        for (body, index) in self.continuations.iter().rev() {
            flow.statements(&body.statements[index + 1..], &target);
        }
        flow.used
    }
}
