//! Flow-sensitive origins and last-use loan checking for local safe views.
//! View slots carry pointers, while their logical type remains the target type.
use super::{
    dataflow,
    loans::{overlaps, Access},
    *,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
type Bindings = BTreeMap<LocalId, BTreeSet<BlockId>>;
#[derive(Clone, Default, PartialEq, Eq)]
struct Origin {
    places: BTreeSet<Place>,
    parents: BTreeSet<BlockId>,
    ancestors: BTreeSet<BlockId>,
}
#[derive(Clone, Debug)]
pub struct Error {
    pub block: BlockId,
    pub message: &'static str,
}
fn resolve(place: &Place, bindings: &Bindings, origins: &[Origin]) -> Origin {
    if matches!(place.projections.first(), Some(Projection::Dereference)) {
        let mut result = Origin::default();
        let mut ancestors: Option<BTreeSet<BlockId>> = None;
        for loan in bindings.get(&place.local).into_iter().flatten() {
            let mut through = origins[*loan].ancestors.clone();
            through.insert(*loan);
            ancestors = Some(match ancestors {
                Some(old) => old.intersection(&through).copied().collect(),
                None => through,
            });
            result.parents.insert(*loan);
            result.parents.extend(&origins[*loan].parents);
            for source in &origins[*loan].places {
                let mut source = source.clone();
                source
                    .projections
                    .extend_from_slice(&place.projections[1..]);
                result.places.insert(source);
            }
        }
        result.ancestors = ancestors.unwrap_or_default();
        result
    } else {
        Origin {
            places: BTreeSet::from([place.clone()]),
            parents: BTreeSet::new(),
            ancestors: BTreeSet::new(),
        }
    }
}
fn merge(target: &mut Bindings, source: &Bindings) -> bool {
    let mut changed = false;
    for (local, loans) in source {
        let entry = target.entry(*local).or_default();
        let before = entry.len();
        entry.extend(loans);
        changed |= entry.len() != before;
    }
    changed
}
pub fn check(function: &Function) -> Vec<Error> {
    if function.locals.iter().all(|local| local.view.is_none()) {
        return Vec::new();
    }
    let count = function.blocks.len();
    let mut incoming = vec![None::<Bindings>; count];
    let mut initial = Bindings::new();
    let mut origins = vec![Origin::default(); count];
    let mut parameter_loans = Vec::new();
    for parameter in &function.parameters {
        if let Some(access) = function.locals[*parameter].view {
            let id = origins.len();
            initial.insert(*parameter, BTreeSet::from([id]));
            origins.push(Origin {
                places: BTreeSet::from([Place::local(*parameter)]),
                ..Origin::default()
            });
            parameter_loans.push(access);
        }
    }
    incoming[function.entry] = Some(initial);
    // Revisit all reachable blocks when a loan's resolved provenance expands.
    loop {
        let mut changed_origins = false;
        let mut queue = VecDeque::from([function.entry]);
        let mut visited = BTreeSet::new();
        while let Some(id) = queue.pop_front() {
            let input = incoming[id].clone().unwrap_or_default();
            let mut success = input.clone();
            let term = &function.blocks[id].terminator;
            if let Terminator::Evaluate {
                operation,
                destination,
                ..
            } = term
            {
                if let Operation::Borrow(source) | Operation::CapturePlace(source) = operation {
                    let resolved = resolve(source, &input, &origins);
                    if origins[id] != resolved {
                        origins[id] = resolved;
                        changed_origins = true;
                    }
                    success.insert(destination.local, BTreeSet::from([id]));
                } else if destination.projections.is_empty() {
                    success.remove(&destination.local);
                }
            }
            if let Terminator::EndStorage { locals, .. } = term {
                for local in locals {
                    success.remove(local);
                }
            }
            visited.insert(id);
            for next in term.successors() {
                let output = match term {
                    Terminator::Evaluate { next: ok, .. } if *ok == next => &success,
                    Terminator::EndStorage { .. } => &success,
                    _ => &input,
                };
                let changed = match &mut incoming[next] {
                    Some(state) => merge(state, output),
                    slot @ None => {
                        *slot = Some(output.clone());
                        true
                    }
                };
                if changed || !visited.contains(&next) {
                    queue.push_back(next);
                }
            }
        }
        if !changed_origins {
            break;
        }
    }
    let live = dataflow::liveness(function);
    let mut errors = Vec::new();
    for (id, block) in function.blocks.iter().enumerate() {
        if origins[id].parents.contains(&id) {
            errors.push(Error {
                block: id,
                message: "cyclic reborrowing across loop iterations is not yet supported",
            });
        }
        let Some(bindings) = &incoming[id] else {
            continue;
        };
        let active: BTreeSet<_> = live.live_in[id]
            .iter()
            .flat_map(|local| bindings.get(local).into_iter().flatten().copied())
            .collect();
        if let Terminator::EndStorage { locals, .. } = &block.terminator {
            for loan in &active {
                if origins[*loan]
                    .places
                    .iter()
                    .any(|p| locals.contains(&p.local))
                {
                    errors.push(Error {
                        block: id,
                        message: "view outlives its local target",
                    });
                }
            }
        }
        let mut access = |place: &Place, kind: Access, mutation: bool| {
            let resolved = resolve(place, bindings, &origins);
            if resolved.places.is_empty() {
                errors.push(Error {
                    block: id,
                    message: "view has no initialized origin",
                });
                return;
            }
            if mutation {
                let mutable = if matches!(place.projections.first(), Some(Projection::Dereference))
                {
                    function.locals[place.local].view == Some(Access::Mutable)
                } else {
                    function.locals[place.local].mutable
                };
                if !mutable {
                    errors.push(Error {
                        block: id,
                        message: "mutable access requires a mutable origin",
                    });
                }
            }
            for loan in &active {
                if resolved.ancestors.contains(loan) {
                    continue;
                }
                let capability = if *loan >= count {
                    Some(parameter_loans[*loan - count])
                } else {
                    let Terminator::Evaluate {
                        destination,
                        operation,
                        ..
                    } = &function.blocks[*loan].terminator
                    else {
                        unreachable!()
                    };
                    if matches!(operation, Operation::CapturePlace(_)) {
                        continue;
                    }
                    function.locals[destination.local].view
                };
                if (kind == Access::Mutable || capability == Some(Access::Mutable))
                    && resolved
                        .places
                        .iter()
                        .any(|a| origins[*loan].places.iter().any(|b| overlaps(a, b)))
                {
                    errors.push(Error {
                        block: id,
                        message: "access conflicts with a view that is used later",
                    });
                    break;
                }
            }
        };
        if let Terminator::Evaluate {
            operation,
            destination,
            ..
        } = &block.terminator
        {
            match operation {
                Operation::Borrow(source) => access(
                    source,
                    function.locals[destination.local]
                        .view
                        .unwrap_or(Access::Readonly),
                    function.locals[destination.local].view == Some(Access::Mutable),
                ),
                Operation::Copy(source) | Operation::TagPlace(source) => {
                    access(source, Access::Readonly, false)
                }
                Operation::Move(source) => access(source, Access::Mutable, false),
                _ => (),
            }
            if matches!(operation, Operation::Move(Place { projections, .. }) if projections.first() == Some(&Projection::Dereference))
            {
                errors.push(Error {
                    block: id,
                    message: "cannot move ownership out of a borrowed view",
                });
            }
            // Root writes initialize/rebind a slot, not necessarily a mutable binding.
            // They still end the previous owner's lifetime and conflict with live loans.
            if !matches!(operation, Operation::Borrow(_) | Operation::CapturePlace(_)) {
                let resolved = resolve(destination, bindings, &origins);
                for loan in &active {
                    if *loan < count
                        && matches!(
                            &function.blocks[*loan].terminator,
                            Terminator::Evaluate {
                                operation: Operation::CapturePlace(_),
                                ..
                            }
                        )
                    {
                        continue;
                    }
                    if !resolved.ancestors.contains(loan)
                        && resolved
                            .places
                            .iter()
                            .any(|a| origins[*loan].places.iter().any(|b| overlaps(a, b)))
                    {
                        errors.push(Error {
                            block: id,
                            message: "replacement conflicts with a view that is used later",
                        });
                        break;
                    }
                }
                if matches!(
                    destination.projections.first(),
                    Some(Projection::Dereference)
                ) && function.locals[destination.local].view != Some(Access::Mutable)
                {
                    errors.push(Error {
                        block: id,
                        message: "cannot write through a readonly view",
                    });
                }
            }
        }
    }
    errors
}
