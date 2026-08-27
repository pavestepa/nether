//! The heap frame an `is_async` [`MirFunction`]'s codegen splits into —
//! see `crate::function::async_fn`'s module docs for how the two LLVM
//! functions (`start`/`poll`) this layout serves are actually built.
//!
//! Field order: `[poll_fn_ptr, output_drop_fn_ptr, output: T, state: i64,
//! <one field per local, in `Local::index()` order>]` — the first three
//! fields deliberately reuse `runtime/task`'s existing `TaskHeader`
//! convention byte-for-byte (`poll` fn pointer at word 0, nullable
//! `output_drop` fn pointer at word 1, output value right after at word
//! 2) so a state-machine frame is drop/poll-compatible with a "completed"
//! task, a timer task, or any other native task payload, with zero
//! `runtime` ABI changes — `nether_rt_task_poll`/`nether_rt_task_drop`
//! already only ever look at a task payload through that shared prefix.
//!
//! This crate's MIR is not SSA — every `Local` already lives in its own
//! persistent storage slot for a function's whole lifetime (an `alloca` in
//! the ordinary/eager codegen path — see `crate` module docs). Turning
//! that per-function storage into per-frame storage is therefore the
//! *entire* state-machine transform: every local, live-across-an-await or
//! not, gets a permanent frame field, addressed exactly like its `alloca`
//! would have been. This flat layout is deliberately simpler than a
//! Rust-style overlay/union of per-state live sets — it costs some unused
//! frame bytes in states that don't touch every local, but needs no
//! liveness analysis this compiler doesn't otherwise have, matching this
//! codebase's existing layout precedents (`layout::EnumLayout`'s own
//! "flat, non-overlapping" choice for the same reason).
use nether_llvm::{Func, ModuleCx, StructTy, Ty};
use nether_mir::{BlockId, Local, MirFunction, Terminator};
use nether_resolver::Definitions;
use nether_typecheck::{alloc_kind, AllocKind, Type};

use crate::function::is_aggregate;
use crate::layout::Layout;
use crate::runtime::Runtime;

#[derive(Clone)]
pub(crate) struct FrameLayout<'ctx> {
    pub ty: StructTy<'ctx>,
    pub poll_field: u32,
    pub output_field: u32,
    pub output_drop_field: u32,
    pub state_field: u32,
    /// Indexed by [`Local::index`].
    pub local_fields: Vec<u32>,
    /// Indexed by [`Local::index`] — `true` when the field holds a
    /// caller-owned alias pointer (a `mut`-scalar or aggregate parameter)
    /// that must be *loaded* to reconstruct the local's own address,
    /// rather than addressed in place — mirrors `FnCodegen::build`'s own
    /// alias-parameter special case (`declare`'s matching by-pointer
    /// convention) for the exact same two shapes.
    pub alias_locals: Vec<bool>,
    /// `state_blocks[i]`'s own poll-entry discriminant is `i as u64` —
    /// a position in this vector, not [`BlockId`]'s own internal
    /// numbering (opaque outside `nether_mir`). Always starts with
    /// `mir_fn.entry`; every other entry is a block whose own terminator
    /// is [`Terminator::Await`] — `nether_mir::split_await_points`'s own
    /// docs on why that block, not its `resume` block, is the correct
    /// per-poll re-entry point.
    state_blocks: Vec<BlockId>,
}

impl FrameLayout<'_> {
    pub fn state_of(&self, block: BlockId) -> u64 {
        self.state_blocks
            .iter()
            .position(|&b| b == block)
            .expect("state_blocks always covers the entry block and every await-check block")
            as u64
    }

    /// `state_blocks()[i]`'s own poll-entry discriminant is `i as u64` —
    /// see the field's own docs.
    pub fn state_blocks(&self) -> &[BlockId] {
        &self.state_blocks
    }

    /// The sentinel discriminant `Terminator::Return`'s codegen
    /// (`FnCodegen::gen_async_return`) stores once a poll function
    /// finishes for good — one past every real `state_blocks` index, so
    /// it can never collide with an actual await-check block. Real
    /// concurrent scheduling (`task.spawn`'s background polling racing an
    /// explicit `await` of the same task) means a task's `poll` can
    /// legitimately be called again after it already returned `true`
    /// once; without this sentinel, the dispatch would re-enter whatever
    /// await-check block the frame was last suspended at and re-run
    /// everything from there — including re-executing already-completed
    /// side effects (a real, observed bug: a spawned task's own
    /// `println`s doubled up under real concurrent polling). Dispatching
    /// this state straight to an immediate `ret true`, before any of the
    /// real per-state checks, makes a completed poll function idempotent.
    pub fn completed_state(&self) -> u64 {
        self.state_blocks.len() as u64
    }
}

/// Computes `mir_fn`'s frame layout — called once per `is_async`
/// function's codegen (`crate::function::async_fn::build_async_function`),
/// shared by both the "start" and "poll" builders it produces; no
/// cross-function caching is needed since nothing else ever asks for it.
pub(crate) fn compute_frame_layout<'ctx>(
    m: &ModuleCx<'ctx>,
    layout: &Layout<'_, 'ctx>,
    mir_fn: &MirFunction,
) -> FrameLayout<'ctx> {
    let ptr = m.ptr_type();
    let i64_ty = m.int_type(64);
    let output_ty = layout.llvm_type(&mir_fn.ret);

    let mut alias_locals = vec![false; mir_fn.locals.len()];
    for &local in &mir_fn.params {
        alias_locals[local.index()] = is_alias_param(mir_fn, local, layout.defs);
    }

    let mut field_tys: Vec<Ty<'ctx>> = vec![ptr, ptr, output_ty, i64_ty];
    let mut local_fields = Vec::with_capacity(mir_fn.locals.len());
    for (i, decl) in mir_fn.locals.iter().enumerate() {
        local_fields.push(field_tys.len() as u32);
        field_tys.push(if alias_locals[i] {
            ptr
        } else {
            layout.llvm_type(&decl.ty)
        });
    }

    let mut state_blocks = vec![mir_fn.entry];
    for block in &mir_fn.blocks {
        if matches!(block.terminator, Terminator::Await { .. }) {
            state_blocks.push(block.id);
        }
    }

    FrameLayout {
        ty: m.struct_type(&field_tys),
        poll_field: 0,
        output_drop_field: 1,
        output_field: 2,
        state_field: 3,
        local_fields,
        alias_locals,
        state_blocks,
    }
}

/// Same predicate `crate::declare`'s param-type computation and
/// `FnCodegen::build`'s matching entry-block handling already use — a
/// `mut`-scalar (non-reference) or aggregate parameter is passed, and
/// therefore must be captured into a suspended frame, as a single alias
/// pointer to caller-owned storage, never a value of its own.
fn is_alias_param(mir_fn: &MirFunction, local: Local, defs: &Definitions) -> bool {
    let decl = mir_fn.local_decl(local);
    (decl.mutable && !matches!(decl.ty, Type::Ref(_) | Type::MutRef(_)))
        || is_aggregate(&decl.ty, defs)
}

/// The `nether_rt_arc_alloc` drop callback for a state-machine frame —
/// replaces reusing `runtime.task_drop` directly, which only ever knew
/// about the output slot and silently leaked any other heap-kind local
/// still held when a frame was dropped while pending. Defers to
/// `nether_rt_task_drop` first (still correct and sufficient for the
/// output slot on its own), then releases every directly heap-kind,
/// non-alias-kind local's own field.
///
/// Relies on `crate::function::FnCodegen::null_frame_field_after_release`
/// keeping the invariant "a directly heap-kind local's field is non-null
/// iff it currently holds a live, owned reference" — given that, this
/// walk needs no per-state knowledge of which locals are actually live in
/// whichever state the frame was suspended in: `nether_rt_arc_release`/
/// `nether_rt_unique_free` are themselves null-safe (`runtime/arc`'s own
/// docs — "later lexical drops remain valid because ARC release accepts
/// null"), so unconditionally releasing every eligible local's current
/// field value is correct whether or not that particular local happens to
/// be live in the frame's current state.
///
/// Excludes an alias-kind local (`FrameLayout::alias_locals`) — its field
/// is a pointer to *caller-owned* storage the frame never owned a
/// reference to, never one of this frame's own credits to release.
///
/// Documented v1 gap: a stack-kind aggregate local (`Tuple`/enum/stack
/// `struct`) with nested heap-kind fields is not covered — only bare
/// heap-kind locals participate in the null-after-release invariant above
/// — so if such a local's inner heap field was already released in-scope
/// before a later pending suspend, there is no nested walk here to skip
/// it safely; extending the null invariant to nested fields would close
/// this gap but needs real per-field liveness this stage deliberately
/// doesn't build. Accepted for v1 per this stage's own scoping.
pub(crate) fn frame_drop_shim<'ctx>(
    m: &ModuleCx<'ctx>,
    layout: &Layout<'_, 'ctx>,
    runtime: &Runtime<'ctx>,
    mir_fn: &MirFunction,
    frame: &FrameLayout<'ctx>,
) -> Func<'ctx> {
    let name = format!("nether_{}_mono{}_frame_drop", mir_fn.name, mir_fn.id.index());
    let f = m.declare_function(&name, m.fn_type(&[m.ptr_type()], None));
    let saved_block = m.current_block();
    let entry = m.append_block(f, "entry");
    m.position_at_end(entry);
    let frame_ptr = m.param(f, 0);

    m.call(runtime.task_drop, &[frame_ptr], "");

    for (i, decl) in mir_fn.locals.iter().enumerate() {
        if frame.alias_locals[i] || alloc_kind(&decl.ty, layout.defs) != AllocKind::Heap {
            continue;
        }
        let field_addr = m.struct_gep(frame.ty, frame_ptr, frame.local_fields[i], "local_field");
        let ptr = m.load(m.ptr_type(), field_addr, "local_ptr");
        let release_fn = if matches!(decl.ty, Type::Unique(_)) {
            runtime.unique_free
        } else {
            runtime.release
        };
        m.call(release_fn, &[ptr], "");
    }

    m.ret(None);
    m.position_at_end(saved_block);
    f
}
