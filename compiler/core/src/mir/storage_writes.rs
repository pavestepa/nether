//! Static may-write dependencies of mutable borrowed aggregate parameters.
//! Include intermediate states, not just returns: writes survive panic and must
//! never publish a descriptor to an owner destroyed inside the callee.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Write {
    pub target: returns::Origin,
    pub origins: BTreeSet<returns::Origin>,
}
pub type Contract = Vec<Write>;

pub fn call(operation: &Operation) -> Option<(usize, &[Operand])> {
    match operation {
        Operation::Call(id, args)
        | Operation::CallView {
            function: id,
            arguments: args,
            ..
        }
        | Operation::CallStorage {
            function: id,
            arguments: args,
            ..
        } => Some((*id, args)),
        _ => None,
    }
}

pub(super) fn effects<'a>(
    f: &'a Function,
    op: &'a Operation,
) -> Option<(&'a [Operand], &'a [Write])> {
    if let Operation::CallIndirect(_, args, contract) = op {
        return Some((args, &contract.writes));
    }
    let (callee, args) = call(op)?;
    Some((
        args,
        f.storage_writes.get(callee).map_or(&[], Vec::as_slice),
    ))
}
pub fn infer(program: &Program) -> Result<Vec<Contract>, returns::Error> {
    program
        .functions
        .iter()
        .enumerate()
        .map(|(id, f)| infer_function(id, f))
        .collect()
}

fn infer_function(id: usize, f: &Function) -> Result<Contract, returns::Error> {
    let parameters = f
        .parameters
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            f.locals[**p].view == Some(loans::Access::Mutable) && f.locals[**p].ty.contains_views()
        })
        .collect::<Vec<_>>();
    if parameters.is_empty() {
        return Ok(Vec::new());
    }
    let provenance = borrow_check::provenance(f);
    let mut writes = BTreeMap::<returns::Origin, BTreeSet<returns::Origin>>::new();
    for block in 0..f.blocks.len() {
        for (parameter, local) in &parameters {
            for (path, origins) in provenance.components(block, &Place::local(**local)) {
                // Components behind a stored readonly view are not writable.
                if path.is_empty() || path.contains(&Projection::Dereference) {
                    continue;
                }
                for origin in origins {
                    let source = f.parameters.iter().position(|p| *p == origin.local);
                    let Some(source) = source.filter(|_| {
                        f.locals[origin.local].view.is_some()
                            || origin.projections.contains(&Projection::Dereference)
                    }) else {
                        return Err(returns::Error { function:id, origin:Some(f.locals[origin.local].span),
                                message:"mutable parameter storage contains a view of an owner destroyed by this function" });
                    };
                    if path.len() > 32 || origin.projections.len() > 32 {
                        return Err(returns::Error {
                            function: id,
                            origin: None,
                            message: "storage write provenance exceeds the supported path limit",
                        });
                    }
                    // Expand dynamic indices before exporting a contract; no
                    // local numbering from this function may escape to callers.
                    for path in returns::concrete_paths(&f.locals[**local].ty, &path, &f.classes) {
                        for source_path in returns::concrete_paths(
                            &f.locals[origin.local].ty,
                            &origin.projections,
                            &f.classes,
                        ) {
                            let mut unchanged = path.clone();
                            unchanged.push(Projection::Dereference);
                            if source == *parameter && source_path == unchanged {
                                continue;
                            }
                            writes
                                .entry(returns::Origin {
                                    parameter: *parameter,
                                    path: path.clone(),
                                })
                                .or_default()
                                .insert(returns::Origin {
                                    parameter: source,
                                    path: source_path,
                                });
                        }
                    }
                }
            }
        }
    }
    Ok(writes
        .into_iter()
        .map(|(target, origins)| Write { target, origins })
        .collect())
}
