//! Consuming-parameter inference (NMM §11.1, §20.3).
//!
//! A non-Copy ordinary parameter is a borrow unless the body passes the
//! parameter root itself to an owning position: an aggregate or enum payload,
//! a stored assignment, or a consuming parameter of a callee. Such a parameter
//! gets an owned entry and callers must supply an owner token. Aliases of the
//! parameter and `return param` are not consumption.
use super::*;
use crate::mir::loans::Access;
use std::collections::BTreeSet;

pub(super) fn base_name(name: &str) -> &str {
    name.split('$').next().unwrap_or(name)
}

fn root(value: &h::Expr, names: &BTreeMap<usize, usize>) -> Option<usize> {
    match &value.kind {
        h::ExprKind::View(id) => Some(names.get(id).copied().unwrap_or(*id)),
        h::ExprKind::ArgumentView { place, .. } | h::ExprKind::Take(place) => root(place, names),
        _ => None,
    }
}

struct Walk<'a> {
    names: &'a BTreeMap<usize, usize>,
    table: &'a BTreeMap<String, Vec<bool>>,
    bodies: &'a BTreeMap<usize, a::Function>,
    consumed: BTreeSet<usize>,
}

impl Walk<'_> {
    fn sink(&mut self, value: &h::Expr) {
        if let Some(id) = root(value, self.names) {
            self.consumed.insert(id);
        }
    }

    fn call(&mut self, callee: usize, arguments: &[h::Expr]) {
        let Some(flags) = self
            .bodies
            .get(&callee)
            .and_then(|d| self.table.get(base_name(&d.name)))
        else {
            return;
        };
        for (argument, consuming) in arguments.iter().zip(flags) {
            if *consuming {
                self.sink(argument);
            }
        }
    }

    fn statements(&mut self, statements: &[h::Statement]) {
        for statement in statements {
            match statement {
                h::Statement::Initialize(_, value)
                | h::Statement::Evaluate(value)
                | h::Statement::Return(Some(value)) => self.expr(value),
                _ => {}
            }
        }
    }

    fn expr(&mut self, value: &h::Expr) {
        use h::ExprKind as K;
        match &value.kind {
            K::StoreView(v) => {
                // A parameter root stored into an aggregate slot is consumed.
                self.sink(v);
                self.expr(v);
            }
            K::StoredView(v)
            | K::Take(v)
            | K::Tag(v)
            | K::Unary(_, v)
            | K::Cast(v)
            | K::Payload(v, _)
            | K::Field(v, _) => self.expr(v),
            K::ArgumentView { place, .. } | K::Borrow { place, .. } => self.expr(place),
            K::StorageCall {
                function,
                arguments,
                ..
            }
            | K::BorrowedCall {
                function,
                arguments,
                ..
            }
            | K::New {
                constructor: function,
                arguments,
            }
            | K::Call(function, arguments) => {
                self.call(*function, arguments);
                arguments.iter().for_each(|a| self.expr(a));
            }
            K::CallIndirect(callee, arguments, contract) => {
                for target in &contract.targets {
                    self.call(*target, arguments);
                }
                self.expr(callee);
                arguments.iter().for_each(|a| self.expr(a));
            }
            K::Identity(a, b) | K::Binary(_, a, b) | K::Index(a, b) => {
                self.expr(a);
                self.expr(b);
            }
            K::Block(block) => self.block(block),
            K::Enum { fields, .. } | K::Tuple(fields) | K::Array(fields) => {
                for field in fields {
                    self.sink(field);
                    self.expr(field);
                }
            }
            K::Match(arms) => {
                for arm in arms {
                    self.statements(&arm.guard_bindings);
                    self.expr(&arm.condition);
                    self.statements(&arm.bindings);
                    if let Some(guard) = &arm.guard {
                        self.expr(guard);
                    }
                    self.block(&arm.body);
                }
            }
            K::Assign {
                operator,
                place,
                value,
            } => {
                if *operator == "=" {
                    self.sink(value);
                }
                self.expr(place);
                self.expr(value);
            }
            K::If { condition, yes, no } => {
                self.expr(condition);
                self.block(yes);
                if let Some(no) = no {
                    self.block(no);
                }
            }
            K::While { condition, body } => {
                self.expr(condition);
                self.block(body);
            }
            K::View(_) | K::Local(_) | K::Value(_) => {}
        }
    }

    fn block(&mut self, block: &h::Block) {
        self.statements(&block.statements);
    }
}

/// Record parameters of a checked body that are consumed, growing the shared
/// table monotonically. A `var` parameter cannot be consumed (NMM §11.2).
pub(super) fn record(
    function: &h::Function,
    declaration: &a::Function,
    mono: &std::cell::RefCell<generics::Monomorphization>,
    names: &BTreeMap<usize, usize>,
) -> Result<()> {
    let consuming = mono.borrow().consuming.clone();
    let table = consuming.borrow().clone();
    let consumed = {
        let mono = mono.borrow();
        let mut walk = Walk {
            names,
            table: &table,
            bodies: &mono.bodies,
            consumed: BTreeSet::new(),
        };
        walk.block(&function.body);
        walk.consumed
    };
    let name = base_name(&declaration.name).to_owned();
    for (index, &local) in function.parameters.iter().enumerate() {
        let Some(access) = function.locals[local].view else {
            continue;
        };
        if !consumed.contains(&local)
            || function.locals[local].ty.copyable()
            || declaration.parameters.get(index).is_none_or(|p| p.receiver)
        {
            continue;
        }
        if access == Access::Mutable {
            return Err(error(
                function.locals[local].span,
                "E0398",
                "a `var` parameter is a mutable borrow and cannot be consumed",
            ));
        }
        let mut flags = consuming.borrow_mut();
        let entry = flags
            .entry(name.clone())
            .or_insert_with(|| vec![false; function.parameters.len()]);
        entry[index] = true;
    }
    Ok(())
}
