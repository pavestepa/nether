//! Definite field initialization before a constructor exposes its whole receiver.
//! The AST check is retained for nominal-registry validation. Source compilation
//! validates declarations here and uses typed MIR field analysis in classes_init.
use super::*;
use std::collections::BTreeSet;
#[derive(Clone)]
struct State {
    initialized: BTreeSet<String>,
    locals: BTreeSet<String>,
    reachable: bool,
}
struct Constructor {
    fields: BTreeSet<String>,
    methods: BTreeSet<String>,
}
pub(super) fn declarations(members: &[a::Member]) -> Result<()> {
    let mut seen = false;
    for member in members {
        if let a::Member::Constructor {
            parameters, span, ..
        } = member
        {
            if seen {
                return Err(error(*span, "E0393", "duplicate constructor"));
            }
            seen = true;
            if let Some(p) = parameters.iter().find(|p| p.receiver) {
                return Err(error(p.span, "E0393", "constructor receiver is implicit"));
            }
        }
    }
    Ok(())
}
pub(super) fn validate(members: &[a::Member]) -> Result<()> {
    let fields = members
        .iter()
        .filter_map(|m| match m {
            a::Member::Field { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();
    let methods = members
        .iter()
        .filter_map(|m| match m {
            a::Member::Method { function, .. }
                if function.parameters.first().is_some_and(|p| p.receiver) =>
            {
                Some(function.name.clone())
            }
            _ => None,
        })
        .collect();
    let checker = Constructor { fields, methods };
    let mut seen = false;
    for member in members {
        if let a::Member::Constructor {
            parameters,
            body,
            span,
        } = member
        {
            if seen {
                return Err(error(*span, "E0393", "duplicate constructor"));
            }
            seen = true;
            let mut state = State {
                initialized: BTreeSet::new(),
                locals: BTreeSet::new(),
                reachable: true,
            };
            for parameter in parameters {
                if parameter.receiver {
                    return Err(error(
                        parameter.span,
                        "E0393",
                        "constructor receiver is implicit",
                    ));
                }
                bindings(&parameter.pattern, &mut state.locals);
            }
            checker.block(body, &mut state)?;
            if state.reachable {
                checker.complete(&state, *span)?;
            }
        }
    }
    Ok(())
}
impl Constructor {
    fn complete(&self, state: &State, span: Span) -> Result<()> {
        let missing = self
            .fields
            .difference(&state.initialized)
            .cloned()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(error(
                span,
                "E0393",
                format!(
                    "constructor target is not fully initialized; missing fields: {}",
                    missing.join(", ")
                ),
            ));
        }
        Ok(())
    }
    fn field<'a>(&self, value: &'a a::Expr, state: &State) -> Option<&'a str> {
        match &value.kind {
            a::ExprKind::Group(v) => self.field(v, state),
            a::ExprKind::Name(name)
                if self.fields.contains(name) && !state.locals.contains(name) =>
            {
                Some(name)
            }
            a::ExprKind::Member { value, name } if is_this(value) => {
                self.fields.contains(name).then_some(name)
            }
            _ => None,
        }
    }
    fn block(&self, block: &a::Block, state: &mut State) -> Result<()> {
        let outer = state.locals.clone();
        for statement in &block.statements {
            if !state.reachable {
                break;
            }
            match &statement.kind {
                a::StatementKind::Binding { pattern, value, .. } => {
                    self.expr(value, state)?;
                    bindings(pattern, &mut state.locals);
                }
                a::StatementKind::Expression(value) => self.expr(value, state)?,
                a::StatementKind::Return(value) => {
                    if let Some(value) = value {
                        self.expr(value, state)?;
                    }
                    self.complete(state, statement.span)?;
                    state.reachable = false;
                }
                a::StatementKind::Break | a::StatementKind::Continue => state.reachable = false,
                a::StatementKind::Unsafe(body) => self.block(body, state)?,
            }
        }
        state.locals = outer;
        Ok(())
    }
    fn expr(&self, value: &a::Expr, state: &mut State) -> Result<()> {
        if !state.reachable {
            return Ok(());
        }
        if let Some(field) = self.field(value, state) {
            if !state.initialized.contains(field) {
                return Err(error(
                    value.span,
                    "E0393",
                    format!("constructor reads uninitialized field '{field}'"),
                ));
            }
            return Ok(());
        }
        match &value.kind {
            a::ExprKind::Name(name)
                if name == "this"
                    || (self.methods.contains(name) && !state.locals.contains(name)) =>
            {
                self.complete(state, value.span)?
            }
            a::ExprKind::Assign {
                operator,
                place,
                value: rhs,
            } => {
                if let Some(field) = self.field(place, state) {
                    if *operator != "=" {
                        self.expr(place, state)?;
                    }
                    self.expr(rhs, state)?;
                    if state.reachable {
                        state.initialized.insert(field.into());
                    }
                } else {
                    self.expr(place, state)?;
                    self.expr(rhs, state)?;
                }
            }
            a::ExprKind::Group(v)
            | a::ExprKind::Unary { value: v, .. }
            | a::ExprKind::Cast { value: v, .. }
            | a::ExprKind::Specialize { value: v, .. } => self.expr(v, state)?,
            a::ExprKind::Member { value: v, .. } => self.expr(v, state)?,
            a::ExprKind::Index { value: v, index } => {
                self.expr(v, state)?;
                self.expr(index, state)?;
            }
            a::ExprKind::Binary {
                operator,
                left,
                right,
            } => {
                self.expr(left, state)?;
                if *operator == "&&" || *operator == "||" {
                    let mut conditional = state.clone();
                    self.expr(right, &mut conditional)?;
                    // The right operand is not necessarily evaluated.
                } else {
                    self.expr(right, state)?;
                }
            }
            a::ExprKind::Tuple(values)
            | a::ExprKind::Array(values)
            | a::ExprKind::New {
                arguments: values, ..
            } => {
                for value in values {
                    self.expr(value, state)?;
                }
            }
            a::ExprKind::Construct { fields, .. } => {
                for (_, value) in fields {
                    self.expr(value, state)?;
                }
            }
            a::ExprKind::Call {
                callee, arguments, ..
            } => {
                if let a::ExprKind::Name(name) = &callee.kind {
                    if self.methods.contains(name) && !state.locals.contains(name) {
                        self.complete(state, callee.span)?;
                    }
                }
                self.expr(callee, state)?;
                for value in arguments {
                    self.expr(value, state)?;
                }
            }
            a::ExprKind::If {
                condition,
                then_block,
                else_branch,
            } => {
                self.expr(condition, state)?;
                let mut yes = state.clone();
                self.block(then_block, &mut yes)?;
                let mut no = state.clone();
                if let Some(other) = else_branch {
                    self.expr(other, &mut no)?;
                }
                join(state, yes, no);
            }
            a::ExprKind::While { condition, body } => {
                self.expr(condition, state)?;
                let mut iteration = state.clone();
                self.block(body, &mut iteration)?;
                // A loop may execute zero times, and break paths do not prove
                // initialization on all exits. Typed CFG checking can refine it.
            }
            a::ExprKind::Branch(body) => self.block(body, state)?,
            a::ExprKind::Match {
                value: scrutinee,
                arms,
            } => {
                self.expr(scrutinee, state)?;
                let before = state.clone();
                let mut joined: Option<State> = None;
                let mut exhaustive = false;
                for arm in arms {
                    let mut branch = before.clone();
                    bindings(&arm.pattern, &mut branch.locals);
                    if let Some(guard) = &arm.guard {
                        self.expr(guard, &mut branch)?;
                    }
                    self.expr(&arm.value, &mut branch)?;
                    branch.locals = before.locals.clone();
                    exhaustive |= arm.guard.is_none()
                        && matches!(
                            arm.pattern.kind,
                            a::PatternKind::Wildcard | a::PatternKind::Binding { .. }
                        );
                    if let Some(previous) = joined.take() {
                        let mut result = before.clone();
                        join(&mut result, previous, branch);
                        joined = Some(result);
                    } else {
                        joined = Some(branch);
                    }
                }
                if let Some(after) = joined {
                    if exhaustive {
                        *state = after;
                    } else {
                        join(state, before, after);
                    }
                }
            }
            a::ExprKind::Name(_) | a::ExprKind::Literal(_) => {}
        }
        Ok(())
    }
}
fn is_this(value: &a::Expr) -> bool {
    match &value.kind {
        a::ExprKind::Name(name) => name == "this",
        a::ExprKind::Group(v) => is_this(v),
        _ => false,
    }
}
fn join(target: &mut State, a: State, b: State) {
    match (a.reachable, b.reachable) {
        (true, true) => {
            target.initialized = a
                .initialized
                .intersection(&b.initialized)
                .cloned()
                .collect();
            target.reachable = true;
        }
        (true, false) => *target = a,
        (false, true) => *target = b,
        (false, false) => target.reachable = false,
    }
}
fn bindings(pattern: &a::Pattern, names: &mut BTreeSet<String>) {
    match &pattern.kind {
        a::PatternKind::Binding { name, .. } => {
            names.insert(name.clone());
        }
        a::PatternKind::Tuple(fields) | a::PatternKind::Array(fields) => {
            for p in fields {
                bindings(p, names);
            }
        }
        a::PatternKind::Variant { fields, .. } => match fields {
            a::PatternFields::Tuple(fields) => {
                for p in fields {
                    bindings(p, names);
                }
            }
            a::PatternFields::Named(fields, _) => {
                for (_, p) in fields {
                    bindings(p, names);
                }
            }
            _ => {}
        },
        _ => {}
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn check(body: &str) -> Result<()> {
        let source=format!("class C {{a:i32;b:i32;fn use_all(this):i32 {{return a+b}};constructor(a:i32,flag:bool) {{{body}}}}}");
        let parsed = nether_frontend::parser::parse(nether_frontend::source::SourceId(0), &source);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let a::ItemKind::Aggregate { members, .. } = &parsed.module.unwrap().items[0].kind else {
            panic!()
        };
        validate(members)
    }
    #[test]
    fn all_reachable_exits_require_initialized_fields() {
        for body in [
            "this.a=a;b=2",
            "if flag {this.a=a} else {this.a=3};b=2",
            "this.a=a;b=this.a;use_all()",
            "if flag {this.a=1;b=2;return};this.a=3;b=4",
        ] {
            check(body).unwrap_or_else(|e| panic!("{body}: {e:?}"));
        }
        for body in [
            "a=a;b=2",
            "if flag {this.a=1};b=2",
            "this.a=b;b=2",
            "this.a=1;use_all();b=2",
            "this.a=1;let alias=this;b=2",
            "this.a=1;return",
            "while flag {this.a=1};b=2",
            "this.a+=1;b=2",
        ] {
            assert!(check(body).is_err(), "accepted {body}");
        }
    }
}
