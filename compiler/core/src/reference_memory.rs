//! Host-side cells used only by the bounded reference interpreters. Clearing a
//! slot ends its language lifetime even when host handles to that cell remain.
//! Native views use raw addresses and never allocate or retain these cells.
use crate::{hir::Value, interpret::TrapKind, mir::Projection};
use std::{cell::RefCell, rc::Rc};
pub(crate) type Slot = Rc<RefCell<Option<Value>>>;
pub(crate) fn slot() -> Slot {
    Rc::new(RefCell::new(None))
}
#[derive(Clone)]
pub(crate) struct Reference {
    pub slot: Slot,
    pub projections: Vec<Projection>,
}
#[derive(Clone)]
pub(crate) enum Argument {
    Owned(Value),
    Borrowed(Reference),
}
impl Reference {
    pub fn read(&self) -> Result<Value, TrapKind> {
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
        let mut target = self.slot.borrow_mut();
        if self.projections.is_empty() {
            *target = Some(value);
            return Ok(());
        }
        write(
            target.as_mut().ok_or(TrapKind::InvalidIr)?,
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
