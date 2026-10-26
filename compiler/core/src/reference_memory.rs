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

/// Non-owning interpreter address. Weak storage cannot keep a returned local alive.
#[derive(Clone)]
pub struct RawPointer {
    heap: Option<std::rc::Weak<RawHeap>>,
    storage: std::rc::Weak<Storage>,
    prefix: Vec<Projection>,
    index: i128,
    length: i128,
    array: bool,
}
impl std::fmt::Debug for RawPointer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RawPointer")
            .field("index", &self.index)
            .field("length", &self.length)
            .finish_non_exhaustive()
    }
}
impl PartialEq for RawPointer {
    fn eq(&self, other: &Self) -> bool {
        match (&self.heap, &other.heap) {
            (Some(a), Some(b)) => return a.ptr_eq(b) && self.index == other.index,
            (None, None) => (),
            _ => return false,
        }
        self.storage.ptr_eq(&other.storage)
            && self.prefix == other.prefix
            && self.index == other.index
    }
}
impl RawPointer {
    pub(crate) fn new(reference: Reference) -> Result<Self, TrapKind> {
        let reference = reference.normalized()?;
        reference.read()?;
        let mut ty = reference.slot.ty().clone();
        let mut array = None;
        for (depth, projection) in reference.projections.iter().enumerate() {
            ty = match (projection, ty) {
                (Projection::Field(i), Type::Array(element, count)) => {
                    if depth + 1 == reference.projections.len() {
                        array = Some((*i as i128, count as i128));
                    }
                    *element
                }
                (Projection::Field(i), Type::Record { fields, .. }) => fields[*i].ty.clone(),
                (Projection::Field(i), Type::Tuple(fields)) => fields[*i].clone(),
                (Projection::Variant(i), Type::Enum { variants, .. }) => {
                    Type::Tuple(variants[*i].fields.clone())
                }
                _ => return Err(TrapKind::InvalidIr),
            };
        }
        let mut prefix = reference.projections;
        if array.is_some() {
            prefix.pop();
        }
        let (index, length) = array.unwrap_or((0, 1));
        Ok(Self {
            heap: None,
            storage: std::rc::Rc::downgrade(&reference.slot.0),
            prefix,
            index,
            length,
            array: array.is_some(),
        })
    }
    pub(crate) fn offset(mut self, amount: i128) -> Result<Self, TrapKind> {
        self.index = self.index.checked_add(amount).ok_or(TrapKind::Bounds)?;
        if self.index < 0 || self.index > self.length {
            return Err(TrapKind::Bounds);
        }
        Ok(self)
    }
    fn reference(&self) -> Result<Reference, TrapKind> {
        if self.index < 0 || self.index >= self.length {
            return Err(TrapKind::Bounds);
        }
        let slot = Slot(self.storage.upgrade().ok_or(TrapKind::InvalidIr)?);
        let mut projections = self.prefix.clone();
        if self.array {
            projections.push(Projection::Field(self.index as usize));
        }
        Ok(Reference { slot, projections })
    }
    pub(crate) fn read(&self) -> Result<Value, TrapKind> {
        if let Some(heap) = &self.heap {
            let heap = heap.upgrade().ok_or(TrapKind::InvalidIr)?;
            return heap
                .slots
                .borrow()
                .get(&(self.index as u64))
                .cloned()
                .ok_or(TrapKind::InvalidIr);
        }
        self.reference()?.read()
    }
    pub(crate) fn write(&self, value: Value) -> Result<Value, TrapKind> {
        if let Some(heap) = &self.heap {
            let heap = heap.upgrade().ok_or(TrapKind::InvalidIr)?;
            let mut slots = heap.slots.borrow_mut();
            let slot = slots
                .get_mut(&(self.index as u64))
                .ok_or(TrapKind::InvalidIr)?;
            *slot = value;
            return Ok(Value::Unit);
        }
        let reference = self.reference()?;
        reference.read()?; // This operation replaces initialized scalar storage only.
        write(
            reference
                .slot
                .borrow_mut()
                .as_mut()
                .ok_or(TrapKind::InvalidIr)?,
            &reference.projections,
            value,
        )?;
        Ok(Value::Unit)
    }
}

struct RawHeap {
    capacity: u64,
    slots: RefCell<std::collections::BTreeMap<u64, Value>>,
}
#[derive(Default)]
pub(crate) struct RawAllocations(Vec<Rc<RawHeap>>);
impl RawAllocations {
    pub(crate) fn allocate(&mut self, ty: &Type, capacity: u64) -> Result<Value, TrapKind> {
        crate::layout::layout(ty)
            .map_err(|_| TrapKind::InvalidIr)?
            .array(capacity)
            .map_err(|_| TrapKind::Allocation)?;
        let heap = Rc::new(RawHeap {
            capacity,
            slots: RefCell::new(Default::default()),
        });
        let pointer = RawPointer {
            heap: Some(Rc::downgrade(&heap)),
            storage: Default::default(),
            prefix: vec![],
            index: 0,
            length: capacity as i128,
            array: true,
        };
        self.0.push(heap);
        Ok(Value::Pointer(pointer))
    }
    pub(crate) fn operation(&mut self, op: &str, argument: Value) -> Result<Value, TrapKind> {
        let Value::Aggregate(mut args) = argument else {
            return Err(TrapKind::InvalidIr);
        };
        let (Some(Value::Pointer(pointer)), Some(Value::Integer(index))) =
            (args.first(), args.get(1))
        else {
            return Err(TrapKind::InvalidIr);
        };
        let heap = pointer
            .heap
            .as_ref()
            .and_then(|h| h.upgrade())
            .ok_or(TrapKind::InvalidIr)?;
        if pointer.index != 0 {
            return Err(TrapKind::InvalidIr);
        }
        let index = u64::try_from(index.bits()).map_err(|_| TrapKind::Bounds)?;
        if op == "storage_release" {
            if index != heap.capacity || !heap.slots.borrow().is_empty() {
                return Err(TrapKind::InvalidIr);
            }
            self.0.retain(|h| !Rc::ptr_eq(h, &heap));
            return Ok(Value::Unit);
        }
        if index >= heap.capacity {
            return Err(TrapKind::Bounds);
        }
        let mut slots = heap.slots.borrow_mut();
        match op {
            "storage_initialize" => {
                if slots.contains_key(&index) || args.len() != 3 {
                    return Err(TrapKind::InvalidIr);
                }
                slots.insert(index, args.pop().unwrap());
                Ok(Value::Unit)
            }
            "storage_replace" => {
                let target = slots.get_mut(&index).ok_or(TrapKind::InvalidIr)?;
                if args.len() != 3 {
                    return Err(TrapKind::InvalidIr);
                }
                Ok(std::mem::replace(target, args.pop().unwrap()))
            }
            "storage_take" => slots.remove(&index).ok_or(TrapKind::InvalidIr),
            "storage_drop" => {
                slots.remove(&index).ok_or(TrapKind::InvalidIr)?;
                Ok(Value::Unit)
            }
            _ => Err(TrapKind::InvalidIr),
        }
    }
}
