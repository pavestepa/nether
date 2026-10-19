//! Reference execution of explicit CFG, independent of HIR control-flow traversal.
use super::*;
mod drop;
use crate::reference_memory::{self as memory, Argument, Reference, Slot};
type Views = std::collections::BTreeMap<usize, Reference>;
use crate::interpret::{binary, cast, Trap, TrapKind};
use nether_semantics::{Integer, IntegerType, OverflowChecks};
pub fn execute(
    program: &Program,
    entry: usize,
    arguments: Vec<Value>,
    checks: OverflowChecks,
    fuel: u64,
) -> Result<Value, Trap> {
    execute_traced(program, entry, arguments, checks, fuel).0
}
#[derive(Clone, Debug, PartialEq)]
pub struct DropEvent {
    pub entry: usize,
    pub value: Value,
}
pub fn execute_traced(
    program: &Program,
    entry: usize,
    arguments: Vec<Value>,
    checks: OverflowChecks,
    fuel: u64,
) -> (Result<Value, Trap>, Vec<DropEvent>) {
    let span = program
        .functions
        .get(entry)
        .map(|f| f.span)
        .unwrap_or(Span {
            source: nether_frontend::source::SourceId(0),
            start: 0,
            end: 0,
        });
    let mut engine = Engine {
        program,
        checks,
        fuel,
        depth: 0,
        active_panic: false,
        clock: Default::default(),
        events: Vec::new(),
    };
    let result = engine.call(
        entry,
        arguments.into_iter().map(Argument::Owned).collect(),
        span,
    );
    (result, engine.events)
}
struct Engine<'a> {
    program: &'a Program,
    checks: OverflowChecks,
    fuel: u64,
    depth: usize,
    active_panic: bool,
    clock: memory::Clock,
    events: Vec<DropEvent>,
}
impl Engine<'_> {
    fn call(&mut self, id: usize, args: Vec<Argument>, span: Span) -> Result<Value, Trap> {
        if self.depth >= 32 {
            return Err(Trap {
                kind: TrapKind::CallDepth,
                span,
            });
        }
        let Some(function) = self.program.functions.get(id) else {
            return Err(Trap {
                kind: TrapKind::InvalidIr,
                span,
            });
        };
        if args.len() != function.parameters.len() {
            return Err(Trap {
                kind: TrapKind::InvalidIr,
                span,
            });
        }
        let mut locals: Vec<Slot> = function
            .locals
            .iter()
            .map(|local| memory::typed_slot(local.ty.clone(), self.clock.clone()))
            .collect();
        let mut views = Views::new();
        for (id, value) in function.parameters.iter().zip(args) {
            match value {
                Argument::Owned(value) => {
                    *locals[*id].borrow_mut() = Some(value);
                    locals[*id]
                        .initialize()
                        .map_err(|kind| Trap { kind, span })?;
                }
                Argument::Borrowed(place) => {
                    views.insert(*id, place);
                }
            }
        }
        self.depth += 1;
        let result = self.run(function, &mut locals, &mut views);
        self.depth -= 1;
        for local in &locals {
            *local.borrow_mut() = None;
        }
        result
    }
    fn run(
        &mut self,
        function: &Function,
        locals: &mut [Slot],
        views: &mut Views,
    ) -> Result<Value, Trap> {
        let mut pc = function.entry;
        let mut pending = None;
        let mut initialization = super::initialization::State::default();
        if function.cleanup.owners.len() != function.locals.len() {
            return Err(Trap {
                kind: TrapKind::InvalidIr,
                span: function.span,
            });
        }
        let mut cleanup = super::cleanup::State::new(&function.cleanup);
        for local in &function.parameters {
            initialization.initialize(Place::local(*local));
            if function.locals[*local].view.is_none() {
                cleanup.initialize(*local);
            }
        }
        loop {
            let Some(block) = function.blocks.get(pc) else {
                return Err(Trap {
                    kind: TrapKind::InvalidIr,
                    span: function.span,
                });
            };
            let span = block.span;
            if self.fuel == 0 {
                return Err(Trap {
                    kind: TrapKind::StepLimit,
                    span,
                });
            }
            self.fuel -= 1;
            if super::dataflow::required_initialized(&block.terminator)
                .iter()
                .any(|place| {
                    initialization.status(place) != super::initialization::Status::Initialized
                })
            {
                return Err(Trap {
                    kind: TrapKind::InvalidIr,
                    span,
                });
            }
            match &block.terminator {
                Terminator::EndStorage {
                    locals: ended,
                    next,
                } => {
                    let was_panicking = pending.is_some();
                    cleanup.end(ended);
                    for (scope, mut selected) in function.cleanup.groups(ended) {
                        if function.cleanup.construction.contains(&scope) {
                            selected.sort_by_key(|id| {
                                function.cleanup.field_order.get(id).copied().unwrap_or(*id)
                            });
                        } else {
                            selected
                                .sort_by_key(|id| std::cmp::Reverse(locals[*id].initialized_at()));
                        }
                        for local in selected {
                            if function.locals[local].view.is_none() {
                                self.drop_place(
                                    Reference {
                                        slot: locals[local].clone(),
                                        projections: vec![],
                                    },
                                    &function.locals[local].ty,
                                    &mut pending,
                                    span,
                                )?;
                            }
                            *locals[local].borrow_mut() = None;
                            locals[local].state().borrow_mut().finish(&[]);
                            views.remove(&local);
                            initialization.set(
                                Place::local(local),
                                super::initialization::Status::Uninitialized,
                            );
                        }
                    }
                    pc = if !was_panicking && pending.is_some() {
                        drop::unwind_entry(function).ok_or(Trap {
                            kind: TrapKind::InvalidIr,
                            span,
                        })?
                    } else {
                        *next
                    };
                }
                Terminator::Evaluate {
                    operation,
                    destination,
                    next,
                    unwind,
                } => {
                    if let Operation::Borrow(source) | Operation::CapturePlace(source) = operation {
                        let resolved = resolve(source, views, locals).and_then(|p| {
                            p.read()?;
                            if matches!(operation, Operation::Borrow(_)) {
                                p.captured()
                            } else {
                                Ok(p)
                            }
                        });
                        match resolved {
                            Ok(place) => {
                                views.insert(destination.local, place);
                                initialization.initialize(destination.clone());
                                pc = *next;
                            }
                            Err(kind) => {
                                pending = Some(Trap { kind, span });
                                pc = *unwind;
                            }
                        }
                        continue;
                    }
                    let target_type = drop::place_type(destination, function);
                    let target =
                        resolve(destination, views, locals).map_err(|kind| Trap { kind, span })?;
                    let is_call = matches!(
                        operation,
                        Operation::Call(..)
                            | Operation::CallIndirect(..)
                            | Operation::CallView { .. }
                            | Operation::CallStorage { .. }
                    );
                    let consumed = super::dataflow::consumed(function, operation);
                    if is_call {
                        for source in &consumed {
                            resolve(source, views, locals)
                                .and_then(|p| p.forget())
                                .map_err(|kind| Trap { kind, span })?;
                            cleanup.forget(source.local);
                            initialization.move_out(source.clone()).map_err(|_| Trap {
                                kind: TrapKind::InvalidIr,
                                span,
                            })?;
                        }
                    }
                    match self.operation(operation, locals, views, span) {
                        Ok(value) => {
                            if !is_call {
                                for source in consumed {
                                    resolve(&source, views, locals)
                                        .and_then(|p| p.forget())
                                        .map_err(|kind| Trap { kind, span })?;
                                    if source.projections.is_empty() {
                                        cleanup.forget(source.local);
                                    }
                                    initialization.move_out(source).map_err(|_| Trap {
                                        kind: TrapKind::InvalidIr,
                                        span,
                                    })?;
                                }
                            }
                            if destination.projections.is_empty()
                                && function.locals[destination.local].view.is_some()
                            {
                                let Value::View(value) = value else {
                                    return Err(Trap {
                                        kind: TrapKind::InvalidIr,
                                        span,
                                    });
                                };
                                views.insert(destination.local, value.0);
                                initialization.initialize(destination.clone());
                                pc = *next;
                                continue;
                            }
                            self.drop_place(target.clone(), &target_type, &mut pending, span)?;
                            if pending.is_some() {
                                self.drop_value(value, &target_type, &mut pending, span)?;
                                pc = *unwind;
                            } else {
                                target.write(value).map_err(|kind| Trap { kind, span })?;
                                initialization.initialize(destination.clone());
                                if destination.projections.is_empty()
                                    && function.locals[destination.local].view.is_none()
                                {
                                    cleanup.initialize(destination.local);
                                }
                                pc = *next;
                            }
                        }
                        Err(trap) => {
                            let trap = if is_call { trap } else { self.new_panic(trap) };
                            if trap.kind == TrapKind::DoublePanic {
                                return Err(trap);
                            }
                            pending = Some(trap);
                            pc = *unwind;
                        }
                    }
                }
                Terminator::CheckPlace {
                    place,
                    next,
                    unwind,
                } => match resolve(place, views, locals).and_then(|p| p.read()) {
                    Ok(_) => pc = *next,
                    Err(kind) => {
                        let trap = self.new_panic(Trap { kind, span });
                        if trap.kind == TrapKind::DoublePanic {
                            return Err(trap);
                        }
                        pending = Some(trap);
                        pc = *unwind;
                    }
                },
                Terminator::Goto(next) => pc = *next,
                Terminator::Branch { condition, yes, no } => match operand(condition, locals) {
                    Ok(Value::Bool(value)) => pc = if value { *yes } else { *no },
                    _ => {
                        return Err(Trap {
                            kind: TrapKind::InvalidIr,
                            span,
                        })
                    }
                },
                Terminator::Return(Operand::Address(place)) => {
                    return resolve(place, views, locals)
                        .and_then(|p| p.captured())
                        .map(|p| Value::View(memory::ViewValue(p)))
                        .map_err(|kind| Trap { kind, span });
                }
                Terminator::Return(value) => {
                    let result = operand(value, locals).map_err(|kind| Trap { kind, span })?;
                    if let Operand::Local(local) = value {
                        if function.locals[*local].ty.owns_storage() {
                            Reference {
                                slot: locals[*local].clone(),
                                projections: vec![],
                            }
                            .forget()
                            .map_err(|kind| Trap { kind, span })?;
                        }
                    }
                    return Ok(result);
                }
                Terminator::ResumePanic => {
                    return Err(pending.unwrap_or(Trap {
                        kind: TrapKind::InvalidIr,
                        span,
                    }))
                }
                Terminator::Unreachable => {
                    return Err(Trap {
                        kind: TrapKind::InvalidIr,
                        span,
                    })
                }
            }
        }
    }
    fn operation(
        &mut self,
        operation: &Operation,
        locals: &[Slot],
        views: &Views,
        span: Span,
    ) -> Result<Value, Trap> {
        let value = |value: &Operand| operand(value, locals).map_err(|kind| Trap { kind, span });
        let values = |values: &[Operand]| values.iter().map(value).collect::<Result<Vec<_>, _>>();
        let result = match operation {
            Operation::StoreView(p) => resolve(p, views, locals)
                .and_then(|p| p.captured())
                .map(|p| Value::View(memory::ViewValue(p))),
            Operation::LoadView(p) => resolve(p, views, locals).and_then(|p| p.read()),
            Operation::Identity(left, right) => {
                let left = resolve(left, views, locals).and_then(|p| p.read());
                let right = resolve(right, views, locals).and_then(|p| p.read());
                match (left, right) {
                    (Ok(Value::Class(a)), Ok(Value::Class(b))) => {
                        Ok(Value::Bool(a.same_identity(&b)))
                    }
                    _ => Err(TrapKind::InvalidIr),
                }
            }
            Operation::AllocateClass(name) => Ok(Value::Class(memory::allocate_class(
                name,
                &self.program.classes[name],
                self.clock.clone(),
            ))),
            Operation::CompleteClass(v) => {
                let value = value(v)?;
                memory::complete_class(&value).map_err(|kind| Trap { kind, span })?;
                Ok(value)
            }
            Operation::Borrow(_) | Operation::CapturePlace(_) => Err(TrapKind::InvalidIr),
            Operation::Copy(place) => resolve(place, views, locals)
                .and_then(|p| p.read())
                .and_then(|v| memory::copy_value(&v, self.clock.clone())),
            Operation::Move(place) => resolve(place, views, locals).and_then(|p| p.read()),
            Operation::Use(v) => return value(v),
            Operation::Unary(operator, v) => match (*operator, value(v)?) {
                ("-", Value::Integer(v)) => v
                    .negate(self.checks)
                    .map(Value::Integer)
                    .map_err(TrapKind::Arithmetic),
                ("!", Value::Integer(v)) => Ok(Value::Integer(v.bit_not())),
                ("!", Value::Bool(v)) => Ok(Value::Bool(!v)),
                ("-", Value::F32(v)) => Ok(Value::F32(-v)),
                ("-", Value::F64(v)) => Ok(Value::F64(-v)),
                _ => Err(TrapKind::InvalidIr),
            },
            Operation::Binary(op, a, b) => binary(op, value(a)?, value(b)?, self.checks),
            Operation::Cast(v, ty) => cast(value(v)?, ty).ok_or(TrapKind::InvalidIr),
            Operation::Aggregate(v) => Ok(Value::Aggregate(values(v)?)),
            Operation::Enum(variant, v) => Ok(Value::Enum {
                variant: *variant,
                fields: values(v)?,
            }),
            Operation::TagPlace(place) => {
                match resolve(place, views, locals)
                    .and_then(|p| p.read())
                    .map_err(|kind| Trap { kind, span })?
                {
                    Value::Enum { variant, .. } => Ok(Value::Integer(Integer::from_bits(
                        IntegerType::U32,
                        variant as u128,
                    ))),
                    _ => Err(TrapKind::InvalidIr),
                }
            }
            Operation::Tag(v) => match value(v)? {
                Value::Enum { variant, .. } => Ok(Value::Integer(Integer::from_bits(
                    IntegerType::U32,
                    variant as u128,
                ))),
                _ => Err(TrapKind::InvalidIr),
            },
            Operation::Call(..)
            | Operation::CallIndirect(..)
            | Operation::CallView { .. }
            | Operation::CallStorage { .. } => {
                let (id, args) = match operation {
                    Operation::Call(id, args) => (*id, args),
                    Operation::CallStorage {
                        function,
                        arguments,
                        ..
                    }
                    | Operation::CallView {
                        function,
                        arguments,
                        ..
                    } => (*function, arguments),
                    Operation::CallIndirect(callee, args, _) => {
                        let Value::Function(id) = value(callee)? else {
                            return Err(Trap {
                                kind: TrapKind::InvalidIr,
                                span,
                            });
                        };
                        (id, args)
                    }
                    _ => unreachable!(),
                };
                let arguments = args
                    .iter()
                    .map(|arg| match arg {
                        Operand::Address(place) => resolve(place, views, locals)
                            .map(Argument::Borrowed)
                            .map_err(|kind| Trap { kind, span }),
                        other => value(other).map(Argument::Owned),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                return self.call(id, arguments, span);
            }
        };
        result.map_err(|kind| Trap { kind, span })
    }
}
fn operand(value: &Operand, locals: &[Slot]) -> Result<Value, TrapKind> {
    match value {
        Operand::Address(_) => Err(TrapKind::InvalidIr),
        Operand::Constant(value) => Ok(value.clone()),
        Operand::Local(local) => locals
            .get(*local)
            .and_then(|slot| slot.borrow().clone())
            .ok_or(TrapKind::InvalidIr),
    }
}
fn indices(place: &Place, locals: &[Slot]) -> Result<Vec<Projection>, TrapKind> {
    place
        .projections
        .iter()
        .map(|p| match p {
            Projection::Index(local) => {
                let Value::Integer(index) = operand(&Operand::Local(*local), locals)? else {
                    return Err(TrapKind::InvalidIr);
                };
                Ok(Projection::Field(
                    usize::try_from(index.bits()).map_err(|_| TrapKind::Bounds)?,
                ))
            }
            Projection::ConstantIndex(index) => Ok(Projection::Field(
                usize::try_from(*index).map_err(|_| TrapKind::Bounds)?,
            )),
            other => Ok(other.clone()),
        })
        .collect()
}

fn resolve(place: &Place, views: &Views, locals: &[Slot]) -> Result<Reference, TrapKind> {
    let result = if matches!(place.projections.first(), Some(Projection::Dereference)) {
        let mut origin = views
            .get(&place.local)
            .cloned()
            .ok_or(TrapKind::InvalidIr)?;
        origin.projections.extend(indices(
            &Place {
                local: place.local,
                projections: place.projections[1..].to_vec(),
            },
            locals,
        )?);
        origin
    } else {
        Reference {
            slot: locals
                .get(place.local)
                .cloned()
                .ok_or(TrapKind::InvalidIr)?,
            projections: indices(place, locals)?,
        }
    };
    result.normalized()
}
