//! Definite constructor field initialization on successful typed MIR paths.
//! Borrow aliases carry provenance; taking the address of an uninitialized field
//! for assignment is distinct from reading or exposing its contents.
use super::*;
use crate::mir::{self, Operand, Operation, Place, Projection, Terminator};
use std::collections::{BTreeSet, VecDeque};

pub(super) fn validate(program: &mir::Program) -> Result<()> {
    for function in &program.functions {
        if !function
            .name
            .rsplit_once(".$init")
            .is_some_and(|(_, s)| s.is_empty() || s.starts_with("$g") || s.starts_with("$entry"))
        {
            continue;
        }
        let receiver = function.parameters[0];
        let Type::Class { .. } = &function.locals[receiver].ty else {
            continue;
        };
        let fields = &program.classes[&function.locals[receiver].ty.class_key()].fields;
        let origins = mir::borrow_check::provenance(function);
        let resolve = |block, place: &Place| origins.places(block, place);
        let mut incoming = vec![None::<BTreeSet<usize>>; function.blocks.len()];
        incoming[function.entry] = Some(BTreeSet::new());
        let mut queue = VecDeque::from([function.entry]);
        while let Some(block) = queue.pop_front() {
            let mut state = incoming[block].clone().unwrap();
            let term = &function.blocks[block].terminator;
            if let Terminator::Evaluate {
                operation,
                destination,
                ..
            } = term
            {
                if !matches!(
                    operation,
                    Operation::Borrow(_) | Operation::CapturePlace(_) | Operation::LoadView(_)
                ) {
                    let targets = resolve(block, destination);
                    if targets.len() == 1 {
                        let target = targets.first().unwrap();
                        if target.local == receiver {
                            if let [Projection::Field(index)] = target.projections.as_slice() {
                                state.insert(*index);
                            }
                        }
                    }
                }
            }
            let successors = match term {
                Terminator::Evaluate { next, .. }
                | Terminator::CheckPlace { next, .. }
                | Terminator::EndStorage { next, .. }
                | Terminator::Goto(next) => vec![*next],
                Terminator::Branch { yes, no, .. } => vec![*yes, *no],
                _ => vec![],
            };
            for next in successors {
                let joined = match &incoming[next] {
                    Some(old) => old.intersection(&state).copied().collect(),
                    None => state.clone(),
                };
                if incoming[next].as_ref() != Some(&joined) {
                    incoming[next] = Some(joined);
                    queue.push_back(next);
                }
            }
        }
        for (block, state) in incoming.iter().enumerate() {
            let Some(state) = state else {
                continue;
            };
            let span = function.blocks[block].span;
            let complete = || {
                if state.len() == fields.len() {
                    Ok(())
                } else {
                    let missing = fields
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| !state.contains(i))
                        .map(|(_, f)| f.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    Err(error(span,"E0393",format!("constructor target is not fully initialized; missing fields: {missing}")))
                }
            };
            // address=true permits forming a direct assignment target. Traversing
            // farther into its payload always requires its initialization first.
            let require = |place: &Place, address: bool, alias: bool| -> Result<()> {
                for target in resolve(block, place) {
                    if target.local != receiver {
                        continue;
                    }
                    match target.projections.as_slice() {
                        [] if !address && !alias => complete()?,
                        [Projection::Field(index), tail @ ..] if !address || !tail.is_empty() => {
                            if !state.contains(index) {
                                return Err(error(
                                    span,
                                    "E0393",
                                    format!(
                                        "constructor reads uninitialized field '{}'",
                                        fields[*index].name
                                    ),
                                ));
                            }
                        }
                        _ => {}
                    }
                }
                Ok(())
            };
            let argument = |value: &Operand| -> Result<()> {
                let place = match value {
                    Operand::Address(p) => {
                        require(p, false, false)?;
                        p.clone()
                    }
                    Operand::Local(p) => Place::local(*p),
                    Operand::Constant(_) => return Ok(()),
                };
                for targets in origins.components(block, &place).values() {
                    for target in targets {
                        require(target, false, false)?;
                    }
                }
                Ok(())
            };
            match &function.blocks[block].terminator {
                Terminator::Return(_) => complete()?,
                Terminator::CheckPlace { place, .. } => require(place, true, false)?,
                Terminator::Evaluate {
                    operation,
                    destination,
                    ..
                } => {
                    require(destination, true, false)?;
                    match operation {
                        Operation::Identity(a, b) => {
                            require(a, false, false)?;
                            require(b, false, false)?;
                        }
                        Operation::Borrow(p) | Operation::StoreView(p) => require(p, false, true)?,
                        Operation::CapturePlace(p) => require(p, true, false)?,
                        Operation::Copy(p)
                        | Operation::Move(p)
                        | Operation::TagPlace(p)
                        | Operation::LoadView(p) => require(p, false, false)?,
                        Operation::Call(_, args)
                        | Operation::CallIndirect(_, args, _)
                        | Operation::CallStorage {
                            arguments: args, ..
                        }
                        | Operation::CallView {
                            arguments: args, ..
                        } => {
                            for a in args {
                                argument(a)?;
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}
