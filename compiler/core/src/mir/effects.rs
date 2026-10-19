//! Effects on borrowed parameter targets, including aliases and recursive calls.
//! A field write cannot replace its containing target. Rebinding a view changes
//! only the local pointer and therefore has no target-write effect.
use super::{borrow_check, loans::Access, *};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TargetEffects {
    pub replace: bool,
    pub write_fields: bool,
}
impl TargetEffects {
    fn include(&mut self, other: Self) {
        self.replace |= other.replace;
        self.write_fields |= other.write_fields;
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    /// Indexed by parameter position, never by local id.
    pub parameters: Vec<TargetEffects>,
}

pub fn infer(program: &Program) -> Vec<Summary> {
    let provenance: Vec<_> = program
        .functions
        .iter()
        .map(borrow_check::provenance)
        .collect();
    let mut summaries: Vec<_> = program
        .functions
        .iter()
        .map(|f| Summary {
            parameters: vec![TargetEffects::default(); f.parameters.len()],
        })
        .collect();
    // Monotone finite domain: recursion (including mutually recursive entries)
    // reaches a fixed point without assuming any recursive call is effect-free.
    loop {
        let mut changed = false;
        for (id, function) in program.functions.iter().enumerate() {
            let mut summary = summaries[id].clone();
            for (block_id, block) in function.blocks.iter().enumerate() {
                if !provenance[id].reachable(block_id) {
                    continue;
                }
                let Terminator::Evaluate {
                    operation,
                    destination,
                    ..
                } = &block.terminator
                else {
                    continue;
                };
                let mut record = |place: &Place, effect: TargetEffects| {
                    for origin in provenance[id].places(block_id, place) {
                        let Some(position) =
                            function.parameters.iter().position(|p| *p == origin.local)
                        else {
                            continue;
                        };
                        if function.locals[origin.local].view.is_none() {
                            continue;
                        }
                        let projected = !origin.projections.is_empty();
                        summary.parameters[position].include(TargetEffects {
                            replace: effect.replace && !projected,
                            write_fields: effect.write_fields || (effect.replace && projected),
                        });
                    }
                };
                // Empty view destinations initialize/rebind the view itself.
                if !destination.projections.is_empty() {
                    record(
                        destination,
                        TargetEffects {
                            replace: true,
                            write_fields: false,
                        },
                    );
                }
                match operation {
                    Operation::Call(callee, arguments)
                    | Operation::CallStorage {
                        function: callee,
                        arguments,
                        ..
                    }
                    | Operation::CallView {
                        function: callee,
                        arguments,
                        ..
                    } => {
                        for (position, argument) in arguments.iter().enumerate() {
                            if let Operand::Address(place) = argument {
                                record(place, summaries[*callee].parameters[position]);
                            }
                        }
                    }
                    Operation::CallIndirect(_, arguments, contract) => {
                        if !contract.targets.is_empty() {
                            for callee in &contract.targets {
                                for (position, argument) in arguments.iter().enumerate() {
                                    if let Operand::Address(place) = argument {
                                        record(place, summaries[*callee].parameters[position])
                                    }
                                }
                            }
                            continue;
                        }
                        for argument in arguments {
                            if let Operand::Address(place) = argument {
                                let mutable = function.locals[place.local]
                                    .view
                                    .map_or(function.locals[place.local].mutable, |a| {
                                        a == Access::Mutable
                                    });
                                if mutable {
                                    record(
                                        place,
                                        TargetEffects {
                                            replace: true,
                                            write_fields: true,
                                        },
                                    );
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            changed |= summary != summaries[id];
            summaries[id] = summary;
        }
        if !changed {
            return summaries;
        }
    }
}

/// A finalizing target permits field writes but cannot start another lifetime.
/// Callers must use the summary of the actual selected entry specialization.
pub fn permits_finalizing(summary: &Summary, parameter: usize) -> bool {
    summary
        .parameters
        .get(parameter)
        .is_some_and(|effect| !effect.replace)
}
