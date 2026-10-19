//! Implicit reads made by user destructors, and possible root cleanup orders.
//! The order domain stores may-before pairs, so joins and loops never assume a
//! favorable initialization order. The runtime still uses its dynamic root list.
use super::*;
use std::collections::{BTreeSet, VecDeque};

pub(super) fn reads(ty: &Type, destructors: &drop_plan::Destructors) -> bool {
    match ty {
        Type::Record { fields, .. }
        | Type::Class {
            storage: Some(fields),
            ..
        } => {
            (ty.contains_views() && destructors.entry(ty).is_some())
                || fields.iter().any(|f| reads(&f.ty, destructors))
        }
        Type::Tuple(fields) => fields.iter().any(|ty| reads(ty, destructors)),
        Type::Array(ty, n) => *n > 0 && reads(ty, destructors),
        Type::Enum { variants, .. } => variants
            .iter()
            .flat_map(|v| &v.fields)
            .any(|ty| reads(ty, destructors)),
        _ => false,
    }
}
#[derive(Clone, Default, PartialEq, Eq)]
struct State {
    active: BTreeSet<LocalId>,
    before: BTreeSet<(LocalId, LocalId)>,
    // Dependencies published by projected writes while the holder predates its
    // target. Root initialization clears these facts even on shared unwind CFG.
    bad: BTreeSet<(LocalId, LocalId)>,
}
impl State {
    fn forget(&mut self, local: LocalId) {
        self.active.remove(&local);
        self.before.retain(|(a, b)| *a != local && *b != local);
        self.bad.retain(|(a, b)| *a != local && *b != local);
    }
    fn initialize(&mut self, local: LocalId) {
        self.forget(local);
        for other in &self.active {
            self.before.insert((*other, local));
        }
        self.active.insert(local);
    }
    fn join(&mut self, other: &Self) -> bool {
        let old = self.clone();
        self.active.extend(&other.active);
        self.before.extend(&other.before);
        self.bad.extend(&other.bad);
        *self != old
    }
}
pub(super) struct Orders {
    incoming: Vec<Option<State>>,
}
impl Orders {
    pub(super) fn cleans_before(
        &self,
        f: &Function,
        block: BlockId,
        holder: LocalId,
        target: LocalId,
    ) -> bool {
        if holder == target {
            return false;
        } // Internal owning-group cycles belong to C7.
        let a = f.cleanup.owners[holder];
        let b = f.cleanup.owners[target];
        if a != b {
            return a > b;
        }
        if f.cleanup.construction.contains(&a) {
            return f.cleanup.field_order.get(&holder).unwrap_or(&holder)
                < f.cleanup.field_order.get(&target).unwrap_or(&target);
        }
        self.incoming[block]
            .as_ref()
            .is_some_and(|s| !s.bad.contains(&(holder, target)))
    }
}
// These operations finish producing the staged value before destination cleanup.
// On unwind the old destination is already dead and the staged owner is cleaned.
pub(super) fn stages_on_unwind(operation: &Operation) -> bool {
    matches!(
        operation,
        Operation::Use(_)
            | Operation::Move(_)
            | Operation::Copy(_)
            | Operation::Aggregate(_)
            | Operation::Enum(..)
            | Operation::CompleteClass(_)
    )
}
pub(super) fn orders(f: &Function, provenance: &borrow_check::Provenance) -> Orders {
    let flow = reachability::analyze(f);
    let mut incoming = vec![None::<State>; f.blocks.len()];
    let mut initial = State::default();
    for p in &f.parameters {
        if f.locals[*p].view.is_none() {
            initial.initialize(*p);
        }
    }
    incoming[f.entry] = Some(initial);
    let mut queue = VecDeque::from([f.entry]);
    while let Some(id) = queue.pop_front() {
        let input = incoming[id].clone().unwrap();
        let term = &f.blocks[id].terminator;
        for next in &flow.successors[id] {
            let mut out = input.clone();
            match term {
                Terminator::Evaluate {
                    operation,
                    destination,
                    next: success,
                    ..
                } => {
                    if *next == *success
                        || storage_writes::call(operation).is_some()
                        || matches!(operation, Operation::CallIndirect(..))
                        || stages_on_unwind(operation)
                    {
                        for source in dataflow::consumed(f, operation) {
                            let targets = provenance.places(id, &source);
                            if targets.len() == 1 {
                                for target in targets {
                                    if target.projections.is_empty() {
                                        out.forget(target.local);
                                    }
                                }
                            }
                        }
                    }
                    if *next != *success && stages_on_unwind(operation) {
                        let targets = provenance.places(id, destination);
                        if targets.len() == 1 {
                            for target in targets {
                                if target.projections.is_empty() {
                                    out.forget(target.local)
                                }
                            }
                        }
                    }
                    if *next == *success {
                        let targets = provenance.places(id, destination);
                        if targets.len() == 1 {
                            for target in targets {
                                if target.projections.is_empty()
                                    && f.locals[target.local].view.is_none()
                                {
                                    out.initialize(target.local);
                                }
                            }
                        } else {
                            // A may-alias write may reinitialize any target. Keep
                            // both possible orders, including the unchanged path.
                            for target in targets {
                                if target.projections.is_empty() {
                                    let mut changed = out.clone();
                                    changed.initialize(target.local);
                                    out.join(&changed);
                                }
                            }
                        }
                    }
                    let mut written = BTreeSet::new();
                    if *next == *success {
                        for target in provenance.places(id, destination) {
                            if !target.projections.is_empty() {
                                written.insert(target.local);
                            }
                        }
                    }
                    if let Some((arguments, writes)) = storage_writes::effects(f, operation) {
                        for write in writes {
                            if let Some(Operand::Address(argument)) =
                                arguments.get(write.target.parameter)
                            {
                                let mut slot = argument.clone();
                                slot.projections.extend_from_slice(&write.target.path);
                                for target in provenance.places(id, &slot) {
                                    written.insert(target.local);
                                }
                            }
                        }
                    }
                    for holder in written {
                        for origins in provenance.components(*next, &Place::local(holder)).values()
                        {
                            for target in origins {
                                if out.before.contains(&(holder, target.local)) {
                                    out.bad.insert((holder, target.local));
                                }
                            }
                        }
                    }
                }
                Terminator::EndStorage { locals, .. } => {
                    for local in locals {
                        out.forget(*local)
                    }
                }
                _ => {}
            }
            let changed = match &mut incoming[*next] {
                Some(old) => old.join(&out),
                slot @ None => {
                    *slot = Some(out);
                    true
                }
            };
            if changed {
                queue.push_back(*next);
            }
        }
    }
    Orders { incoming }
}
