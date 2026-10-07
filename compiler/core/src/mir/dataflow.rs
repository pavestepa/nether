//! Backward liveness with distinct successful and panic edges.
use super::*;
use std::collections::BTreeSet;
pub type Locals = BTreeSet<LocalId>;
#[derive(Debug)]
pub struct Liveness {
    pub live_in: Vec<Locals>,
    pub live_out: Vec<Locals>,
}
pub fn liveness(function: &Function) -> Liveness {
    let mut input = vec![Locals::new(); function.blocks.len()];
    let mut output = input.clone();
    loop {
        let mut changed = false;
        for (id, block) in function.blocks.iter().enumerate().rev() {
            let mut live = Locals::new();
            for next in block.terminator.successors() {
                live.extend(&input[next]);
            }
            output[id] = live.clone();
            if let Terminator::Evaluate {
                destination,
                next,
                unwind,
                ..
            } = &block.terminator
            {
                live = input[*next].clone();
                if destination.projections.is_empty() {
                    live.remove(&destination.local);
                }
                live.extend(&input[*unwind]);
            }
            if let Terminator::EndStorage { locals, .. } = &block.terminator {
                for local in locals {
                    live.remove(local);
                }
            }
            live.extend(uses(&block.terminator));
            if input[id] != live {
                input[id] = live;
                changed = true;
            }
        }
        if !changed {
            return Liveness {
                live_in: input,
                live_out: output,
            };
        }
    }
}
pub fn uses(terminator: &Terminator) -> Locals {
    let mut result = Locals::new();
    match terminator {
        Terminator::Evaluate {
            operation,
            destination,
            ..
        } => {
            if !destination.projections.is_empty() {
                place_uses(destination, &mut result);
            }
            match operation {
                Operation::TagPlace(place)
                | Operation::Borrow(place)
                | Operation::CapturePlace(place)
                | Operation::Copy(place)
                | Operation::Move(place) => place_uses(place, &mut result),
                Operation::Use(value)
                | Operation::Unary(_, value)
                | Operation::Cast(value, _)
                | Operation::Tag(value) => operand_uses(value, &mut result),
                Operation::Binary(_, a, b) => {
                    operand_uses(a, &mut result);
                    operand_uses(b, &mut result);
                }
                Operation::Aggregate(values)
                | Operation::Enum(_, values)
                | Operation::Call(_, values) => {
                    for value in values {
                        operand_uses(value, &mut result);
                    }
                }
            }
        }
        Terminator::CheckPlace { place, .. } => place_uses(place, &mut result),
        Terminator::Branch { condition, .. } | Terminator::Return(condition) => {
            operand_uses(condition, &mut result)
        }
        _ => (),
    }
    result
}
fn operand_uses(value: &Operand, output: &mut Locals) {
    match value {
        Operand::Local(local) => {
            output.insert(*local);
        }
        Operand::Address(place) => place_uses(place, output),
        _ => (),
    }
}
fn place_uses(place: &Place, output: &mut Locals) {
    output.insert(place.local);
    for projection in &place.projections {
        if let Projection::Index(local) = projection {
            output.insert(*local);
        }
    }
}

/// Required value reads, as distinct from merely forming an assignment address.
pub fn required_initialized(terminator: &Terminator) -> Vec<Place> {
    let mut places = Vec::new();
    let mut add_operand = |operand: &Operand| match operand {
        Operand::Local(local) => places.push(Place::local(*local)),
        Operand::Address(place) => places.push(place.clone()),
        _ => (),
    };
    match terminator {
        Terminator::Evaluate { operation, .. } => match operation {
            Operation::TagPlace(place)
            | Operation::Borrow(place)
            | Operation::CapturePlace(place)
            | Operation::Copy(place)
            | Operation::Move(place) => places.push(place.clone()),
            Operation::Use(value)
            | Operation::Unary(_, value)
            | Operation::Cast(value, _)
            | Operation::Tag(value) => add_operand(value),
            Operation::Binary(_, left, right) => {
                add_operand(left);
                add_operand(right);
            }
            Operation::Aggregate(values)
            | Operation::Enum(_, values)
            | Operation::Call(_, values) => {
                for value in values {
                    add_operand(value);
                }
            }
        },
        Terminator::Branch { condition, .. } | Terminator::Return(condition) => {
            add_operand(condition)
        }
        _ => (),
    }
    let address = match terminator {
        Terminator::Evaluate { destination, .. } => Some(destination),
        Terminator::CheckPlace { place, .. } => Some(place),
        _ => None,
    };
    let mut indices = Vec::new();
    for place in places.iter().chain(address) {
        for projection in &place.projections {
            if let Projection::Index(local) = projection {
                indices.push(Place::local(*local));
            }
        }
    }
    places.extend(indices);
    for place in &mut places {
        if matches!(place.projections.first(), Some(Projection::Dereference)) {
            place.projections.clear();
        }
    }
    places
}

pub fn place_initialization_errors(function: &Function) -> Vec<(BlockId, Place)> {
    use super::initialization::{State, Status};
    let mut universe: std::collections::BTreeSet<Place> =
        (0..function.locals.len()).map(Place::local).collect();
    for block in &function.blocks {
        universe.extend(required_initialized(&block.terminator));
        if let Terminator::Evaluate { destination, .. } = &block.terminator {
            universe.insert(destination.clone());
        }
    }
    for place in universe.clone() {
        for length in 0..place.projections.len() {
            universe.insert(Place {
                local: place.local,
                projections: place.projections[..length].to_vec(),
            });
        }
    }
    let mut initial = State::default();
    for local in &function.parameters {
        initial.initialize(Place::local(*local));
    }
    let mut incoming: Vec<Option<State>> = vec![None; function.blocks.len()];
    incoming[function.entry] = Some(initial);
    let mut queue = std::collections::VecDeque::from([function.entry]);
    while let Some(id) = queue.pop_front() {
        let state = incoming[id].as_ref().unwrap().clone();
        for next in function.blocks[id].terminator.successors() {
            let mut output = state.clone();
            if let Terminator::EndStorage { locals, .. } = &function.blocks[id].terminator {
                for local in locals {
                    output.set(Place::local(*local), Status::Uninitialized);
                }
            }
            if let Terminator::Evaluate {
                operation,
                destination,
                next: success,
                ..
            } = &function.blocks[id].terminator
            {
                if *success == next || matches!(operation, Operation::Call(..)) {
                    for source in consumed(function, operation) {
                        output.set(source, Status::Uninitialized);
                    }
                }
                if *success == next {
                    output.initialize(destination.clone());
                }
            }
            let updated = match &incoming[next] {
                Some(old) => old.join(&output, universe.iter().cloned()),
                None => output,
            };
            let changed = incoming[next].as_ref().is_none_or(|old| {
                universe
                    .iter()
                    .any(|place| old.status(place) != updated.status(place))
            });
            if changed {
                incoming[next] = Some(updated);
                queue.push_back(next);
            }
        }
    }
    let mut errors = Vec::new();
    for (id, state) in incoming.iter().enumerate() {
        if let Some(state) = state {
            for place in required_initialized(&function.blocks[id].terminator) {
                if state.status(&place) != Status::Initialized {
                    errors.push((id, place));
                }
            }
        }
    }
    errors
}
pub fn initialization_errors(function: &Function) -> Vec<(BlockId, LocalId)> {
    place_initialization_errors(function)
        .into_iter()
        .map(|(block, place)| (block, place.local))
        .collect()
}

/// Value operands transfer non-Copy ownership into the receiving operation.
pub fn consumed(function: &Function, operation: &Operation) -> Vec<Place> {
    let mut places = Vec::new();
    let mut operand = |value: &Operand| {
        if let Operand::Local(local) = value {
            if !function.locals[*local].ty.copyable() {
                places.push(Place::local(*local));
            }
        }
    };
    match operation {
        Operation::Move(source) => places.push(source.clone()),
        Operation::Use(value) => operand(value),
        Operation::Aggregate(values) | Operation::Enum(_, values) | Operation::Call(_, values) => {
            values.iter().for_each(operand)
        }
        _ => (),
    }
    places
}
