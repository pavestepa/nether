use std::collections::HashMap;

use nether_monomorphization::MonoFnId;

use crate::node::{CallTarget, Instr, Local, MirFunction, Operand, Rvalue};

/// Inserts `Retain`/`Release` instructions into already-built MIR
/// (`docs/architecture/crates.md` § `compiler/mir`, `arc-model.md`).
///
/// Implements the two ARC rules that are genuine local properties of an
/// instruction, recoverable by pattern-matching the already-flattened CFG
/// (unlike scope-exit releases and bare-local-alias bind-time retains —
/// see `build.rs`'s module docs and `FnBuilder::prepare_new_binding` for
/// why those are decided earlier, during `build_mir`, while more context
/// is still available):
/// - **§3.1, bind-time retain for container reads**: a bare-local, heap-
///   kind destination assigned from a `Field`/`VariantField`/`Index`
///   rvalue — reading a reference out of a container that keeps its own
///   copy — gets an extra `Retain` right after. `Use` is deliberately
///   *not* in this set: by the time this pass runs, `build_mir` has
///   already inserted an explicit `Retain` itself, directly, everywhere
///   one is actually needed for a `Use` (see `FnBuilder::
///   prepare_new_binding`) — treating every `Use` as aliasing here too
///   would double-count it. A *constructing* rvalue
///   (`Construct`/`ConstructVariant`/`Tuple`/`Array`/`Concat`/`ToString`/
///   `Closure`)
///   or a call's result never needs one at all — both already start (or
///   arrive) at refcount 1 for this binding, which is this pass's whole
///   implementation of RVO (see `arc-model.md` §3.4).
/// - **§3.3, call-argument passing**: every heap-kind operand passed into
///   a call is retained immediately before the call instruction (the
///   `CallArrayMethod` receiver counts as an argument here too).
///
///   What happens *after* the call differs by callee kind, and this is a
///   real correctness fix over an earlier version of this pass (caught by
///   actually running generated code against a real allocator, not just
///   inspecting IR shape): a `Call` to another Nether function hands that
///   pre-call retain to the callee's own parameter binding, which
///   `build_mir` *always* releases at its own scope exit (or, if the
///   callee returns that exact parameter directly, hands the same credit
///   straight through to its caller instead — `FnBuilder::
///   lower_escaping_value` no longer adds a *second* retain for that case,
///   for the same reason). Adding a release here too, on top of the
///   callee's own guaranteed release, double-frees the argument the
///   moment the callee returns — a real bug this crate's own tests didn't
///   catch, since none of them re-read a call's own argument afterward,
///   which is exactly the pattern that crashes.
///
///   `CallBuiltin`/`CallArrayMethod` are different: their "callee" is
///   native `runtime` code, not a Nether MIR function with a body of its
///   own to release anything at scope exit — so the retain/release here
///   is the *entire* transaction, a purely transactional bump for the
///   call's own duration that nets to zero and grants the native callee
///   nothing lasting. Where a native callee genuinely needs to keep a
///   reference (`nether_rt_array_push`, storing an element into the
///   array's own long-lived storage), it performs that retain itself —
///   see `nether_codegen::shims`'s module docs.
pub fn insert_arc(functions: &mut [MirFunction]) {
    let mutable_params: HashMap<MonoFnId, Vec<bool>> = functions
        .iter()
        .map(|function| {
            (
                function.id,
                function
                    .params
                    .iter()
                    .map(|local| function.locals[local.0 as usize].mutable)
                    .collect(),
            )
        })
        .collect();
    for f in functions {
        insert_arc_fn(f, &mutable_params);
    }
}

fn insert_arc_fn(f: &mut MirFunction, mutable_params: &HashMap<MonoFnId, Vec<bool>>) {
    for block in &mut f.blocks {
        let old = std::mem::take(&mut block.instrs);
        let mut new_instrs = Vec::with_capacity(old.len());
        for instr in old {
            match instr {
                Instr::Assign(place, rvalue) => {
                    let call_args = call_operands(&rvalue, mutable_params);
                    let releases_after_call = releases_after_call(&rvalue);
                    let heap_args: Vec<Local> = call_args
                        .iter()
                        .flatten()
                        .filter_map(|op| match op {
                            Operand::Local(l)
                                if f.locals[l.0 as usize].needs_drop
                                    && !matches!(
                                        f.locals[l.0 as usize].ty,
                                        nether_typecheck::Type::Unique(_)
                                    ) =>
                            {
                                Some(*l)
                            }
                            _ => None,
                        })
                        .collect();
                    for &l in &heap_args {
                        new_instrs.push(Instr::Retain(l));
                    }
                    let dest_local = place.local;
                    let retain_dest = place.projection.is_empty()
                        && f.locals[dest_local.0 as usize].needs_drop
                        && !matches!(
                            f.locals[dest_local.0 as usize].ty,
                            nether_typecheck::Type::Unique(_)
                        )
                        && is_aliasing(&rvalue);
                    new_instrs.push(Instr::Assign(place, rvalue));
                    if releases_after_call {
                        for &l in &heap_args {
                            new_instrs.push(Instr::Release(l));
                        }
                    }
                    if retain_dest {
                        new_instrs.push(Instr::Retain(dest_local));
                    }
                }
                other => new_instrs.push(other),
            }
        }
        block.instrs = new_instrs;
    }
}

fn call_operands(
    rvalue: &Rvalue,
    mutable_params: &HashMap<MonoFnId, Vec<bool>>,
) -> Option<Vec<Operand>> {
    match rvalue {
        Rvalue::Call {
            target: CallTarget::Fn(id),
            args,
        } => {
            let mutable = mutable_params.get(id);
            Some(
                args.iter()
                    .enumerate()
                    .filter(|(index, _)| {
                        !mutable
                            .and_then(|params| params.get(*index))
                            .copied()
                            .unwrap_or(false)
                    })
                    .map(|(_, operand)| operand.clone())
                    .collect(),
            )
        }
        Rvalue::Call { args, .. } | Rvalue::CallBuiltin { args, .. } => Some(args.clone()),
        Rvalue::CallArrayMethod { receiver, args, .. } => {
            let mut all = Vec::with_capacity(args.len() + 1);
            all.push(receiver.clone());
            all.extend(args.iter().cloned());
            Some(all)
        }
        Rvalue::CallWitness { args, .. } => Some(args.clone()),
        _ => None,
    }
}

/// Whether the caller also releases a heap-kind argument right after the
/// call, on top of retaining it before — see [`insert_arc`]'s own docs
/// for why this must be `false` for `Call` (a Nether callee's own scope
/// exit already releases its parameter) and `true` for `CallBuiltin`/
/// `CallArrayMethod` (a native callee with no such mechanism of its own).
fn releases_after_call(rvalue: &Rvalue) -> bool {
    matches!(
        rvalue,
        Rvalue::CallBuiltin { .. } | Rvalue::CallArrayMethod { .. }
    )
}

fn is_aliasing(rvalue: &Rvalue) -> bool {
    matches!(
        rvalue,
        Rvalue::Field { .. }
            | Rvalue::VariantField { .. }
            | Rvalue::Index { .. }
            | Rvalue::Await(_)
    )
}
