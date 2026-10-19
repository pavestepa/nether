//! Host-side cells used only by the bounded reference interpreters. Clearing a
//! slot ends its language lifetime even when host handles to that cell remain.
//! Native views use raw addresses and never allocate or retain these cells.
use crate::{
    hir::{Type, Value},
    interpret::TrapKind,
    mir::Projection,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
pub(crate) type Clock = Rc<Cell<u64>>;
pub(crate) struct Storage {
    value: RefCell<Option<Value>>,
    pub state: RefCell<crate::mir::drop_state::State>,
    pub initialized_at: Cell<u64>,
    clock: Clock,
    ty: Type,
}
#[derive(Clone)]
pub(crate) struct Slot(Rc<Storage>);
impl std::ops::Deref for Slot {
    type Target = RefCell<Option<Value>>;
    fn deref(&self) -> &Self::Target {
        &self.0.value
    }
}
impl Slot {
    pub fn ty(&self) -> &Type {
        &self.0.ty
    }
    pub fn state(&self) -> &RefCell<crate::mir::drop_state::State> {
        &self.0.state
    }
    pub fn initialized_at(&self) -> u64 {
        self.0.initialized_at.get()
    }
    pub fn initialize(&self) -> Result<(), TrapKind> {
        self.0
            .state
            .borrow_mut()
            .initialize(&[])
            .map_err(|_| TrapKind::InvalidIr)?;
        let stamp = self
            .0
            .clock
            .get()
            .checked_add(1)
            .ok_or(TrapKind::StepLimit)?;
        self.0.clock.set(stamp);
        self.0.initialized_at.set(stamp);
        Ok(())
    }
}
pub(crate) fn typed_slot(ty: Type, clock: Clock) -> Slot {
    Slot(Rc::new(Storage {
        value: RefCell::new(None),
        state: RefCell::new(Default::default()),
        initialized_at: Cell::new(0),
        clock,
        ty,
    }))
}
#[derive(Clone)]
pub(crate) struct Reference {
    pub slot: Slot,
    pub projections: Vec<Projection>,
}
/// Returned descriptor used only by the reference interpreters.
#[derive(Clone)]
pub struct ViewValue(pub(crate) Reference);
impl std::fmt::Debug for ViewValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("View").field(&self.0.read()).finish()
    }
}
impl PartialEq for ViewValue {
    fn eq(&self, other: &Self) -> bool {
        self.0.read() == other.0.read()
    }
}
/// A handle used only by reference interpreters; native code owns a Box.
#[derive(Clone)]
pub struct ClassValue {
    pub(crate) payload: Slot,
}
impl ClassValue {
    pub(crate) fn same_identity(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.payload.0, &other.payload.0)
    }
}
impl std::fmt::Debug for ClassValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Class")
            .field(&*self.payload.borrow())
            .finish()
    }
}
impl PartialEq for ClassValue {
    fn eq(&self, other: &Self) -> bool {
        self.payload.ty() == other.payload.ty() && *self.payload.borrow() == *other.payload.borrow()
    }
}
pub(crate) fn allocate_class(
    name: &str,
    definition: &crate::hir::ClassDefinition,
    clock: Clock,
) -> ClassValue {
    let payload = typed_slot(
        Type::Record {
            name: name.into(),
            copy: false,
            fields: definition.fields.clone(),
        },
        clock,
    );
    *payload.borrow_mut() = Some(Value::Aggregate(vec![Value::Unit; definition.fields.len()]));
    payload
        .state()
        .borrow_mut()
        .constructing(&[])
        .expect("new class storage");
    ClassValue { payload }
}
pub(crate) fn class_reference(value: ClassValue, clock: Clock) -> Reference {
    let Type::Record { name, .. } = value.payload.ty() else {
        unreachable!()
    };
    let slot = typed_slot(
        Type::Class {
            name: name.clone(),
            copy: false,
            storage: None,
        },
        clock,
    );
    let place = Reference {
        slot,
        projections: vec![],
    };
    place.write(Value::Class(value)).expect("new handle");
    place
}
pub(crate) fn complete_class(value: &Value) -> Result<(), TrapKind> {
    let Value::Class(value) = value else {
        return Err(TrapKind::InvalidIr);
    };
    value
        .payload
        .state()
        .borrow_mut()
        .initialize(&[])
        .map_err(|_| TrapKind::InvalidIr)
}
/// Explicit language Copy; ordinary host clones are identity-preserving reads.
pub(crate) fn copy_value(value: &Value, clock: Clock) -> Result<Value, TrapKind> {
    Ok(match value {
        Value::Class(object) => {
            let copied = copy_value(
                object
                    .payload
                    .borrow()
                    .as_ref()
                    .ok_or(TrapKind::InvalidIr)?,
                clock.clone(),
            )?;
            let payload = typed_slot(object.payload.ty().clone(), clock);
            payload.initialize()?;
            *payload.borrow_mut() = Some(copied);
            Value::Class(ClassValue { payload })
        }
        Value::Aggregate(fields) => Value::Aggregate(
            fields
                .iter()
                .map(|v| copy_value(v, clock.clone()))
                .collect::<Result<_, _>>()?,
        ),
        Value::Enum { variant, fields } => Value::Enum {
            variant: *variant,
            fields: fields
                .iter()
                .map(|v| copy_value(v, clock.clone()))
                .collect::<Result<_, _>>()?,
        },
        value => value.clone(),
    })
}
#[derive(Clone)]
pub(crate) enum Argument {
    Owned(Value),
    Borrowed(Reference),
}
impl Reference {
    pub fn normalized(&self) -> Result<Self, TrapKind> {
        let mut result = Self {
            slot: self.slot.clone(),
            projections: vec![],
        };
        for projection in &self.projections {
            if let Value::Class(value) = result.read_raw()? {
                result = Self {
                    slot: value.payload,
                    projections: vec![],
                };
            }
            result.projections.push(projection.clone());
        }
        Ok(result)
    }
    pub fn captured(&self) -> Result<Self, TrapKind> {
        if let Value::Class(value) = self.read()? {
            Ok(class_reference(value, self.slot.0.clock.clone()))
        } else {
            Ok(self.clone())
        }
    }
    pub fn read(&self) -> Result<Value, TrapKind> {
        self.normalized()?.read_raw()
    }

    pub fn parts(&self) -> Vec<crate::mir::drop_plan::Part> {
        use crate::mir::drop_plan::Part;
        let mut ty = self.slot.0.ty.clone();
        let mut parts = Vec::new();
        for projection in &self.projections {
            match (projection, &ty) {
                (Projection::Field(i), Type::Array(element, _)) => {
                    parts.push(Part::Element(*i as u64));
                    ty = (**element).clone();
                }
                (Projection::Field(i), Type::Record { fields, .. }) => {
                    parts.push(Part::Field(*i));
                    ty = fields[*i].ty.clone();
                }
                (Projection::Field(i), Type::Tuple(fields)) => {
                    parts.push(Part::Field(*i));
                    ty = fields[*i].clone();
                }
                (Projection::Variant(i), Type::Enum { variants, .. }) => {
                    parts.push(Part::Variant(*i));
                    ty = Type::Tuple(variants[*i].fields.clone());
                }
                (Projection::Field(i), _) => parts.push(Part::Field(*i)),
                _ => unreachable!("reference must have resolved projections"),
            }
        }
        parts
    }
    pub fn projected(&self, parts: &[crate::mir::drop_plan::Part]) -> Self {
        use crate::mir::drop_plan::Part;
        let mut result = self.clone();
        result
            .projections
            .extend(parts.iter().map(|part| match part {
                Part::Field(i) => Projection::Field(*i),
                Part::Element(i) => Projection::Field(*i as usize),
                Part::Variant(i) => Projection::Variant(*i),
            }));
        result.normalized().expect("checked drop path")
    }
    pub fn forget(&self) -> Result<(), TrapKind> {
        self.slot
            .state()
            .borrow_mut()
            .forget(&self.parts())
            .map_err(|_| TrapKind::InvalidIr)
    }
    fn read_raw(&self) -> Result<Value, TrapKind> {
        let stored = self.slot.borrow();
        let mut value = stored.as_ref().ok_or(TrapKind::InvalidIr)?;
        let mut path = self.projections.as_slice();
        while let Some((first, rest)) = path.split_first() {
            match (first, value) {
                (Projection::Field(index), Value::Aggregate(fields)) => {
                    value = fields.get(*index).ok_or(TrapKind::Bounds)?;
                    path = rest;
                }
                (Projection::Variant(expected), Value::Enum { variant, fields })
                    if expected == variant =>
                {
                    if rest.is_empty() {
                        return Ok(Value::Aggregate(fields.clone()));
                    }
                    let Some((Projection::Field(index), tail)) = rest.split_first() else {
                        return Err(TrapKind::InvalidIr);
                    };
                    value = fields.get(*index).ok_or(TrapKind::Bounds)?;
                    path = tail;
                }
                _ => return Err(TrapKind::InvalidIr),
            }
        }
        Ok(value.clone())
    }
    pub fn write(&self, value: Value) -> Result<(), TrapKind> {
        if self.projections.is_empty() {
            self.slot.initialize()?;
            *self.slot.borrow_mut() = Some(value);
            return Ok(());
        }
        self.slot
            .state()
            .borrow_mut()
            .initialize(&self.parts())
            .map_err(|_| TrapKind::InvalidIr)?;
        write(
            self.slot.borrow_mut().as_mut().ok_or(TrapKind::InvalidIr)?,
            &self.projections,
            value,
        )
    }
}
fn write(target: &mut Value, path: &[Projection], value: Value) -> Result<(), TrapKind> {
    let Some((first, rest)) = path.split_first() else {
        *target = value;
        return Ok(());
    };
    match (first, target) {
        (Projection::Field(index), Value::Aggregate(fields)) => {
            write(fields.get_mut(*index).ok_or(TrapKind::Bounds)?, rest, value)
        }
        (Projection::Variant(expected), Value::Enum { variant, fields }) if expected == variant => {
            if let Some((Projection::Field(index), tail)) = rest.split_first() {
                write(fields.get_mut(*index).ok_or(TrapKind::Bounds)?, tail, value)
            } else if rest.is_empty() {
                let Value::Aggregate(v) = value else {
                    return Err(TrapKind::InvalidIr);
                };
                *fields = v;
                Ok(())
            } else {
                Err(TrapKind::InvalidIr)
            }
        }
        _ => Err(TrapKind::InvalidIr),
    }
}
