//! Proven control-flow edges for local enum discriminants and scalar constants.
//! Unknown inputs, calls and aliased writes retain all possible branches. This
//! analysis never depends on loans, so provenance can use it without a cycle.
use super::*;
use nether_semantics::{Integer, IntegerType};
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Known {
    Bool(bool),
    Integer(Integer),
    Variant(usize),
}
type State = BTreeMap<LocalId, Known>;

pub(crate) struct Flow {
    pub successors: Vec<Vec<BlockId>>,
    pub reachable: Vec<bool>,
}

fn operand(value: &Operand, state: &State) -> Option<Known> {
    match value {
        Operand::Local(local) => state.get(local).cloned(),
        Operand::Constant(Value::Bool(value)) => Some(Known::Bool(*value)),
        Operand::Constant(Value::Integer(value)) => Some(Known::Integer(*value)),
        _ => None,
    }
}
fn read(place: &Place, state: &State) -> Option<Known> {
    place
        .projections
        .is_empty()
        .then(|| state.get(&place.local).cloned())
        .flatten()
}
fn value(operation: &Operation, state: &State) -> Option<Known> {
    match operation {
        Operation::Use(value) => operand(value, state),
        Operation::Copy(place) | Operation::Move(place) => read(place, state),
        Operation::Enum(variant, _) => Some(Known::Variant(*variant)),
        Operation::Tag(value) => tag(operand(value, state)),
        Operation::TagPlace(place) => tag(read(place, state)),
        Operation::Unary("!", value) => match operand(value, state) {
            Some(Known::Bool(value)) => Some(Known::Bool(!value)),
            _ => None,
        },
        Operation::Binary(operator @ ("==" | "!="), a, b) => {
            let a = operand(a, state)?;
            let b = operand(b, state)?;
            if !matches!(
                (&a, &b),
                (Known::Bool(_), Known::Bool(_)) | (Known::Integer(_), Known::Integer(_))
            ) {
                return None;
            }
            Some(Known::Bool((a == b) == (*operator == "==")))
        }
        _ => None,
    }
}
fn tag(value: Option<Known>) -> Option<Known> {
    match value {
        Some(Known::Variant(variant)) => Some(Known::Integer(Integer::from_bits(
            IntegerType::U32,
            variant as u128,
        ))),
        _ => None,
    }
}
fn successors(term: &Terminator, state: &State) -> Vec<BlockId> {
    if let Terminator::Branch { condition, yes, no } = term {
        if let Some(Known::Bool(condition)) = operand(condition, state) {
            return vec![if condition { *yes } else { *no }];
        }
    }
    term.successors()
}
fn transfer(function: &Function, term: &Terminator, input: &State, next: BlockId) -> State {
    let mut state = input.clone();
    match term {
        Terminator::Evaluate {
            operation,
            destination,
            next: success,
            ..
        } => {
            // Calls may change any reachable target, including through aliases.
            if matches!(
                operation,
                Operation::Call(..)
                    | Operation::CallIndirect(..)
                    | Operation::CallView { .. }
                    | Operation::CallStorage { .. }
            ) {
                state.clear();
            }
            if next == *success {
                let result = value(operation, input);
                for source in dataflow::consumed(function, operation) {
                    state.remove(&source.local);
                }
                if destination.projections.is_empty()
                    && function.locals[destination.local].view.is_none()
                {
                    state.remove(&destination.local);
                    if let Some(result) = result {
                        state.insert(destination.local, result);
                    }
                } else if !destination.projections.is_empty() {
                    // Projected storage can involve class identity, captured
                    // places or dynamic indices. Do not assume distinct roots.
                    state.clear();
                }
            }
        }
        Terminator::EndStorage { locals, .. } => {
            for local in locals {
                state.remove(local);
            }
        }
        _ => {}
    }
    state
}

pub(crate) fn analyze(function: &Function) -> Flow {
    let mut incoming = vec![None::<State>; function.blocks.len()];
    incoming[function.entry] = Some(State::new());
    let mut queue = VecDeque::from([function.entry]);
    while let Some(id) = queue.pop_front() {
        let input = incoming[id].as_ref().unwrap().clone();
        let term = &function.blocks[id].terminator;
        for next in successors(term, &input) {
            let output = transfer(function, term, &input, next);
            let joined = match &incoming[next] {
                Some(old) => old
                    .iter()
                    .filter(|(local, known)| output.get(local) == Some(*known))
                    .map(|(local, known)| (*local, known.clone()))
                    .collect(),
                None => output,
            };
            if incoming[next].as_ref() != Some(&joined) {
                incoming[next] = Some(joined);
                queue.push_back(next);
            }
        }
    }
    Flow {
        successors: function
            .blocks
            .iter()
            .zip(&incoming)
            .map(|(block, state)| {
                state
                    .as_ref()
                    .map_or_else(Vec::new, |state| successors(&block.terminator, state))
            })
            .collect(),
        reachable: incoming.iter().map(Option::is_some).collect(),
    }
}

/// All later checks and backends must see the same proven control flow. In
/// particular, an unreachable owned return must not enter a borrowed Rust ABI.
pub(super) fn prune(function: &mut Function) {
    let flow = analyze(function);
    for (id, block) in function.blocks.iter_mut().enumerate() {
        if !flow.reachable[id] {
            block.terminator = Terminator::Unreachable;
        } else if matches!(block.terminator, Terminator::Branch { .. })
            && flow.successors[id].len() == 1
        {
            block.terminator = Terminator::Goto(flow.successors[id][0]);
        }
    }
}
