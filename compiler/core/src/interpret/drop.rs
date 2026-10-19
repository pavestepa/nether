use super::*;
use crate::mir::drop_plan::{self, Failure, Initialization, Memory, Part};
use crate::reference_memory::Reference;
struct DropMemory<'a, 'p> {
    engine: &'a mut Engine<'p>,
    place: Reference,
    span: Span,
}
impl Memory for DropMemory<'_, '_> {
    type Panic = Trap;
    fn class(
        &mut self,
        _name: &str,
        path: &[Part],
        pending: &mut Option<Trap>,
    ) -> std::result::Result<(), Failure<Trap>> {
        let Value::Class(value) = self
            .place
            .projected(path)
            .read()
            .map_err(|_| Failure::InvalidLifecycle)?
        else {
            return Err(Failure::InvalidLifecycle);
        };
        let ty = value.payload.ty().clone();
        let plan = self.engine.destructors.class_payload(_name, &ty);
        let mut payload = DropMemory {
            engine: self.engine,
            place: Reference {
                slot: value.payload.clone(),
                projections: vec![],
            },
            span: self.span,
        };
        drop_plan::execute(&plan, &mut payload, pending)?;
        *value.payload.borrow_mut() = None;
        Ok(())
    }

    fn initialization(&self, path: &[Part]) -> Initialization {
        let place = self.place.projected(path);
        let state = place.slot.state().borrow().get(&place.parts());
        state
    }
    fn begin(&mut self, path: &[Part]) {
        let place = self.place.projected(path);
        place
            .slot
            .state()
            .borrow_mut()
            .begin(&place.parts())
            .expect("validated drop state");
    }
    fn finish(&mut self, path: &[Part]) {
        let place = self.place.projected(path);
        place.slot.state().borrow_mut().finish(&place.parts());
    }
    fn variant(&self, path: &[Part]) -> Option<usize> {
        match self.place.projected(path).read() {
            Ok(Value::Enum { variant, .. }) => Some(variant),
            _ => None,
        }
    }
    fn destructor(&mut self, entry: usize, path: &[Part]) -> std::result::Result<(), Trap> {
        let value = self.place.projected(path).read().map_err(|kind| Trap {
            kind,
            span: self.span,
        })?;
        self.engine.events.push(DropEvent { entry, value });
        let target = self.place.projected(path);
        let receiver = if matches!(
            self.engine.program.functions[entry].locals
                [self.engine.program.functions[entry].parameters[0]]
                .ty,
            Type::Class { .. }
        ) {
            memory::class_reference(
                memory::ClassValue {
                    payload: target.slot.clone(),
                },
                self.engine.clock.clone(),
            )
        } else {
            target
        };
        self.engine
            .call(entry, vec![Argument::Borrowed(receiver)], self.span)
            .map(|_| ())
            .map_err(|signal| match signal {
                Signal::Trap(t) => t,
                _ => Trap {
                    kind: TrapKind::InvalidIr,
                    span: self.span,
                },
            })
    }
}
impl Engine<'_> {
    pub(super) fn drop_place(
        &mut self,
        place: Reference,
        ty: &Type,
        pending: &mut Option<Trap>,
        span: Span,
    ) -> std::result::Result<(), Trap> {
        let plan = self.destructors.plan(ty);
        let mut memory = DropMemory {
            engine: self,
            place,
            span,
        };
        drop_plan::execute(&plan, &mut memory, pending).map_err(|failure| match failure {
            Failure::DoublePanic { second, .. } => Trap {
                kind: TrapKind::DoublePanic,
                span: second.span,
            },
            _ => Trap {
                kind: TrapKind::InvalidIr,
                span,
            },
        })
    }
    pub(super) fn drop_value(
        &mut self,
        value: Value,
        ty: &Type,
        pending: &mut Option<Trap>,
        span: Span,
    ) -> std::result::Result<(), Trap> {
        if matches!(value, Value::View(_)) {
            return Ok(());
        }
        let slot = memory::typed_slot(ty.clone(), self.clock.clone());
        let place = Reference {
            slot,
            projections: vec![],
        };
        place.write(value).map_err(|kind| Trap { kind, span })?;
        self.drop_place(place, ty, pending, span)
    }
}
impl Engine<'_> {
    pub(super) fn finish_scope(
        &mut self,
        result: Result<Value>,
        locals: &[usize],
        construction: bool,
        result_type: &Type,
        frame: &mut Frame,
        span: Span,
    ) -> Result<Value> {
        if matches!(&result,Err(Signal::Trap(t)) if t.kind==TrapKind::DoublePanic) {
            return result;
        }
        let mut pending = match &result {
            Err(Signal::Trap(t)) => Some(t.clone()),
            _ => None,
        };
        if pending.is_some() {
            self.active_panic = true;
        }
        let mut selected = locals.to_vec();
        if !construction {
            selected.sort_by_key(|id| std::cmp::Reverse(frame.locals[*id].initialized_at()));
        }
        for local in selected {
            if !frame.views.contains_key(&local) {
                let slot = frame.locals[local].clone();
                self.drop_place(
                    Reference {
                        slot: slot.clone(),
                        projections: vec![],
                    },
                    slot.ty(),
                    &mut pending,
                    span,
                )
                .map_err(Signal::Trap)?;
                *slot.borrow_mut() = None;
                slot.state().borrow_mut().finish(&[]);
            }
            frame.views.remove(&local);
        }
        if pending.is_some() {
            if let Ok(value) = result {
                self.drop_value(value, result_type, &mut pending, span)
                    .map_err(Signal::Trap)?;
            }
            Err(Signal::Trap(pending.unwrap()))
        } else {
            result
        }
    }
    pub(super) fn aggregate(
        &mut self,
        fields: &[Expr],
        frame: &mut Frame,
        span: Span,
    ) -> Result<Vec<Value>> {
        let mut values = Vec::new();
        for field in fields {
            match self.expr(field, frame) {
                Ok(value) => values.push(value),
                Err(signal) => {
                    if matches!(&signal,Signal::Trap(t) if t.kind==TrapKind::DoublePanic) {
                        return Err(signal);
                    }
                    let mut pending = match &signal {
                        Signal::Trap(t) => Some(t.clone()),
                        _ => None,
                    };
                    for (value, field) in values.into_iter().zip(fields) {
                        self.drop_value(value, &field.ty, &mut pending, span)
                            .map_err(Signal::Trap)?;
                    }
                    return Err(pending.map(Signal::Trap).unwrap_or(signal));
                }
            }
        }
        Ok(values)
    }
}
