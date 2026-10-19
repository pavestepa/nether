use super::*;
use crate::mir::drop_plan::{self, Failure, Initialization, Memory, Part};
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
        let plan = self.engine.program.destructors.class_payload(_name, &ty);
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
    fn destructor(&mut self, entry: usize, path: &[Part]) -> Result<(), Trap> {
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
    }
}
impl Engine<'_> {
    pub(super) fn drop_place(
        &mut self,
        place: Reference,
        ty: &Type,
        pending: &mut Option<Trap>,
        span: Span,
    ) -> Result<(), Trap> {
        let plan = self.program.destructors.plan(ty);
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
    ) -> Result<(), Trap> {
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
    pub(super) fn new_panic(&mut self, trap: Trap) -> Trap {
        let kind = if self.active_panic {
            TrapKind::DoublePanic
        } else {
            trap.kind
        };
        self.active_panic = true;
        Trap {
            kind,
            span: trap.span,
        }
    }
}
pub(super) fn place_type(place: &Place, function: &Function) -> Type {
    let mut ty = function.locals[place.local].ty.clone();
    for projection in &place.projections {
        ty = match (projection, &ty) {
            (Projection::Dereference, _) => ty.clone(),
            (Projection::Field(i), Type::Record { fields, .. }) => fields[*i].ty.clone(),
            (Projection::Field(i), Type::Class { .. }) => {
                function.classes[&ty.class_key()].fields[*i].ty.clone()
            }
            (Projection::Field(i), Type::Tuple(fields)) => fields[*i].clone(),
            (Projection::Variant(i), Type::Enum { variants, .. }) => {
                Type::Tuple(variants[*i].fields.clone())
            }
            (Projection::Index(_) | Projection::ConstantIndex(_), Type::Array(element, _)) => {
                (**element).clone()
            }
            _ => unreachable!("checked place"),
        };
    }
    ty
}
pub(super) fn unwind_entry(function: &Function) -> Option<usize> {
    function
        .blocks
        .iter()
        .position(|block| match &block.terminator {
            Terminator::EndStorage { locals, next } => {
                locals.len() == function.locals.len()
                    && matches!(function.blocks[*next].terminator, Terminator::ResumePanic)
            }
            _ => false,
        })
}
