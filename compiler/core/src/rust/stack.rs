//! Conservative frame allocation: closed scalar/class programs only. No class
//! escapes through results, owned parameters, aggregates, function pointers or
//! identity observations. A slot is reused only after every owner is dead.
use super::*;
use std::collections::{BTreeSet, VecDeque};
fn scalar(t: &Type) -> bool {
    matches!(
        t,
        Type::Unit | Type::Bool | Type::Char | Type::Integer(_) | Type::F32 | Type::F64
    )
}
pub(super) fn eligible(p: &Program) -> bool {
    p.classes
        .values()
        .all(|c| !c.copy && c.fields.len() <= 16 && c.fields.iter().all(|f| scalar(&f.ty)))
        && p.functions.iter().all(|f| {
            scalar(&f.result)
                && f.locals
                    .iter()
                    .all(|l| scalar(&l.ty) || matches!(l.ty, Type::Class { .. }))
                && f.parameters
                    .iter()
                    .all(|l| scalar(&f.locals[*l].ty) || f.locals[*l].view.is_some())
                && f.locals.iter().all(|l| {
                    !matches!(l.ty, Type::Class { .. }) || p.destructors.entry(&l.ty).is_none()
                })
                && f.blocks.iter().all(|b| match &b.terminator {
                    Terminator::Evaluate {
                        operation: Operation::Identity(..) | Operation::CallIndirect(..),
                        ..
                    } => false,
                    Terminator::Evaluate {
                        operation: Operation::Copy(_),
                        destination,
                        ..
                    } => !matches!(f.locals[destination.local].ty, Type::Class { .. }),
                    _ => true,
                })
        })
}
pub(super) fn slots(f: &Function) -> BTreeSet<usize> {
    let owner =
        |l: usize| f.locals[l].view.is_none() && matches!(f.locals[l].ty, Type::Class { .. });
    let mut incoming = vec![None; f.blocks.len()];
    incoming[f.entry] = Some(BTreeSet::<usize>::new());
    let mut queue = VecDeque::from([f.entry]);
    while let Some(id) = queue.pop_front() {
        let term = &f.blocks[id].terminator;
        let mut live = incoming[id].clone().unwrap();
        if let Terminator::EndStorage { locals, .. } = term {
            for l in locals {
                live.remove(l);
            }
        }
        for successor in term.successors() {
            let mut next_live = live.clone();
            if let Terminator::Evaluate {
                operation,
                destination,
                next,
                ..
            } = term
            {
                if successor == *next {
                    for p in mir::dataflow::consumed(f, operation) {
                        if p.projections.is_empty() {
                            next_live.remove(&p.local);
                        }
                    }
                    if owner(destination.local) {
                        next_live.insert(destination.local);
                    }
                }
            }
            match &mut incoming[successor] {
                Some(old) => {
                    let n = old.len();
                    old.extend(next_live);
                    if n != old.len() {
                        queue.push_back(successor);
                    }
                }
                slot @ None => {
                    *slot = Some(next_live);
                    queue.push_back(successor);
                }
            }
        }
    }
    f.blocks
        .iter()
        .enumerate()
        .filter_map(|(id, b)| match &b.terminator {
            Terminator::Evaluate {
                operation: Operation::AllocateClass(_),
                destination,
                ..
            } if incoming[id].as_ref().is_some_and(|s| s.is_empty()) => Some(destination.local),
            _ => None,
        })
        .take(16)
        .collect()
}
