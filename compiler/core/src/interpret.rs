//! Bounded reference execution for the currently checked value subset.
//! This is an oracle, not the native runtime or a substitute for Rust source lowering.
use crate::hir::*;
mod drop;
pub use crate::mir::interpret::DropEvent;
use crate::reference_memory::{self as memory, Argument, Reference as Place, Slot};
use nether_frontend::source::Span;
use nether_semantics::{ArithmeticError, BinaryOp, Integer, OverflowChecks};

#[derive(Clone, Debug, PartialEq)]
pub enum TrapKind {
    Arithmetic(ArithmeticError),
    Bounds,
    StepLimit,
    CallDepth,
    InvalidIr,
    Allocation,
    DoublePanic,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Trap {
    pub kind: TrapKind,
    pub span: Span,
}
#[derive(Debug)]
enum Signal {
    Return,
    Break,
    Continue,
    Trap(Trap),
}
type Result<T> = std::result::Result<T, Signal>;

pub fn execute(
    program: &Program,
    entry: FunctionId,
    arguments: Vec<Value>,
    checks: OverflowChecks,
    fuel: u64,
) -> std::result::Result<Value, Trap> {
    execute_traced(program, entry, arguments, checks, fuel).0
}
pub fn execute_traced(
    program: &Program,
    entry: FunctionId,
    arguments: Vec<Value>,
    checks: OverflowChecks,
    fuel: u64,
) -> (std::result::Result<Value, Trap>, Vec<DropEvent>) {
    let mut engine = Engine {
        program,
        checks,
        fuel,
        depth: 0,
        clock: Default::default(),
        allocations: Default::default(),
        destructors: crate::mir::drop_plan::Destructors::from_functions(&program.functions)
            .expect("checked destructors"),
        active_panic: false,
        events: Vec::new(),
    };
    let span = program
        .functions
        .get(entry)
        .map(|f| f.span)
        .unwrap_or(Span {
            source: nether_frontend::source::SourceId(0),
            start: 0,
            end: 0,
        });
    let result = match engine.call(
        entry,
        arguments.into_iter().map(Argument::Owned).collect(),
        span,
    ) {
        Ok(value) => Ok(value),
        Err(Signal::Trap(trap)) => Err(trap),
        _ => Err(Trap {
            kind: TrapKind::InvalidIr,
            span,
        }),
    };
    (result, engine.events)
}

struct Engine<'a> {
    program: &'a Program,
    checks: OverflowChecks,
    fuel: u64,
    depth: usize,
    clock: memory::Clock,
    allocations: memory::RawAllocations,
    destructors: crate::mir::drop_plan::Destructors,
    active_panic: bool,
    events: Vec<DropEvent>,
}
struct Frame {
    returned_view: bool,
    views: std::collections::BTreeMap<usize, Place>,
    locals: Vec<Slot>,
    return_staging: Option<(Value, Type)>,
    temporary_places: Vec<Slot>,
}
impl Engine<'_> {
    fn tick(&mut self, span: Span) -> Result<()> {
        if self.fuel == 0 {
            return Err(trap(TrapKind::StepLimit, span));
        }
        self.fuel -= 1;
        Ok(())
    }
    fn call(&mut self, id: FunctionId, arguments: Vec<Argument>, span: Span) -> Result<Value> {
        self.tick(span)?;
        if self.depth >= 32 {
            return Err(trap(TrapKind::CallDepth, span));
        }
        let function = self
            .program
            .functions
            .get(id)
            .ok_or_else(|| trap(TrapKind::InvalidIr, span))?;
        if function.parameters.len() != arguments.len() {
            return Err(trap(TrapKind::InvalidIr, span));
        }
        let mut frame = Frame {
            returned_view: function.result_view.is_some(),
            return_staging: None,
            temporary_places: Vec::new(),
            views: Default::default(),
            locals: function
                .locals
                .iter()
                .map(|local| memory::typed_slot(local.ty.clone(), self.clock.clone()))
                .collect(),
        };
        for (id, value) in function.parameters.iter().zip(arguments) {
            match value {
                Argument::Owned(value) => {
                    Place {
                        slot: frame.locals[*id].clone(),
                        projections: vec![],
                    }
                    .write(value)
                    .map_err(|kind| trap(kind, span))?;
                }
                Argument::Borrowed(place) => {
                    frame.views.insert(*id, place);
                }
            }
        }
        self.depth += 1;
        let result = self.block(&function.body, &mut frame);
        let parameters: Vec<_> = function
            .parameters
            .iter()
            .copied()
            .filter(|id| function.locals[*id].view.is_none())
            .collect();
        let result = self.finish_scope(result, &parameters, false, &Type::Unit, &mut frame, span);
        self.depth -= 1;
        match result {
            Err(Signal::Return) => Ok(frame
                .return_staging
                .take()
                .map(|p| p.0)
                .unwrap_or(Value::Unit)),
            Ok(_) if function.result == Type::Unit => Ok(Value::Unit),
            Ok(_) => Err(trap(TrapKind::InvalidIr, span)),
            Err(Signal::Trap(mut failure)) => {
                if failure.kind != TrapKind::DoublePanic {
                    if let Some((value, ty)) = frame.return_staging.take() {
                        let mut pending = Some(failure.clone());
                        if let Err(error) = self.drop_value(value, &ty, &mut pending, span) {
                            failure = error;
                        }
                    }
                }
                Err(Signal::Trap(failure))
            }
            other => other,
        }
    }
    fn block(&mut self, block: &Block, frame: &mut Frame) -> Result<Value> {
        self.block_mode(block, frame, false)
    }
    fn block_mode(&mut self, block: &Block, frame: &mut Frame, borrowed: bool) -> Result<Value> {
        let result = self.block_body(block, frame, borrowed);
        self.finish_scope(
            result,
            &block.storage,
            !block.construction.is_empty(),
            &block.result,
            frame,
            block.span,
        )
    }
    fn block_body(&mut self, block: &Block, frame: &mut Frame, borrowed: bool) -> Result<Value> {
        let mut result = Value::Unit;
        for (index, statement) in block.statements.iter().enumerate() {
            self.tick(block.span)?;
            result = match statement {
                Statement::Initialize(id, value) => {
                    let value = self.expr(value, frame)?;
                    Place {
                        slot: frame.locals[*id].clone(),
                        projections: vec![],
                    }
                    .write(value)
                    .map_err(|kind| trap(kind, block.span))?;
                    Value::Unit
                }
                Statement::Evaluate(value) => {
                    let evaluated = self.expr_mode(
                        value,
                        frame,
                        borrowed && index + 1 == block.statements.len(),
                    )?;
                    if index + 1 == block.statements.len() && block.result != Type::Unit {
                        evaluated
                    } else {
                        let mut pending = None;
                        self.drop_value(evaluated, &value.ty, &mut pending, value.span)
                            .map_err(Signal::Trap)?;
                        if let Some(failure) = pending {
                            return Err(Signal::Trap(failure));
                        }
                        Value::Unit
                    }
                }
                Statement::Return(value) => {
                    let evaluated = if frame.returned_view {
                        let value = value
                            .as_ref()
                            .ok_or_else(|| trap(TrapKind::InvalidIr, block.span))?;
                        self.expr_mode(value, frame, true)?
                    } else {
                        value
                            .as_ref()
                            .map(|v| self.expr(v, frame))
                            .transpose()?
                            .unwrap_or(Value::Unit)
                    };
                    frame.return_staging = Some((
                        evaluated,
                        value.as_ref().map(|v| v.ty.clone()).unwrap_or(Type::Unit),
                    ));
                    return Err(Signal::Return);
                }
                Statement::Break => return Err(Signal::Break),
                Statement::Continue => return Err(Signal::Continue),
            };
        }
        if block.result == Type::Unit {
            Ok(Value::Unit)
        } else {
            Ok(result)
        }
    }
    fn expr(&mut self, expr: &Expr, frame: &mut Frame) -> Result<Value> {
        self.expr_mode(expr, frame, false)
    }
    fn expr_mode(&mut self, expr: &Expr, frame: &mut Frame, borrowed: bool) -> Result<Value> {
        let already_panicking = self.active_panic;
        let first_temporary = frame.temporary_places.len();
        let result = self.expr_body(expr, frame, borrowed);
        let result = match result {
            Err(Signal::Trap(mut failure)) => {
                if already_panicking {
                    failure.kind = TrapKind::DoublePanic;
                }
                self.active_panic = true;
                Err(Signal::Trap(failure))
            }
            result => result,
        };
        if matches!(&result,Err(Signal::Trap(t)) if t.kind==TrapKind::DoublePanic) {
            return result;
        }
        let mut pending = match &result {
            Err(Signal::Trap(t)) => Some(t.clone()),
            _ => None,
        };
        for slot in frame.temporary_places.drain(first_temporary..).rev() {
            self.drop_place(
                Place {
                    slot: slot.clone(),
                    projections: vec![],
                },
                slot.ty(),
                &mut pending,
                expr.span,
            )
            .map_err(Signal::Trap)?;
        }
        if pending.is_some() {
            if let Ok(value) = result {
                self.drop_value(value, &expr.ty, &mut pending, expr.span)
                    .map_err(Signal::Trap)?;
            }
            Err(Signal::Trap(pending.unwrap()))
        } else {
            result
        }
    }
    fn expr_body(&mut self, expr: &Expr, frame: &mut Frame, borrowed: bool) -> Result<Value> {
        self.tick(expr.span)?;
        let span = expr.span;
        if borrowed
            && !matches!(
                expr.kind,
                ExprKind::Block(_) | ExprKind::If { .. } | ExprKind::Match(_)
            )
        {
            return Ok(Value::View(memory::ViewValue(
                self.place(expr, frame)?
                    .captured()
                    .map_err(|kind| trap(kind, span))?,
            )));
        }
        match &expr.kind {
            ExprKind::StoreView(value) => Ok(Value::View(memory::ViewValue(
                self.place(value, frame)?
                    .captured()
                    .map_err(|kind| trap(kind, span))?,
            ))),
            ExprKind::StoredView(_) => {
                let place = self.place(expr, frame)?;
                let value = place.read().map_err(|kind| trap(kind, span))?;
                if expr.ty.copyable() {
                    memory::copy_value(&value, self.clock.clone()).map_err(|kind| trap(kind, span))
                } else {
                    Err(trap(TrapKind::InvalidIr, span))
                }
            }
            ExprKind::Identity(left, right) => {
                let Value::Class(left) = self
                    .place(left, frame)?
                    .read()
                    .map_err(|kind| trap(kind, span))?
                else {
                    return Err(trap(TrapKind::InvalidIr, span));
                };
                let Value::Class(right) = self
                    .place(right, frame)?
                    .read()
                    .map_err(|kind| trap(kind, span))?
                else {
                    return Err(trap(TrapKind::InvalidIr, span));
                };
                Ok(Value::Bool(left.same_identity(&right)))
            }
            ExprKind::Take(value) => {
                let place = self.place(value, frame)?;
                let value = place.read().map_err(|kind| trap(kind, span))?;
                place.forget().map_err(|kind| trap(kind, span))?;
                Ok(value)
            }
            ExprKind::View(_)
            | ExprKind::Local(_)
            | ExprKind::Field(..)
            | ExprKind::Index(..)
            | ExprKind::Payload(..) => {
                let place = self.place(expr, frame)?;
                let value = place.read().map_err(|kind| trap(kind, span))?;
                if !expr.ty.copyable() {
                    place.forget().map_err(|kind| trap(kind, span))?;
                    Ok(value)
                } else {
                    memory::copy_value(&value, self.clock.clone()).map_err(|kind| trap(kind, span))
                }
            }
            ExprKind::ArgumentView { .. } => Err(trap(TrapKind::InvalidIr, span)),
            ExprKind::Borrow { local, place } => {
                let place = self.place(place, frame)?;
                frame
                    .views
                    .insert(*local, place.captured().map_err(|kind| trap(kind, span))?);
                Ok(Value::Unit)
            }
            ExprKind::Block(block) => self.block_mode(block, frame, borrowed),
            ExprKind::Enum { variant, fields } => Ok(Value::Enum {
                variant: *variant,
                fields: self.aggregate(fields, frame, span)?,
            }),
            ExprKind::Tag(value) => {
                let Value::Enum { variant, .. } = self
                    .place(value, frame)?
                    .read()
                    .map_err(|kind| trap(kind, span))?
                else {
                    return Err(trap(TrapKind::InvalidIr, span));
                };
                Ok(Value::Integer(Integer::from_bits(
                    nether_semantics::IntegerType::U32,
                    variant as u128,
                )))
            }
            ExprKind::Match(arms) => {
                for arm in arms {
                    if self.expr(&arm.condition, frame)? != Value::Bool(true) {
                        continue;
                    }
                    self.block(
                        &Block {
                            construction: vec![],
                            storage: vec![],
                            statements: arm.guard_bindings.clone(),
                            result: Type::Unit,
                            diverges: false,
                            span,
                        },
                        frame,
                    )?;
                    if let Some(guard) = &arm.guard {
                        if self.expr(guard, frame)? != Value::Bool(true) {
                            continue;
                        }
                    }
                    self.block(
                        &Block {
                            construction: vec![],
                            storage: vec![],
                            statements: arm.bindings.clone(),
                            result: Type::Unit,
                            diverges: false,
                            span: expr.span,
                        },
                        frame,
                    )?;
                    return self.block_mode(&arm.body, frame, borrowed);
                }
                Err(trap(TrapKind::InvalidIr, span))
            }

            ExprKind::Value(value) => Ok(value.clone()),
            ExprKind::Tuple(values) | ExprKind::Array(values) => {
                Ok(Value::Aggregate(self.aggregate(values, frame, span)?))
            }
            ExprKind::Unary("storage_allocate", count) => {
                let Value::Integer(count) = self.expr(count, frame)? else {
                    return Err(trap(TrapKind::InvalidIr, span));
                };
                let Type::Pointer { pointee, .. } = &expr.ty else {
                    return Err(trap(TrapKind::InvalidIr, span));
                };
                self.allocations
                    .allocate(pointee, count.bits() as u64)
                    .map_err(|kind| trap(kind, span))
            }
            ExprKind::Unary(
                op @ ("storage_initialize" | "storage_replace" | "storage_take" | "storage_drop"
                | "storage_release"),
                args,
            ) => {
                let args = self.expr(args, frame)?;
                self.allocations
                    .operation(op, args)
                    .map_err(|kind| trap(kind, span))
            }
            ExprKind::Unary("raw_addr" | "raw_addr_mut", value) => {
                memory::RawPointer::new(self.place(value, frame)?)
                    .map(Value::Pointer)
                    .map_err(|kind| trap(kind, span))
            }
            ExprKind::Unary(operator, value) => {
                let value = self.expr(value, frame)?;
                match (*operator, value) {
                    ("raw_read", Value::Pointer(p)) => p.read().map_err(|kind| trap(kind, span)),
                    ("-", Value::Integer(value)) => value
                        .negate(self.checks)
                        .map(Value::Integer)
                        .map_err(|e| trap(TrapKind::Arithmetic(e), span)),
                    ("!", Value::Integer(value)) => Ok(Value::Integer(value.bit_not())),
                    ("!", Value::Bool(value)) => Ok(Value::Bool(!value)),
                    ("-", Value::F32(value)) => Ok(Value::F32(-value)),
                    ("-", Value::F64(value)) => Ok(Value::F64(-value)),
                    _ => Err(trap(TrapKind::InvalidIr, span)),
                }
            }
            ExprKind::Binary(operator, left, right) => {
                let left = self.expr(left, frame)?;
                if (*operator == "&&" && left == Value::Bool(false))
                    || (*operator == "||" && left == Value::Bool(true))
                {
                    return Ok(left);
                }
                let right = self.expr(right, frame)?;
                binary(operator, left, right, self.checks).map_err(|kind| trap(kind, span))
            }
            ExprKind::Cast(value) => cast(self.expr(value, frame)?, &expr.ty)
                .ok_or_else(|| trap(TrapKind::InvalidIr, span)),
            ExprKind::StorageCall { .. }
            | ExprKind::BorrowedCall { .. }
            | ExprKind::New { .. }
            | ExprKind::Call(..)
            | ExprKind::CallIndirect(..) => {
                let (id, arguments) = match &expr.kind {
                    ExprKind::Call(id, arguments) => (*id, arguments),
                    ExprKind::StorageCall {
                        function,
                        arguments,
                        ..
                    }
                    | ExprKind::BorrowedCall {
                        function,
                        arguments,
                        ..
                    } => (*function, arguments),
                    ExprKind::New {
                        constructor,
                        arguments,
                    } => (*constructor, arguments),
                    ExprKind::CallIndirect(callee, arguments, _) => {
                        let Value::Function(id) = self.expr(callee, frame)? else {
                            return Err(trap(TrapKind::InvalidIr, span));
                        };
                        (id, arguments)
                    }
                    _ => unreachable!(),
                };
                let mut args = Vec::new();
                for (index, value) in arguments.iter().enumerate() {
                    let argument = match &value.kind {
                        ExprKind::ArgumentView { place, .. } => self
                            .place(place, frame)
                            .and_then(|p| p.captured().map_err(|kind| trap(kind, span)))
                            .map(Argument::Borrowed),
                        _ => self.expr(value, frame).map(Argument::Owned),
                    };
                    match argument {
                        Ok(argument) => args.push(argument),
                        Err(signal) => {
                            if matches!(&signal,Signal::Trap(t) if t.kind==TrapKind::DoublePanic) {
                                return Err(signal);
                            }
                            let mut pending = match &signal {
                                Signal::Trap(t) => Some(t.clone()),
                                _ => None,
                            };
                            for (argument, expression) in
                                args.into_iter().zip(&arguments[..index]).rev()
                            {
                                if let Argument::Owned(value) = argument {
                                    self.drop_value(value, &expression.ty, &mut pending, span)
                                        .map_err(Signal::Trap)?;
                                }
                            }
                            return Err(pending.map(Signal::Trap).unwrap_or(signal));
                        }
                    }
                }
                if matches!(&expr.kind, ExprKind::New { .. }) {
                    let name = &expr.ty.class_key();
                    let object = memory::allocate_class(
                        name,
                        &self.program.classes[name],
                        self.clock.clone(),
                    );
                    args.insert(
                        0,
                        Argument::Borrowed(memory::class_reference(
                            object.clone(),
                            self.clock.clone(),
                        )),
                    );
                    match self.call(id, args, span) {
                        Ok(_) => {
                            let value = Value::Class(object);
                            memory::complete_class(&value).map_err(|kind| trap(kind, span))?;
                            Ok(value)
                        }
                        Err(Signal::Trap(error)) => {
                            if error.kind == TrapKind::DoublePanic {
                                return Err(Signal::Trap(error));
                            }
                            let mut pending = Some(error);
                            self.drop_value(Value::Class(object), &expr.ty, &mut pending, span)
                                .map_err(Signal::Trap)?;
                            Err(Signal::Trap(pending.unwrap()))
                        }
                        Err(signal) => Err(signal),
                    }
                } else {
                    self.call(id, args, span)
                }
            }
            ExprKind::Assign {
                place,
                operator,
                value,
            } => {
                let ty = place.ty.clone();
                let place = self.place(place, frame)?;
                let old = if *operator != "=" {
                    Some(place.read().map_err(|kind| trap(kind, span))?)
                } else {
                    None
                };
                let value = self.expr(value, frame)?;
                let value = if let Some(old) = old {
                    binary(operator.trim_end_matches('='), old, value, self.checks)
                        .map_err(|kind| trap(kind, span))?
                } else {
                    value
                };
                let mut pending = None;
                self.drop_place(place.clone(), &ty, &mut pending, span)
                    .map_err(Signal::Trap)?;
                if pending.is_some() {
                    self.drop_value(value, &ty, &mut pending, span)
                        .map_err(Signal::Trap)?;
                    return Err(Signal::Trap(pending.unwrap()));
                }
                place.write(value).map_err(|kind| trap(kind, span))?;
                Ok(Value::Unit)
            }
            ExprKind::If { condition, yes, no } => {
                let Value::Bool(condition) = self.expr(condition, frame)? else {
                    return Err(trap(TrapKind::InvalidIr, span));
                };
                if condition {
                    self.block_mode(yes, frame, borrowed)
                } else if let Some(no) = no {
                    self.block_mode(no, frame, borrowed)
                } else {
                    Ok(Value::Unit)
                }
            }
            ExprKind::While { condition, body } => {
                loop {
                    self.tick(span)?;
                    let Value::Bool(condition) = self.expr(condition, frame)? else {
                        return Err(trap(TrapKind::InvalidIr, span));
                    };
                    if !condition {
                        break;
                    }
                    match self.block(body, frame) {
                        Ok(_) | Err(Signal::Continue) => {}
                        Err(Signal::Break) => break,
                        other => return other,
                    }
                }
                Ok(Value::Unit)
            }
        }
    }
    fn place(&mut self, expr: &Expr, frame: &mut Frame) -> Result<Place> {
        let place = match &expr.kind {
            ExprKind::StoredView(storage) => {
                let Value::View(value) = self.expr(storage, frame)? else {
                    return Err(trap(TrapKind::InvalidIr, expr.span));
                };
                value.0
            }
            ExprKind::BorrowedCall { .. }
            | ExprKind::CallIndirect(_, _, crate::hir::CallContract { view: Some(_), .. }) => {
                let Value::View(value) = self.expr(expr, frame)? else {
                    return Err(trap(TrapKind::InvalidIr, expr.span));
                };
                value.0
            }
            ExprKind::View(id) => frame
                .views
                .get(id)
                .cloned()
                .ok_or_else(|| trap(TrapKind::InvalidIr, expr.span))?,
            ExprKind::Local(local) => Place {
                slot: frame.locals[*local].clone(),
                projections: Vec::new(),
            },
            ExprKind::Index(base, index) => {
                let mut place = self.place(base, frame)?;
                let index = index_value(self.expr(index, frame)?, expr.span)?;
                place.projections.push(crate::mir::Projection::Field(index));
                place
            }
            ExprKind::Payload(base, variant) => {
                let mut place = self.place(base, frame)?;
                place
                    .projections
                    .push(crate::mir::Projection::Variant(*variant));
                return Ok(place);
            }
            ExprKind::Field(base, index) => {
                let mut place = self.place(base, frame)?;
                place
                    .projections
                    .push(crate::mir::Projection::Field(*index));
                place
            }
            _ => {
                let value = self.expr(expr, frame)?;
                let slot = memory::typed_slot(expr.ty.clone(), self.clock.clone());
                let place = Place {
                    slot: slot.clone(),
                    projections: vec![],
                };
                place.write(value).map_err(|kind| trap(kind, expr.span))?;
                frame.temporary_places.push(slot);
                place
            }
        };
        let place = place.normalized().map_err(|kind| trap(kind, expr.span))?;
        // Check bounds before evaluating the RHS; each index is evaluated once.
        let _ = place.read().map_err(|kind| trap(kind, expr.span))?;
        Ok(place)
    }
}

fn trap(kind: TrapKind, span: Span) -> Signal {
    Signal::Trap(Trap { kind, span })
}
fn index_value(value: Value, span: Span) -> Result<usize> {
    let Value::Integer(value) = value else {
        return Err(trap(TrapKind::InvalidIr, span));
    };
    usize::try_from(value.bits()).map_err(|_| trap(TrapKind::Bounds, span))
}

pub fn binary(
    operator: &str,
    left: Value,
    right: Value,
    checks: OverflowChecks,
) -> std::result::Result<Value, TrapKind> {
    match (operator, &left, &right) {
        ("raw_write", Value::Pointer(p), value) => return p.write(value.clone()),
        ("raw_offset", Value::Pointer(p), Value::Integer(n)) => {
            return p
                .clone()
                .offset(n.signed_value().ok_or(TrapKind::InvalidIr)?)
                .map(Value::Pointer)
        }
        _ => (),
    }
    if let (Value::Integer(a), Value::Integer(b)) = (&left, &right) {
        let operation = match operator {
            "+" => Some(BinaryOp::Add),
            "-" => Some(BinaryOp::Subtract),
            "*" => Some(BinaryOp::Multiply),
            "/" => Some(BinaryOp::Divide),
            "%" => Some(BinaryOp::Remainder),
            "&" => Some(BinaryOp::BitAnd),
            "|" => Some(BinaryOp::BitOr),
            "^" => Some(BinaryOp::BitXor),
            "<<" => Some(BinaryOp::ShiftLeft),
            ">>" => Some(BinaryOp::ShiftRight),
            _ => None,
        };
        if let Some(operation) = operation {
            return a
                .binary(operation, *b, checks)
                .map(Value::Integer)
                .map_err(TrapKind::Arithmetic);
        }
        return comparison(operator, Some(a.compare(*b).map_err(TrapKind::Arithmetic)?))
            .map(Value::Bool)
            .ok_or(TrapKind::InvalidIr);
    }
    macro_rules! float {
        ($a:expr,$b:expr,$variant:ident) => {{
            match operator {
                "+" => Ok(Value::$variant($a + $b)),
                "-" => Ok(Value::$variant($a - $b)),
                "*" => Ok(Value::$variant($a * $b)),
                "/" => Ok(Value::$variant($a / $b)),
                "%" => Ok(Value::$variant($a % $b)),
                _ => comparison(operator, $a.partial_cmp(&$b))
                    .map(Value::Bool)
                    .ok_or(TrapKind::InvalidIr),
            }
        }};
    }
    match (left, right) {
        (Value::F32(a), Value::F32(b)) => float!(a, b, F32),
        (Value::F64(a), Value::F64(b)) => float!(a, b, F64),
        (Value::Bool(a), Value::Bool(b)) => match operator {
            "&&" => Ok(Value::Bool(a && b)),
            "||" => Ok(Value::Bool(a || b)),
            _ => comparison(operator, Some(a.cmp(&b)))
                .map(Value::Bool)
                .ok_or(TrapKind::InvalidIr),
        },
        (Value::Char(a), Value::Char(b)) => comparison(operator, Some(a.cmp(&b)))
            .map(Value::Bool)
            .ok_or(TrapKind::InvalidIr),
        _ => Err(TrapKind::InvalidIr),
    }
}
fn comparison(operator: &str, ordering: Option<std::cmp::Ordering>) -> Option<bool> {
    use std::cmp::Ordering::*;
    Some(match operator {
        "==" => ordering == Some(Equal),
        "!=" => ordering != Some(Equal),
        "<" => ordering == Some(Less),
        ">" => ordering == Some(Greater),
        "<=" => matches!(ordering, Some(Less | Equal)),
        ">=" => matches!(ordering, Some(Greater | Equal)),
        _ => return None,
    })
}
pub fn cast(value: Value, target: &Type) -> Option<Value> {
    match target {
        Type::Pointer { .. } => {
            if matches!(value, Value::Pointer(_)) {
                Some(value)
            } else {
                None
            }
        }
        Type::Integer(ty) => Some(Value::Integer(match value {
            Value::Integer(value) => value.cast(*ty),
            Value::F32(value) => Integer::from_float(*ty, f64::from(value)),
            Value::F64(value) => Integer::from_float(*ty, value),
            Value::Bool(value) => Integer::from_bits(*ty, u128::from(value)),
            Value::Char(value) => Integer::from_bits(*ty, u128::from(u32::from(value))),
            _ => return None,
        })),
        Type::F32 => Some(Value::F32(match value {
            Value::Integer(value) => {
                if let Some(signed) = value.signed_value() {
                    signed as f32
                } else {
                    value.bits() as f32
                }
            }
            Value::F32(v) => v,
            Value::F64(v) => v as f32,
            _ => return None,
        })),
        Type::F64 => Some(Value::F64(match value {
            Value::Integer(value) => {
                if let Some(signed) = value.signed_value() {
                    signed as f64
                } else {
                    value.bits() as f64
                }
            }
            Value::F32(v) => f64::from(v),
            Value::F64(v) => v,
            _ => return None,
        })),
        Type::Char => {
            if let Value::Integer(v) = value {
                if v.ty() == nether_semantics::IntegerType::U8 {
                    Some(Value::Char(char::from(v.bits() as u8)))
                } else {
                    None
                }
            } else {
                None
            }
        }
        _ => None,
    }
}
