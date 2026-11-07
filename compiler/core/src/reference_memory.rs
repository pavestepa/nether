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
    raw_origin: RefCell<Option<RawPointer>>,
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
        raw_origin: RefCell::new(None),
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
    // Keep Value compact: recursive diagnostic execution places values on the
    // host stack. Pointer copies share immutable type descriptions.
    address_only: Option<u64>,
    storage_type: Rc<Type>,
    view_type: Rc<Type>,
    heap: Option<std::rc::Weak<RawHeap>>,
    storage: std::rc::Weak<Storage>,
    prefix: Rc<Vec<Projection>>,
    region_offset: u64,
    allocation_extent: u64,
    index: i128,
    byte_in_element: i128,
    length: i128,
    array: bool,
}
impl std::fmt::Debug for RawPointer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RawPointer")
            .field("index", &self.index)
            .field("byte_in_element", &self.byte_in_element)
            .field("length", &self.length)
            .finish_non_exhaustive()
    }
}
impl PartialEq for RawPointer {
    fn eq(&self, other: &Self) -> bool {
        if self.address_only.is_some() || other.address_only.is_some() {
            return self.address_only == other.address_only;
        }
        match (&self.heap, &other.heap) {
            (Some(a), Some(b)) => {
                return a.ptr_eq(b)
                    && self.index == other.index
                    && self.byte_in_element == other.byte_in_element
            }
            (None, None) => (),
            _ => return false,
        }
        self.storage.ptr_eq(&other.storage)
            && self.prefix == other.prefix
            && self.index == other.index
            && self.byte_in_element == other.byte_in_element
    }
}
impl RawPointer {
    pub(crate) fn new(reference: Reference) -> Result<Self, TrapKind> {
        let reference = reference.normalized()?;
        reference.read()?;
        if reference.projections.is_empty() {
            if let Some(pointer) = reference.slot.0.raw_origin.borrow().as_ref() {
                return Ok(pointer.clone());
            }
        }
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
        let region_offset = projection_byte_offset(reference.slot.ty(), &prefix).unwrap_or(0);
        let allocation_extent = crate::layout::layout(reference.slot.ty()).map_or(0, |l| l.size());
        Ok(Self {
            address_only: None,
            storage_type: Rc::new(ty.clone()),
            view_type: Rc::new(ty),
            heap: None,
            storage: std::rc::Rc::downgrade(&reference.slot.0),
            prefix: Rc::new(prefix),
            region_offset,
            allocation_extent,
            index,
            byte_in_element: 0,
            length,
            array: array.is_some(),
        })
    }
    pub(crate) fn reinterpret(mut self, target: &Type) -> Option<Self> {
        if *self.view_type != *target && !self.view_type.raw_reinterpret_to(target) {
            return None;
        }
        self.view_type = Rc::new(target.clone());
        Some(self)
    }
    fn storage_stride(&self) -> Result<i128, TrapKind> {
        crate::layout::layout(&self.storage_type)
            .map(|v| i128::from(v.size()))
            .map_err(|_| TrapKind::InvalidIr)
    }
    fn byte_position(&self) -> Result<i128, TrapKind> {
        self.index
            .checked_mul(self.storage_stride()?)
            .and_then(|n| n.checked_add(self.byte_in_element))
            .ok_or(TrapKind::Bounds)
    }
    pub(crate) fn raw_offset(self, amount: i128) -> Result<Self, TrapKind> {
        if amount != 0 && self.storage_stride()? == 0 {
            return Err(TrapKind::Bounds);
        }
        self.offset(amount)
    }
    pub(crate) fn offset(mut self, amount: i128) -> Result<Self, TrapKind> {
        if self.address_only.is_some() {
            return if amount == 0 {
                Ok(self)
            } else {
                Err(TrapKind::Bounds)
            };
        }
        let stride = self.storage_stride()?;
        if stride == 0 {
            self.index = self.index.checked_add(amount).ok_or(TrapKind::Bounds)?;
        } else {
            let view = crate::layout::layout(&self.view_type).map_err(|_| TrapKind::InvalidIr)?;
            let position = amount
                .checked_mul(i128::from(view.size()))
                .and_then(|n| self.byte_position().ok()?.checked_add(n))
                .ok_or(TrapKind::Bounds)?;
            if position < 0 || position > self.length * stride {
                return Err(TrapKind::Bounds);
            }
            self.index = position / stride;
            self.byte_in_element = position % stride;
        }
        if self.index < 0 || self.index > self.length {
            return Err(TrapKind::Bounds);
        }
        Ok(self)
    }
    fn same_allocation(&self, other: &Self) -> bool {
        if self.address_only.is_some() || other.address_only.is_some() {
            return false;
        }
        match (&self.heap, &other.heap) {
            (Some(a), Some(b)) => a.ptr_eq(b),
            (None, None) => self.storage.ptr_eq(&other.storage),
            _ => false,
        }
    }
    fn same_region(&self, other: &Self) -> bool {
        if self.address_only.is_some() || other.address_only.is_some() {
            return false;
        }
        match (&self.heap, &other.heap) {
            (Some(a), Some(b)) => a.ptr_eq(b),
            (None, None) => {
                self.storage.ptr_eq(&other.storage)
                    && self.prefix == other.prefix
                    && self.array == other.array
            }
            _ => false,
        }
    }
    pub(crate) fn difference(&self, other: &Self) -> Result<i128, TrapKind> {
        let same = match (&self.heap, &other.heap) {
            (Some(a), Some(b)) => a.ptr_eq(b) && a.upgrade().is_some(),
            (None, None) => {
                self.storage.ptr_eq(&other.storage)
                    && self.storage.upgrade().is_some()
                    && self.prefix == other.prefix
                    && self.array == other.array
            }
            _ => false,
        };
        if !same
            || self.length != other.length
            || self.index < 0
            || other.index < 0
            || self.index > self.length
            || other.index > other.length
        {
            return Err(TrapKind::Bounds);
        }
        let view = crate::layout::layout(&self.view_type).map_err(|_| TrapKind::InvalidIr)?;
        let size = i128::from(view.size());
        let bytes = self.byte_position()? - other.byte_position()?;
        if self.view_type != other.view_type || size == 0 || bytes % size != 0 {
            return Err(TrapKind::Bounds);
        }
        let distance = bytes / size;
        i64::try_from(distance)
            .map(|n| n as i128)
            .map_err(|_| TrapKind::Bounds)
    }
    fn reference(&self) -> Result<Reference, TrapKind> {
        if self.index < 0 || self.index >= self.length {
            return Err(TrapKind::Bounds);
        }
        let slot = Slot(self.storage.upgrade().ok_or(TrapKind::InvalidIr)?);
        let mut projections = (*self.prefix).clone();
        if self.array {
            projections.push(Projection::Field(self.index as usize));
        }
        Ok(Reference { slot, projections })
    }
    fn bits_window(&self) -> Result<(i128, usize, usize), TrapKind> {
        let source = crate::layout::layout(&self.storage_type).map_err(|_| TrapKind::InvalidIr)?;
        let target = crate::layout::layout(&self.view_type).map_err(|_| TrapKind::InvalidIr)?;
        let position = self.byte_position()?;
        if position < 0
            || position + i128::from(target.size()) > self.length * i128::from(source.size())
            || position % i128::from(target.align()) != 0
            || source.align() < target.align()
        {
            return Err(TrapKind::Bounds);
        }
        Ok((position, source.size() as usize, target.size() as usize))
    }
    pub(crate) fn read(&self) -> Result<Value, TrapKind> {
        if self.storage_type == self.view_type && self.byte_in_element == 0 {
            return self.read_storage();
        }
        let (position, stride, size) = self.bits_window()?;
        let mut bytes = Vec::with_capacity(size);
        let mut offset = 0;
        while offset < size {
            let absolute = position + offset as i128;
            let mut source = self.clone();
            source.index = absolute / stride as i128;
            source.byte_in_element = 0;
            let stored = bits_bytes(source.read_storage()?, &self.storage_type)?;
            let start = (absolute % stride as i128) as usize;
            let count = (size - offset).min(stride - start);
            bytes.extend_from_slice(&stored[start..start + count]);
            offset += count;
        }
        bits_value(&bytes, &self.view_type)
    }
    fn read_storage(&self) -> Result<Value, TrapKind> {
        if let Some(heap) = &self.heap {
            let heap = heap.upgrade().ok_or(TrapKind::InvalidIr)?;
            return heap
                .slots
                .borrow()
                .get(&(self.index as u64))
                .ok_or(TrapKind::InvalidIr)?
                .borrow()
                .clone()
                .ok_or(TrapKind::InvalidIr);
        }
        self.reference()?.read()
    }
    pub(crate) fn initialize(&self, value: Value) -> Result<Value, TrapKind> {
        if self.index < 0
            || self.index >= self.length
            || self.byte_in_element != 0
            || crate::layout::layout(&self.view_type)
                .map_err(|_| TrapKind::InvalidIr)?
                .size()
                != self.storage_stride()? as u64
        {
            return Err(TrapKind::Bounds);
        }
        let value = if self.storage_type == self.view_type {
            value
        } else {
            bits_value(&bits_bytes(value, &self.view_type)?, &self.storage_type)?
        };
        if let Some(heap) = &self.heap {
            let heap = heap.upgrade().ok_or(TrapKind::InvalidIr)?;
            let mut slots = heap.slots.borrow_mut();
            let slot = slots
                .entry(self.index as u64)
                .or_insert_with(|| typed_slot((*self.storage_type).clone(), heap.clock.clone()));
            if slot.borrow().is_some() {
                return Err(TrapKind::InvalidIr);
            }
            let mut origin = self.clone();
            origin.view_type = self.storage_type.clone();
            slot.0.raw_origin.replace(Some(origin));
            slot.initialize()?;
            *slot.borrow_mut() = Some(value);
        } else {
            let reference = self.reference()?;
            if reference.slot.state().borrow().get(&reference.parts())
                != crate::mir::drop_plan::Initialization::Vacant
            {
                return Err(TrapKind::InvalidIr);
            }
            reference.write(value)?;
        }
        Ok(Value::Unit)
    }
    pub(crate) fn take(&self) -> Result<Value, TrapKind> {
        // An owning operation must target an entire original slot, never a
        // byte window into another value. Byte access creates no new owner.
        if self.byte_in_element != 0
            || crate::layout::layout(&self.view_type)
                .map_err(|_| TrapKind::InvalidIr)?
                .size()
                != self.storage_stride()? as u64
        {
            return Err(TrapKind::InvalidIr);
        }
        // Decode before invalidating: a same-size reinterpret transfers the
        // requested typed value, not the original diagnostic representation.
        let value = self.read()?;
        if let Some(heap) = &self.heap {
            let heap = heap.upgrade().ok_or(TrapKind::InvalidIr)?;
            heap.slots
                .borrow()
                .get(&(self.index as u64))
                .ok_or(TrapKind::InvalidIr)?
                .borrow_mut()
                .take()
                .ok_or(TrapKind::InvalidIr)?;
            return Ok(value);
        }
        let reference = self.reference()?;
        reference.forget()?;
        if reference.projections.is_empty() {
            *reference.slot.borrow_mut() = None;
        } else {
            write(
                reference
                    .slot
                    .borrow_mut()
                    .as_mut()
                    .ok_or(TrapKind::InvalidIr)?,
                &reference.projections,
                Value::Unit,
            )?;
        }
        Ok(value)
    }
    pub(crate) fn write(&self, value: Value) -> Result<Value, TrapKind> {
        if self.storage_type == self.view_type && self.byte_in_element == 0 {
            return self.write_storage(value);
        }
        let (position, stride, size) = self.bits_window()?;
        let bytes = bits_bytes(value, &self.view_type)?;
        if bytes.len() != size {
            return Err(TrapKind::InvalidIr);
        }
        let first = position / stride as i128;
        let last = (position + size as i128 - 1) / stride as i128;
        let mut updates = Vec::new();
        for index in first..=last {
            let mut destination = self.clone();
            destination.index = index;
            destination.byte_in_element = 0;
            let mut stored = bits_bytes(destination.read_storage()?, &self.storage_type)?;
            for (offset, byte) in stored.iter_mut().enumerate() {
                let relative = index * stride as i128 + offset as i128 - position;
                if relative >= 0 && relative < size as i128 {
                    *byte = bytes[relative as usize];
                }
            }
            updates.push((destination, bits_value(&stored, &self.storage_type)?));
        }
        for (destination, value) in updates {
            destination.write_storage(value)?;
        }
        Ok(Value::Unit)
    }
    fn write_storage(&self, value: Value) -> Result<Value, TrapKind> {
        if let Some(heap) = &self.heap {
            let heap = heap.upgrade().ok_or(TrapKind::InvalidIr)?;
            let mut slots = heap.slots.borrow_mut();
            let slot = slots
                .get_mut(&(self.index as u64))
                .ok_or(TrapKind::InvalidIr)?;
            let mut stored = slot.borrow_mut();
            if stored.is_none() {
                return Err(TrapKind::InvalidIr);
            }
            *stored = Some(value);
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

fn projection_byte_offset(ty: &Type, path: &[Projection]) -> Option<u64> {
    let Some((head, tail)) = path.split_first() else {
        return Some(0);
    };
    let (offset, child) = match (ty, head) {
        (Type::Array(element, _), Projection::Field(index)) => (
            crate::layout::layout(element)
                .ok()?
                .size()
                .checked_mul(*index as u64)?,
            (**element).clone(),
        ),
        (Type::Record { fields, .. }, Projection::Field(index)) => {
            let layout = nether_semantics::AggregateLayout::new(
                &fields
                    .iter()
                    .map(|f| crate::layout::layout(&f.ty))
                    .collect::<Result<Vec<_>, _>>()
                    .ok()?,
            )
            .ok()?;
            (
                *layout.field_offsets.get(*index)?,
                fields.get(*index)?.ty.clone(),
            )
        }
        (Type::Tuple(fields), Projection::Field(index)) => {
            let layout = nether_semantics::AggregateLayout::new(
                &fields
                    .iter()
                    .map(crate::layout::layout)
                    .collect::<Result<Vec<_>, _>>()
                    .ok()?,
            )
            .ok()?;
            (
                *layout.field_offsets.get(*index)?,
                fields.get(*index)?.clone(),
            )
        }
        (Type::Enum { variants, .. }, Projection::Variant(index)) => {
            let layout = nether_semantics::EnumLayout::new(
                &variants
                    .iter()
                    .map(|v| crate::layout::layout(&Type::Tuple(v.fields.clone())))
                    .collect::<Result<Vec<_>, _>>()
                    .ok()?,
            )
            .ok()?;
            (
                layout.payload_offset,
                Type::Tuple(variants.get(*index)?.fields.clone()),
            )
        }
        _ => return None,
    };
    offset.checked_add(projection_byte_offset(&child, tail)?)
}

struct RawHeap {
    owned: Cell<bool>,
    capacity: u64,
    slots: RefCell<std::collections::BTreeMap<u64, Slot>>,
    clock: Clock,
}
struct ExposedAddress {
    base: u64,
    extent: u64,
    pointer: RawPointer,
    writable: bool,
}
pub(crate) struct RawAllocations {
    heaps: Vec<Rc<RawHeap>>,
    exposed: Vec<ExposedAddress>,
    next_address: u64,
}
impl Default for RawAllocations {
    fn default() -> Self {
        Self {
            heaps: vec![],
            exposed: vec![],
            next_address: 4096,
        }
    }
}
impl RawAllocations {
    pub(crate) fn expose(
        &mut self,
        value: Value,
        writable: bool,
        target: &Type,
    ) -> Result<Value, TrapKind> {
        let Type::Integer(kind) = target else {
            return Err(TrapKind::InvalidIr);
        };
        let Value::Pointer(pointer) = value else {
            return Err(TrapKind::InvalidIr);
        };
        let address = if let Some(address) = pointer.address_only {
            address
        } else {
            let position = u64::try_from(pointer.byte_position()?).map_err(|_| TrapKind::Bounds)?;
            let existing = self
                .exposed
                .iter_mut()
                .find(|entry| entry.pointer.same_region(&pointer));
            if let Some(entry) = existing {
                entry.writable |= writable;
                entry.base.checked_add(position).ok_or(TrapKind::Bounds)?
            } else {
                let extent = u64::try_from(pointer.length * pointer.storage_stride()?)
                    .map_err(|_| TrapKind::Bounds)?;
                // Different projections of the same object expose the same
                // allocation address plus their physical field/element offsets.
                let known = self.exposed.iter().find(|entry| {
                    pointer.allocation_extent != 0 && entry.pointer.same_allocation(&pointer)
                });
                let base = if let Some(entry) = known {
                    entry
                        .base
                        .checked_sub(entry.pointer.region_offset)
                        .and_then(|base| base.checked_add(pointer.region_offset))
                        .ok_or(TrapKind::Bounds)?
                } else {
                    let base = self.next_address;
                    self.next_address = base
                        .checked_add(pointer.allocation_extent.max(extent))
                        .and_then(|n| n.checked_add(4096))
                        .and_then(|n| n.checked_add(15))
                        .map(|n| n & !15)
                        .ok_or(TrapKind::Allocation)?;
                    base.checked_add(pointer.region_offset)
                        .ok_or(TrapKind::Bounds)?
                };
                let mut root = pointer;
                root.index = 0;
                root.byte_in_element = 0;
                self.exposed.push(ExposedAddress {
                    base,
                    extent,
                    pointer: root,
                    writable,
                });
                base.checked_add(position).ok_or(TrapKind::Bounds)?
            }
        };
        Ok(Value::Integer(nether_semantics::Integer::from_bits(
            *kind,
            u128::from(address),
        )))
    }
    pub(crate) fn recover(&self, value: Value, target: &Type) -> Result<Value, TrapKind> {
        let Value::Integer(address) = value else {
            return Err(TrapKind::InvalidIr);
        };
        let Type::Pointer { mutable, pointee } = target else {
            return Err(TrapKind::InvalidIr);
        };
        let address = u64::try_from(address.bits()).map_err(|_| TrapKind::Bounds)?;
        let mut exposed = self.exposed.iter().collect::<Vec<_>>();
        exposed.sort_by_key(|entry| std::cmp::Reverse(entry.extent));
        for entry in exposed {
            if address < entry.base || address - entry.base > entry.extent {
                continue;
            }
            if *mutable && !entry.writable {
                continue;
            }
            let mut pointer = entry.pointer.clone();
            let stride = pointer.storage_stride()?;
            if stride == 0 {
                return Err(TrapKind::InvalidIr);
            }
            let position = i128::from(address - entry.base);
            pointer.index = position / stride;
            pointer.byte_in_element = position % stride;
            if let Some(pointer) = pointer.reinterpret(pointee) {
                return Ok(Value::Pointer(pointer));
            }
        }
        if self
            .exposed
            .iter()
            .any(|entry| address >= entry.base && address - entry.base <= entry.extent)
        {
            return Err(TrapKind::InvalidIr);
        }
        Ok(Value::Pointer(RawPointer {
            address_only: Some(address),
            storage_type: Rc::new((**pointee).clone()),
            view_type: Rc::new((**pointee).clone()),
            heap: None,
            storage: Default::default(),
            prefix: Rc::new(vec![]),
            region_offset: 0,
            allocation_extent: 0,
            index: 0,
            byte_in_element: 0,
            length: 0,
            array: false,
        }))
    }

    pub(crate) fn storage_view(&self, storage: Value, index: Value) -> Result<Value, TrapKind> {
        let Value::Aggregate(fields) = storage else {
            return Err(TrapKind::InvalidIr);
        };
        let [Value::Pointer(pointer), Value::Integer(capacity)] = fields.as_slice() else {
            return Err(TrapKind::InvalidIr);
        };
        let Value::Integer(index) = index else {
            return Err(TrapKind::InvalidIr);
        };
        if index.bits() >= capacity.bits() {
            return Err(TrapKind::Bounds);
        }
        let heap = pointer
            .heap
            .as_ref()
            .and_then(|h| h.upgrade())
            .ok_or(TrapKind::InvalidIr)?;
        let slot = heap
            .slots
            .borrow()
            .get(&(index.bits() as u64))
            .cloned()
            .ok_or(TrapKind::InvalidIr)?;
        let place = Reference {
            slot,
            projections: vec![],
        };
        place.read()?;
        Ok(Value::View(ViewValue(place)))
    }
    pub(crate) fn allocate(
        &mut self,
        ty: &Type,
        capacity: u64,
        clock: Clock,
    ) -> Result<Value, TrapKind> {
        crate::layout::layout(ty)
            .map_err(|_| TrapKind::InvalidIr)?
            .array(capacity)
            .map_err(|_| TrapKind::Allocation)?;
        let heap = Rc::new(RawHeap {
            clock,
            owned: Cell::new(true),
            capacity,
            slots: RefCell::new(Default::default()),
        });
        let pointer = RawPointer {
            address_only: None,
            storage_type: Rc::new(ty.clone()),
            view_type: Rc::new(ty.clone()),
            heap: Some(Rc::downgrade(&heap)),
            storage: Default::default(),
            prefix: Rc::new(vec![]),
            region_offset: 0,
            allocation_extent: crate::layout::layout(ty)
                .map_err(|_| TrapKind::InvalidIr)?
                .array(capacity)
                .map_err(|_| TrapKind::Allocation)?
                .size(),
            index: 0,
            byte_in_element: 0,
            length: capacity as i128,
            array: true,
        };
        self.heaps.push(heap);
        Ok(Value::Pointer(pointer))
    }
    pub(crate) fn drop_range_arguments(
        &self,
        argument: Value,
    ) -> Result<(Value, u64, u64), TrapKind> {
        let Value::Aggregate(args) = argument else {
            return Err(TrapKind::InvalidIr);
        };
        let [Value::Pointer(pointer), Value::Integer(capacity), Value::Integer(start), Value::Integer(count)] =
            args.as_slice()
        else {
            return Err(TrapKind::InvalidIr);
        };
        let heap = pointer
            .heap
            .as_ref()
            .and_then(|h| h.upgrade())
            .ok_or(TrapKind::InvalidIr)?;
        if pointer.byte_in_element != 0
            || pointer.index != 0
            || capacity.bits() != heap.capacity as u128
        {
            return Err(TrapKind::InvalidIr);
        }
        if start.bits() > capacity.bits() || count.bits() > capacity.bits() - start.bits() {
            return Err(TrapKind::Bounds);
        }
        let (start, count) = (start.bits() as u64, count.bits() as u64);
        if heap
            .slots
            .borrow()
            .range(start..start + count)
            .filter(|(_, s)| s.borrow().is_some())
            .count() as u64
            != count
        {
            return Err(TrapKind::InvalidIr);
        }
        Ok((Value::Pointer(pointer.clone()), start, count))
    }
    pub(crate) fn operation(&mut self, op: &str, argument: Value) -> Result<Value, TrapKind> {
        let Value::Aggregate(mut args) = argument else {
            return Err(TrapKind::InvalidIr);
        };
        if op == "storage_move_slots" {
            let [Value::Pointer(destination), Value::Integer(dc), Value::Integer(ds), Value::Pointer(source), Value::Integer(sc), Value::Integer(ss), Value::Integer(count)] =
                args.as_slice()
            else {
                return Err(TrapKind::InvalidIr);
            };
            let target = destination
                .heap
                .as_ref()
                .and_then(|h| h.upgrade())
                .ok_or(TrapKind::InvalidIr)?;
            let origin = source
                .heap
                .as_ref()
                .and_then(|h| h.upgrade())
                .ok_or(TrapKind::InvalidIr)?;
            let (dc, ds, sc, ss, count) =
                (dc.bits(), ds.bits(), sc.bits(), ss.bits(), count.bits());
            if destination.byte_in_element != 0
                || source.byte_in_element != 0
                || destination.index != 0
                || source.index != 0
                || dc != target.capacity as u128
                || sc != origin.capacity as u128
            {
                return Err(TrapKind::InvalidIr);
            }
            if ds > dc || count > dc - ds || ss > sc || count > sc - ss {
                return Err(TrapKind::Bounds);
            }
            let same = Rc::ptr_eq(&target, &origin);
            // Validate every slot before transferring any ownership.
            for index in 0..count {
                if !origin
                    .slots
                    .borrow()
                    .get(&((ss + index) as u64))
                    .is_some_and(|s| s.borrow().is_some())
                {
                    return Err(TrapKind::InvalidIr);
                }
                let dest = ds + index;
                if !(same && dest >= ss && dest < ss + count)
                    && target
                        .slots
                        .borrow()
                        .get(&(dest as u64))
                        .is_some_and(|s| s.borrow().is_some())
                {
                    return Err(TrapKind::InvalidIr);
                }
            }
            if same && ds == ss {
                return Ok(Value::Unit);
            }
            let values: Vec<_> = (0..count)
                .map(|i| {
                    origin
                        .slots
                        .borrow()
                        .get(&((ss + i) as u64))
                        .unwrap()
                        .borrow_mut()
                        .take()
                        .unwrap()
                })
                .collect();
            for (i, value) in values.into_iter().enumerate() {
                let mut slots = target.slots.borrow_mut();
                let slot = slots.entry((ds + i as u128) as u64).or_insert_with(|| {
                    typed_slot((*destination.storage_type).clone(), target.clock.clone())
                });
                slot.0
                    .raw_origin
                    .replace(Some(destination.clone().offset((ds + i as u128) as i128)?));
                slot.initialize()?;
                *slot.borrow_mut() = Some(value);
            }
            return Ok(Value::Unit);
        }
        let (Some(Value::Pointer(pointer)), Some(Value::Integer(index))) =
            (args.first(), args.get(1))
        else {
            return Err(TrapKind::InvalidIr);
        };
        if op == "raw_take" {
            return pointer.clone().offset(index.bits() as i128)?.take();
        }
        if op == "raw_initialize" {
            if args.len() != 3 {
                return Err(TrapKind::InvalidIr);
            }
            let pointer = pointer.clone().offset(index.bits() as i128)?;
            return pointer.initialize(args.pop().unwrap());
        }
        let heap = pointer
            .heap
            .as_ref()
            .and_then(|h| h.upgrade())
            .ok_or(TrapKind::InvalidIr)?;
        if pointer.byte_in_element != 0 || pointer.index != 0 {
            return Err(TrapKind::InvalidIr);
        }
        let index = u64::try_from(index.bits()).map_err(|_| TrapKind::Bounds)?;
        if matches!(op, "storage_address" | "storage_address_mut") {
            let [_, _, Value::Integer(offset)] = args.as_slice() else {
                return Err(TrapKind::InvalidIr);
            };
            if index != heap.capacity || offset.bits() > index as u128 {
                return Err(TrapKind::Bounds);
            }
            return pointer
                .clone()
                .offset(offset.bits() as i128)
                .map(Value::Pointer);
        }
        if op == "storage_check_range" {
            let [_, _, Value::Integer(start), Value::Integer(count)] = args.as_slice() else {
                return Err(TrapKind::InvalidIr);
            };
            if index != heap.capacity
                || start.bits() > index as u128
                || count.bits() > index as u128 - start.bits()
            {
                return Err(TrapKind::Bounds);
            }
            return Ok(Value::Unit);
        }
        if op == "storage_adopt" && pointer.storage_type != pointer.view_type {
            return Err(TrapKind::InvalidIr);
        }
        if matches!(op, "storage_detach" | "storage_adopt") {
            let detaching = op == "storage_detach";
            if index != heap.capacity || heap.owned.get() != detaching {
                return Err(TrapKind::InvalidIr);
            }
            heap.owned.set(!detaching);
            return if detaching {
                Ok(Value::Aggregate(args))
            } else {
                Ok(Value::Unit)
            };
        }
        if op == "storage_release" {
            if index != heap.capacity
                || !heap.owned.get()
                || heap.slots.borrow().values().any(|s| s.borrow().is_some())
            {
                return Err(TrapKind::InvalidIr);
            }
            self.heaps.retain(|h| !Rc::ptr_eq(h, &heap));
            return Ok(Value::Unit);
        }
        if index >= heap.capacity {
            return Err(TrapKind::Bounds);
        }
        let mut slots = heap.slots.borrow_mut();
        match op {
            "storage_initialize" => {
                if slots.get(&index).is_some_and(|s| s.borrow().is_some()) || args.len() != 3 {
                    return Err(TrapKind::InvalidIr);
                }
                let slot = slots.entry(index).or_insert_with(|| {
                    typed_slot((*pointer.storage_type).clone(), heap.clock.clone())
                });
                slot.0
                    .raw_origin
                    .replace(Some(pointer.clone().offset(i128::from(index))?));
                slot.initialize()?;
                *slot.borrow_mut() = Some(args.pop().unwrap());
                Ok(Value::Unit)
            }
            "storage_replace" => {
                let target = slots.get_mut(&index).ok_or(TrapKind::InvalidIr)?;
                if args.len() != 3 {
                    return Err(TrapKind::InvalidIr);
                }
                target
                    .borrow_mut()
                    .replace(args.pop().unwrap())
                    .ok_or(TrapKind::InvalidIr)
            }
            "storage_take" => slots
                .get(&index)
                .ok_or(TrapKind::InvalidIr)?
                .borrow_mut()
                .take()
                .ok_or(TrapKind::InvalidIr),
            "storage_drop" => {
                slots
                    .get(&index)
                    .ok_or(TrapKind::InvalidIr)?
                    .borrow_mut()
                    .take()
                    .ok_or(TrapKind::InvalidIr)?;
                Ok(Value::Unit)
            }
            _ => Err(TrapKind::InvalidIr),
        }
    }
}

// Only padding-free, provenance-free payloads cross this byte representation.
// Native code operates directly on the original address; this is diagnostic-only.
fn bits_bytes(value: Value, ty: &Type) -> Result<Vec<u8>, TrapKind> {
    if !ty.raw_bits_value() {
        return Err(TrapKind::InvalidIr);
    }
    if let Value::Enum { variant, fields } = value {
        let Type::Enum { variants, .. } = ty else {
            return Err(TrapKind::InvalidIr);
        };
        let definition = variants.get(variant).ok_or(TrapKind::InvalidIr)?;
        if definition.fields.len() != fields.len() {
            return Err(TrapKind::InvalidIr);
        }
        let mut bytes = (variant as u32).to_ne_bytes().to_vec();
        for (value, ty) in fields.into_iter().zip(&definition.fields) {
            bytes.extend(bits_bytes(value, ty)?);
        }
        return Ok(bytes);
    }
    if let Value::Aggregate(values) = value {
        let mut types = bits_fields(ty).ok_or(TrapKind::InvalidIr)?;
        let mut bytes = Vec::new();
        for value in values {
            bytes.extend(bits_bytes(value, types.next().ok_or(TrapKind::InvalidIr)?)?);
        }
        if types.next().is_some() {
            return Err(TrapKind::InvalidIr);
        }
        return Ok(bytes);
    }
    let bits = match value {
        Value::Integer(n) if *ty == Type::Integer(n.ty()) => n.bits(),
        Value::F32(n) if *ty == Type::F32 => u128::from(n.to_bits()),
        Value::F64(n) if *ty == Type::F64 => u128::from(n.to_bits()),
        Value::Bool(n) if *ty == Type::Bool => u128::from(n),
        Value::Char(n) if *ty == Type::Char => u128::from(n as u32),
        _ => return Err(TrapKind::InvalidIr),
    };
    let size = crate::layout::layout(ty)
        .map_err(|_| TrapKind::InvalidIr)?
        .size() as usize;
    let bytes = bits.to_ne_bytes();
    Ok(if cfg!(target_endian = "little") {
        bytes[..size].to_vec()
    } else {
        bytes[16 - size..].to_vec()
    })
}
fn bits_fields(ty: &Type) -> Option<Box<dyn Iterator<Item = &Type> + '_>> {
    match ty {
        Type::Array(element, count) => {
            Some(Box::new(std::iter::repeat_n(&**element, *count as usize)))
        }
        Type::Tuple(fields) => Some(Box::new(fields.iter())),
        Type::Record {
            copy: true, fields, ..
        } => Some(Box::new(fields.iter().map(|f| &f.ty))),
        _ => None,
    }
}
fn bits_value(bytes: &[u8], ty: &Type) -> Result<Value, TrapKind> {
    if !ty.raw_bits_value() {
        return Err(TrapKind::InvalidIr);
    }
    let size = crate::layout::layout(ty)
        .map_err(|_| TrapKind::InvalidIr)?
        .size() as usize;
    if size != bytes.len() {
        return Err(TrapKind::InvalidIr);
    }
    if let Type::Enum { variants, .. } = ty {
        let tag = u32::from_ne_bytes(
            bytes
                .get(..4)
                .ok_or(TrapKind::InvalidIr)?
                .try_into()
                .map_err(|_| TrapKind::InvalidIr)?,
        ) as usize;
        let variant = variants.get(tag).ok_or(TrapKind::InvalidIr)?;
        let mut offset = 4;
        let mut fields = Vec::new();
        for ty in &variant.fields {
            let size = crate::layout::layout(ty)
                .map_err(|_| TrapKind::InvalidIr)?
                .size() as usize;
            fields.push(bits_value(
                bytes
                    .get(offset..offset + size)
                    .ok_or(TrapKind::InvalidIr)?,
                ty,
            )?);
            offset += size;
        }
        return Ok(Value::Enum {
            variant: tag,
            fields,
        });
    }
    if let Some(types) = bits_fields(ty) {
        let mut offset = 0;
        let mut values = Vec::new();
        for ty in types {
            let size = crate::layout::layout(ty)
                .map_err(|_| TrapKind::InvalidIr)?
                .size() as usize;
            values.push(bits_value(
                bytes
                    .get(offset..offset + size)
                    .ok_or(TrapKind::InvalidIr)?,
                ty,
            )?);
            offset += size;
        }
        return Ok(Value::Aggregate(values));
    }
    if size > 16 {
        return Err(TrapKind::InvalidIr);
    }
    let mut padded = [0; 16];
    if cfg!(target_endian = "little") {
        padded[..size].copy_from_slice(bytes)
    } else {
        padded[16 - size..].copy_from_slice(bytes)
    }
    let bits = u128::from_ne_bytes(padded);
    Ok(match ty {
        Type::Integer(ty) => Value::Integer(nether_semantics::Integer::from_bits(*ty, bits)),
        Type::F32 => Value::F32(f32::from_bits(bits as u32)),
        Type::F64 => Value::F64(f64::from_bits(bits as u64)),
        Type::Bool if bits <= 1 => Value::Bool(bits == 1),
        Type::Char => Value::Char(char::from_u32(bits as u32).ok_or(TrapKind::InvalidIr)?),
        _ => return Err(TrapKind::InvalidIr),
    })
}
