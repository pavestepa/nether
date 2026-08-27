use crate::node::{BasicBlock, BlockId, Instr, Local, MirFunction, Operand, Rvalue, Terminator};

/// Splits every `Rvalue::Await` in an `is_async` function's MIR into a real
/// suspension point in the control-flow graph — [`Terminator::Await`]'s own
/// docs explain why `codegen` needs the CFG shaped this way rather than a
/// plain instruction it can lower in place. Must run after
/// [`crate::insert_arc`]: the aliasing-retain `insert_arc` already adds
/// right after an `Rvalue::Await` assignment (`crate::arc`'s own
/// `is_aliasing` rule), and the awaited operand's temporary release
/// (`nether_mir::build`'s own `lower_expr` for `MonoExprKind::Await`), both
/// need to land on the *resume* side of the split, which only works if
/// they're still sitting right after the assignment when this pass moves
/// it — this pass relocates existing instructions rather than re-deriving
/// their placement.
pub fn split_await_points(functions: &mut [MirFunction]) {
    for f in functions {
        if f.is_async {
            split_await_points_fn(f);
        }
    }
}

fn split_await_points_fn(f: &mut MirFunction) {
    let mut block_index = 0;
    while block_index < f.blocks.len() {
        if let Some((pos, task, output_local)) = find_await(&f.blocks[block_index].instrs) {
            split_block_at(f, block_index, pos, task, output_local);
        }
        block_index += 1;
    }
}

/// The first `Instr::Assign` of a bare-local place from `Rvalue::Await` in
/// `instrs`, if any — `nether_mir::build`'s `materialize` always produces a
/// fresh, unprojected `Place` for an `Await` rvalue (there is no surface
/// syntax to await into a field/index place directly), so a bare-local
/// match is exhaustive here.
fn find_await(instrs: &[Instr]) -> Option<(usize, Operand, Local)> {
    instrs.iter().enumerate().find_map(|(i, instr)| match instr {
        Instr::Assign(place, Rvalue::Await(task)) if place.projection.is_empty() => {
            Some((i, task.clone(), place.local))
        }
        _ => None,
    })
}

/// Splits `f.blocks[block_index]` at instruction `pos` (the `Rvalue::Await`
/// assignment found by [`find_await`]) into *three* blocks, not two:
///
/// - `block_index` keeps everything before `pos` (the instructions that
///   compute `task`) and gets a plain `Terminator::Goto` into a fresh
///   `check` block.
/// - `check` is empty and ends in `Terminator::Await { task, ... }` — this,
///   not `block_index`, is `codegen`'s per-poll re-entry point on a
///   pending suspend. It has to be its own instruction-free block: if
///   `codegen` instead re-entered `block_index` directly on every re-poll,
///   the instructions that compute `task` (a call, say) would re-run on
///   every single poll, recomputing a *new* task instead of re-checking
///   the one already parked in the frame — silently discarding the
///   original, still-pending task each time. Splitting the "compute" and
///   "check" halves into separate blocks means the compute half only ever
///   runs once, on the real first pass through `block_index`.
/// - `resume` holds everything after `pos` — including whatever
///   `insert_arc`/`build_mir` already placed right after the await, plus
///   the block's original terminator — and only ever runs once the await
///   is genuinely ready.
fn split_block_at(
    f: &mut MirFunction,
    block_index: usize,
    pos: usize,
    task: Operand,
    output_local: Local,
) {
    let block = &mut f.blocks[block_index];
    let tail = block.instrs.split_off(pos + 1);
    block.instrs.truncate(pos);
    let resume_terminator = std::mem::replace(&mut block.terminator, Terminator::Unreachable);

    let resume_id = BlockId(f.blocks.len() as u32);
    f.blocks.push(BasicBlock {
        id: resume_id,
        instrs: tail,
        terminator: resume_terminator,
    });

    let check_id = BlockId(f.blocks.len() as u32);
    f.blocks.push(BasicBlock {
        id: check_id,
        instrs: Vec::new(),
        terminator: Terminator::Await {
            task,
            output_local,
            resume: resume_id,
        },
    });

    f.blocks[block_index].terminator = Terminator::Goto(check_id);
}
