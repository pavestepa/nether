use super::*;
use crate::hir::{Expr, ExprKind, Statement};
pub fn lower(program: &h::Program) -> Program {
    Program {
        functions: program.functions.iter().map(function).collect(),
        main: program.main,
    }
}
fn function(source: &h::Function) -> Function {
    let mut builder = Builder {
        function: Function {
            name: source.name.clone(),
            parameters: source.parameters.clone(),
            locals: source.locals.clone(),
            result: source.result.clone(),
            blocks: vec![],
            entry: 0,
            span: source.span,
        },
        current: None,
        unwind: 0,
        loops: vec![],
        scopes: vec![],
        temporary_start: source.locals.len(),
    };
    let entry = builder.block_id(source.span);
    builder.function.entry = entry;
    builder.current = Some(entry);
    builder.unwind = builder.block_id(source.span);
    builder.function.blocks[builder.unwind].terminator = Terminator::ResumePanic;
    builder.block(&source.body);
    if builder.current.is_some() {
        builder.end(Terminator::Return(Operand::Constant(Value::Unit)));
    }
    // Every failure exits the frame through an explicit storage boundary. Drop
    // elaboration can use this edge for initialized roots and argument temporaries.
    let resume = builder.block_id(source.span);
    builder.function.blocks[resume].terminator = Terminator::ResumePanic;
    builder.function.blocks[builder.unwind].terminator = Terminator::EndStorage {
        locals: (0..builder.function.locals.len()).collect(),
        next: resume,
    };
    builder.function
}
struct Builder {
    function: Function,
    current: Option<BlockId>,
    unwind: BlockId,
    loops: Vec<(BlockId, BlockId, usize, LocalId)>,
    scopes: Vec<Vec<LocalId>>,
    temporary_start: LocalId,
}
impl Builder {
    fn block_id(&mut self, span: Span) -> BlockId {
        let id = self.function.blocks.len();
        self.function.blocks.push(Block {
            terminator: Terminator::Unreachable,
            span,
        });
        id
    }
    fn end(&mut self, terminator: Terminator) {
        if let Some(id) = self.current.take() {
            self.function.blocks[id].terminator = terminator;
        }
    }
    fn temp(&mut self, ty: &Type, span: Span) -> LocalId {
        let id = self.function.locals.len();
        self.function.locals.push(h::Local {
            view: None,
            name: "<temporary>".into(),
            ty: ty.clone(),
            mutable: true,
            span,
        });
        id
    }
    fn evaluate_into(&mut self, operation: Operation, destination: Place, span: Span) {
        if self.current.is_none() {
            return;
        }
        self.function.blocks[self.current.unwrap()].span = span;
        let next = self.block_id(span);
        self.end(Terminator::Evaluate {
            operation,
            destination,
            next,
            unwind: self.unwind,
        });
        self.current = Some(next);
    }
    fn evaluate(&mut self, operation: Operation, ty: &Type, span: Span) -> Operand {
        let id = self.temp(ty, span);
        self.evaluate_into(operation, Place::local(id), span);
        Operand::Local(id)
    }
    fn check_place(&mut self, place: &Place, span: Span) {
        if self.current.is_none() {
            return;
        }
        self.function.blocks[self.current.unwrap()].span = span;
        let next = self.block_id(span);
        self.end(Terminator::CheckPlace {
            place: place.clone(),
            next,
            unwind: self.unwind,
        });
        self.current = Some(next);
    }
    fn end_storage(&mut self, from: usize, span: Span) {
        if self.current.is_none() {
            return;
        }
        let locals = self.scopes[from..]
            .iter()
            .rev()
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        self.end_locals(locals, span);
    }
    fn end_locals(&mut self, locals: Vec<LocalId>, span: Span) {
        if locals.is_empty() || self.current.is_none() {
            return;
        }
        let next = self.block_id(span);
        self.end(Terminator::EndStorage { locals, next });
        self.current = Some(next);
    }
    fn end_temporaries(&mut self, from: LocalId, keep: Option<&Operand>, span: Span) {
        let locals = (from..self.function.locals.len())
            .filter(|id| !matches!(keep, Some(Operand::Local(keep)) if keep == id))
            .collect();
        self.end_locals(locals, span);
    }
    fn block(&mut self, body: &h::Block) -> Operand {
        self.scopes.push(body.storage.clone());
        let mut result = Operand::Constant(Value::Unit);
        for (index, statement) in body.statements.iter().enumerate() {
            if self.current.is_none() {
                break;
            }
            let temporary_start = self.function.locals.len();
            result = self.statement(statement, body.span);
            let keep_result = index + 1 == body.statements.len() && body.result != Type::Unit;
            self.end_temporaries(temporary_start, keep_result.then_some(&result), body.span);
        }
        self.end_storage(self.scopes.len() - 1, body.span);
        self.scopes.pop();
        if body.result == Type::Unit {
            Operand::Constant(Value::Unit)
        } else {
            result
        }
    }
    fn statement(&mut self, statement: &Statement, span: Span) -> Operand {
        match statement {
            Statement::Initialize(id, value) => {
                let operand = self.expr(value);
                self.evaluate_into(Operation::Use(operand), Place::local(*id), value.span);
            }
            Statement::Evaluate(value) => return self.expr(value),
            Statement::Return(value) => {
                let result = value
                    .as_ref()
                    .map_or(Operand::Constant(Value::Unit), |v| self.expr(v));
                self.end_temporaries(self.temporary_start, Some(&result), span);
                self.end_storage(0, span);
                self.end(Terminator::Return(result));
            }
            Statement::Break | Statement::Continue => {
                let (test, end, depth, temporary_start) = *self.loops.last().unwrap();
                self.end_temporaries(temporary_start, None, span);
                self.end_storage(depth, span);
                self.end(Terminator::Goto(if matches!(statement, Statement::Break) {
                    end
                } else {
                    test
                }));
            }
        }
        let _ = span;
        Operand::Constant(Value::Unit)
    }
    fn place(&mut self, expr: &Expr) -> Place {
        match &expr.kind {
            ExprKind::Local(id) => Place::local(*id),
            ExprKind::View(id) => Place {
                local: *id,
                projections: vec![Projection::Dereference],
            },
            ExprKind::Field(base, index) => {
                let mut place = self.place(base);
                place.projections.push(Projection::Field(*index));
                place
            }
            ExprKind::Payload(base, variant) => {
                let mut place = self.place(base);
                place.projections.push(Projection::Variant(*variant));
                place
            }
            ExprKind::Index(base, index) => {
                let mut place = self.place(base);
                if let ExprKind::Value(Value::Integer(value)) = &index.kind {
                    place
                        .projections
                        .push(Projection::ConstantIndex(value.bits() as u64));
                    self.check_place(&place, expr.span);
                    return place;
                }
                let value = self.expr(index);
                let id = self.temp(&index.ty, index.span);
                self.evaluate_into(Operation::Use(value), Place::local(id), index.span);
                place.projections.push(Projection::Index(id));
                self.check_place(&place, expr.span);
                place
            }
            _ => {
                let value = self.expr(expr);
                let id = self.temp(&expr.ty, expr.span);
                self.evaluate_into(Operation::Use(value), Place::local(id), expr.span);
                Place::local(id)
            }
        }
    }
    fn expr(&mut self, expr: &Expr) -> Operand {
        if self.current.is_none() {
            return Operand::Constant(Value::Unit);
        }
        let operation = match &expr.kind {
            ExprKind::ArgumentView { place, access } => {
                let source = self.place(place);
                let id = self.temp(&place.ty, expr.span);
                self.function.locals[id].view = Some(*access);
                self.evaluate_into(Operation::Borrow(source), Place::local(id), expr.span);
                return Operand::Address(Place {
                    local: id,
                    projections: vec![Projection::Dereference],
                });
            }
            ExprKind::Value(value) => Operation::Use(Operand::Constant(value.clone())),
            ExprKind::Borrow { local, place } => {
                let place = self.place(place);
                self.evaluate_into(Operation::Borrow(place), Place::local(*local), expr.span);
                return Operand::Constant(Value::Unit);
            }
            ExprKind::View(_)
            | ExprKind::Local(_)
            | ExprKind::Field(_, _)
            | ExprKind::Index(_, _)
            | ExprKind::Payload(_, _) => {
                let place = self.place(expr);
                if expr.ty.copyable() {
                    Operation::Copy(place)
                } else {
                    Operation::Move(place)
                }
            }
            ExprKind::Block(body) => return self.block(body),
            ExprKind::Tuple(values) | ExprKind::Array(values) => {
                Operation::Aggregate(values.iter().map(|v| self.expr(v)).collect())
            }
            ExprKind::Enum { variant, fields } => {
                Operation::Enum(*variant, fields.iter().map(|v| self.expr(v)).collect())
            }
            ExprKind::Tag(value) => Operation::TagPlace(self.place(value)),
            ExprKind::Unary(operator, value) => Operation::Unary(operator, self.expr(value)),
            ExprKind::Cast(value) => Operation::Cast(self.expr(value), expr.ty.clone()),
            ExprKind::Binary(operator, left, right) if ["&&", "||"].contains(operator) => {
                let condition = self.expr(left);
                let rhs = self.block_id(right.span);
                let constant = self.block_id(expr.span);
                let end = self.block_id(expr.span);
                let result = self.temp(&Type::Bool, expr.span);
                let (yes, no) = if *operator == "&&" {
                    (rhs, constant)
                } else {
                    (constant, rhs)
                };
                self.end(Terminator::Branch { condition, yes, no });
                self.current = Some(rhs);
                let value = self.expr(right);
                self.evaluate_into(Operation::Use(value), Place::local(result), right.span);
                self.end(Terminator::Goto(end));
                self.current = Some(constant);
                self.evaluate_into(
                    Operation::Use(Operand::Constant(Value::Bool(*operator == "||"))),
                    Place::local(result),
                    expr.span,
                );
                self.end(Terminator::Goto(end));
                self.current = Some(end);
                return Operand::Local(result);
            }
            ExprKind::Binary(operator, left, right) => {
                let a = self.expr(left);
                let b = self.expr(right);
                Operation::Binary(operator, a, b)
            }
            ExprKind::Call(id, args) => {
                Operation::Call(*id, args.iter().map(|v| self.expr(v)).collect())
            }
            ExprKind::Assign {
                place,
                operator,
                value,
            } => {
                let mut target = self.place(place);
                if matches!(target.projections.first(), Some(Projection::Dereference)) {
                    let id = self.temp(&place.ty, place.span);
                    self.function.locals[id].view = Some(super::loans::Access::Mutable);
                    self.evaluate_into(
                        Operation::CapturePlace(target),
                        Place::local(id),
                        place.span,
                    );
                    target = Place {
                        local: id,
                        projections: vec![Projection::Dereference],
                    };
                }
                self.check_place(&target, place.span);
                let old = (*operator != "=")
                    .then(|| self.evaluate(Operation::Copy(target.clone()), &place.ty, place.span));
                let value = self.expr(value);
                let operation = if let Some(old) = old {
                    Operation::Binary(operator.trim_end_matches('='), old, value)
                } else {
                    Operation::Use(value)
                };
                self.evaluate_into(operation, target, expr.span);
                return Operand::Constant(Value::Unit);
            }
            ExprKind::If { condition, yes, no } => {
                let condition = self.expr(condition);
                let yes_id = self.block_id(yes.span);
                let no_id = self.block_id(expr.span);
                let end = self.block_id(expr.span);
                let result = self.temp(&expr.ty, expr.span);
                self.end(Terminator::Branch {
                    condition,
                    yes: yes_id,
                    no: no_id,
                });
                self.current = Some(yes_id);
                let value = self.block(yes);
                let yes_reaches = self.current.is_some();
                self.evaluate_into(Operation::Use(value), Place::local(result), expr.span);
                self.end(Terminator::Goto(end));
                self.current = Some(no_id);
                let value = no
                    .as_ref()
                    .map_or(Operand::Constant(Value::Unit), |body| self.block(body));
                let no_reaches = self.current.is_some();
                self.evaluate_into(Operation::Use(value), Place::local(result), expr.span);
                self.end(Terminator::Goto(end));
                self.current = (yes_reaches || no_reaches).then_some(end);
                return Operand::Local(result);
            }
            ExprKind::While { condition, body } => {
                let temporary_start = self.function.locals.len();
                let test = self.block_id(condition.span);
                let run = self.block_id(body.span);
                let end = self.block_id(expr.span);
                self.end(Terminator::Goto(test));
                self.current = Some(test);
                let condition = self.expr(condition);
                self.end_temporaries(temporary_start, Some(&condition), expr.span);
                self.end(Terminator::Branch {
                    condition,
                    yes: run,
                    no: end,
                });
                self.loops.push((test, end, self.scopes.len(), temporary_start));
                self.current = Some(run);
                self.block(body);
                self.end(Terminator::Goto(test));
                self.loops.pop();
                self.current = Some(end);
                return Operand::Constant(Value::Unit);
            }
            ExprKind::Match(arms) => {
                let end = self.block_id(expr.span);
                let result = self.temp(&expr.ty, expr.span);
                let mut reaches = false;
                for arm in arms {
                    let condition = self.expr(&arm.condition);
                    let body = self.block_id(arm.body.span);
                    let next = self.block_id(expr.span);
                    self.end(Terminator::Branch {
                        condition,
                        yes: body,
                        no: next,
                    });
                    self.current = Some(body);
                    for binding in &arm.guard_bindings {
                        self.statement(binding, expr.span);
                    }
                    if let Some(guard) = &arm.guard {
                        let condition = self.expr(guard);
                        let accepted = self.block_id(guard.span);
                        self.end(Terminator::Branch {
                            condition,
                            yes: accepted,
                            no: next,
                        });
                        self.current = Some(accepted);
                    }
                    for binding in &arm.bindings {
                        self.statement(binding, expr.span);
                    }
                    let value = self.block(&arm.body);
                    reaches |= self.current.is_some();
                    self.evaluate_into(Operation::Use(value), Place::local(result), expr.span);
                    self.end(Terminator::Goto(end));
                    self.current = Some(next);
                }
                self.end(Terminator::Unreachable);
                self.current = reaches.then_some(end);
                return Operand::Local(result);
            }
        };
        self.evaluate(operation, &expr.ty, expr.span)
    }
}
