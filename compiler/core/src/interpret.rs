//! Bounded reference execution for the currently checked value subset.
//! This is an oracle, not the native runtime or a substitute for LLVM lowering.
use crate::hir::*;
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
}
#[derive(Clone, Debug, PartialEq)]
pub struct Trap {
    pub kind: TrapKind,
    pub span: Span,
}
#[derive(Debug)]
enum Signal {
    Return(Value),
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
    let mut engine = Engine {
        program,
        checks,
        fuel,
        depth: 0,
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
    match engine.call(
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
    }
}

struct Engine<'a> {
    program: &'a Program,
    checks: OverflowChecks,
    fuel: u64,
    depth: usize,
}
struct Frame {
    views: std::collections::BTreeMap<usize, Place>,
    locals: Vec<Slot>,
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
            views: Default::default(),
            locals: (0..function.locals.len()).map(|_| memory::slot()).collect(),
        };
        for (id, value) in function.parameters.iter().zip(arguments) {
            match value {
                Argument::Owned(value) => *frame.locals[*id].borrow_mut() = Some(value),
                Argument::Borrowed(place) => {
                    frame.views.insert(*id, place);
                }
            }
        }
        self.depth += 1;
        let result = self.block(&function.body, &mut frame);
        self.depth -= 1;
        for local in &frame.locals {
            *local.borrow_mut() = None;
        }
        match result {
            Err(Signal::Return(value)) => Ok(value),
            Ok(_) if function.result == Type::Unit => Ok(Value::Unit),
            Ok(_) => Err(trap(TrapKind::InvalidIr, span)),
            other => other,
        }
    }
    fn block(&mut self, block: &Block, frame: &mut Frame) -> Result<Value> {
        let result = self.block_body(block, frame);
        for local in &block.storage {
            *frame.locals[*local].borrow_mut() = None;
            frame.views.remove(local);
        }
        result
    }
    fn block_body(&mut self, block: &Block, frame: &mut Frame) -> Result<Value> {
        let mut result = Value::Unit;
        for statement in &block.statements {
            self.tick(block.span)?;
            result = match statement {
                Statement::Initialize(id, value) => {
                    let value = self.expr(value, frame)?;
                    *frame.locals[*id].borrow_mut() = Some(value);
                    Value::Unit
                }
                Statement::Evaluate(value) => self.expr(value, frame)?,
                Statement::Return(value) => {
                    return Err(Signal::Return(
                        value
                            .as_ref()
                            .map(|v| self.expr(v, frame))
                            .transpose()?
                            .unwrap_or(Value::Unit),
                    ))
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
        self.tick(expr.span)?;
        let span = expr.span;
        match &expr.kind {
            ExprKind::View(id) => frame
                .views
                .get(id)
                .ok_or_else(|| trap(TrapKind::InvalidIr, span))?
                .read()
                .map_err(|kind| trap(kind, span)),
            ExprKind::ArgumentView { .. } => Err(trap(TrapKind::InvalidIr, span)),
            ExprKind::Borrow { local, place } => {
                let place = self.place(place, frame)?;
                frame.views.insert(*local, place);
                Ok(Value::Unit)
            }
            ExprKind::Block(block) => self.block(block, frame),
            ExprKind::Enum { variant, fields } => {
                let mut values = Vec::new();
                for field in fields {
                    values.push(self.expr(field, frame)?);
                }
                Ok(Value::Enum {
                    variant: *variant,
                    fields: values,
                })
            }
            ExprKind::Tag(value) => {
                let Value::Enum { variant, .. } = self.expr(value, frame)? else {
                    return Err(trap(TrapKind::InvalidIr, span));
                };
                Ok(Value::Integer(Integer::from_bits(
                    nether_semantics::IntegerType::U32,
                    variant as u128,
                )))
            }
            ExprKind::Payload(value, expected) => {
                let Value::Enum { variant, fields } = self.expr(value, frame)? else {
                    return Err(trap(TrapKind::InvalidIr, span));
                };
                if variant != *expected {
                    return Err(trap(TrapKind::InvalidIr, span));
                }
                Ok(Value::Aggregate(fields))
            }
            ExprKind::Match(arms) => {
                for arm in arms {
                    if self.expr(&arm.condition, frame)? != Value::Bool(true) {
                        continue;
                    }
                    self.block(
                        &Block {
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
                            storage: vec![],
                            statements: arm.bindings.clone(),
                            result: Type::Unit,
                            diverges: false,
                            span: expr.span,
                        },
                        frame,
                    )?;
                    return self.block(&arm.body, frame);
                }
                Err(trap(TrapKind::InvalidIr, span))
            }

            ExprKind::Value(value) => Ok(value.clone()),
            ExprKind::Local(id) => frame
                .locals
                .get(*id)
                .and_then(|v| v.borrow().clone())
                .ok_or_else(|| trap(TrapKind::InvalidIr, span)),
            ExprKind::Tuple(values) | ExprKind::Array(values) => Ok(Value::Aggregate(
                values
                    .iter()
                    .map(|v| self.expr(v, frame))
                    .collect::<Result<_>>()?,
            )),
            ExprKind::Unary(operator, value) => {
                let value = self.expr(value, frame)?;
                match (*operator, value) {
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
            ExprKind::Call(id, arguments) => {
                let mut args = Vec::new();
                for value in arguments {
                    args.push(match &value.kind {
                        ExprKind::ArgumentView { place, .. } => {
                            Argument::Borrowed(self.place(place, frame)?)
                        }
                        _ => Argument::Owned(self.expr(value, frame)?),
                    });
                }
                self.call(*id, args, span)
            }
            ExprKind::Assign {
                place,
                operator,
                value,
            } => {
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
                place.write(value).map_err(|kind| trap(kind, span))?;
                Ok(Value::Unit)
            }
            ExprKind::Index(value, index) => {
                if is_place(value) {
                    let place = self.place(expr, frame)?;
                    return place.read().map_err(|kind| trap(kind, span));
                }
                let value = self.expr(value, frame)?;
                let index = index_value(self.expr(index, frame)?, span)?;
                let Value::Aggregate(values) = value else {
                    return Err(trap(TrapKind::InvalidIr, span));
                };
                values
                    .get(index)
                    .cloned()
                    .ok_or_else(|| trap(TrapKind::Bounds, span))
            }
            ExprKind::Field(value, index) => {
                let Value::Aggregate(values) = self.expr(value, frame)? else {
                    return Err(trap(TrapKind::InvalidIr, span));
                };
                values
                    .get(*index)
                    .cloned()
                    .ok_or_else(|| trap(TrapKind::Bounds, span))
            }
            ExprKind::If { condition, yes, no } => {
                let Value::Bool(condition) = self.expr(condition, frame)? else {
                    return Err(trap(TrapKind::InvalidIr, span));
                };
                if condition {
                    self.block(yes, frame)
                } else if let Some(no) = no {
                    self.block(no, frame)
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
            _ => return Err(trap(TrapKind::InvalidIr, expr.span)),
        };
        // Check bounds before evaluating the RHS; each index is evaluated once.
        let _ = place.read().map_err(|kind| trap(kind, expr.span))?;
        Ok(place)
    }
}

fn is_place(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Local(_) | ExprKind::View(_) => true,
        ExprKind::Index(base, _) | ExprKind::Field(base, _) => is_place(base),
        _ => false,
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
