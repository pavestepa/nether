//! Codegen for an `is_async` [`MirFunction`] — replaces the old
//! eager-then-wrap shortcut (`gen_completed_task`/`gen_await`, both
//! removed) with the real state-machine transform `frame`'s module docs
//! describe.
//!
//! Every `is_async` function becomes *two* LLVM functions instead of one:
//!
//! - The **start** function keeps the mangled name every existing call
//!   site already uses (`crate::declare`'s `is_async` branch only changes
//!   its *return type*, to `ptr`) — it never runs any of the function's
//!   own MIR; it just heap-allocates the frame, stores the incoming
//!   parameters into it, and returns the frame pointer as a `Task<T>`.
//!   Built by [`build_async_start`], a small hand-rolled prologue — there
//!   is no MIR body to translate here, so it doesn't go through
//!   [`FnCodegen`] at all.
//! - The **poll** function (`crate::declare_poll`) carries the *entire*
//!   translated MIR body, built through the ordinary [`FnCodegen`]
//!   machinery with `frame: Some(_)` — see `function.rs`'s own docs on
//!   how that changes local addressing and the entry dispatch.
use super::*;
use crate::frame::{compute_frame_layout, frame_drop_shim, FrameLayout};

pub(super) fn build_async_function<'m, 'ctx>(
    env: &'m FunctionEnv<'m, 'ctx>,
    mir_fn: &'m MirFunction,
    start_fn: Func<'ctx>,
) {
    let frame = compute_frame_layout(env.m, env.layout, mir_fn);
    let poll_fn = env.poll_funcs[&mir_fn.id];

    build_async_start(env, mir_fn, &frame, start_fn, poll_fn);

    let mut fc = FnCodegen {
        m: env.m,
        layout: env.layout,
        runtime: env.runtime,
        shims: env.shims,
        funcs: env.funcs,
        mir_functions: env.mir_functions,
        mir_fn,
        llvm_fn: poll_fn,
        locals: Vec::new(),
        blocks: HashMap::new(),
        frame: Some(frame),
        frame_ptr: None,
        unit_slot: None,
        entry_block: None,
    };
    fc.build();
}

/// Builds the small, MIR-free prologue described in this module's own
/// docs. `mir_fn.is_closure` is assumed `false` here — there is no
/// surface syntax for an async closure today (only `async fn`/`async
/// method(...)`), so an `is_async` `MirFunction` never carries closure
/// captures to initialize.
fn build_async_start<'ctx>(
    env: &FunctionEnv<'_, 'ctx>,
    mir_fn: &MirFunction,
    frame: &FrameLayout<'ctx>,
    start_fn: Func<'ctx>,
    poll_fn: Func<'ctx>,
) {
    let m = env.m;
    let entry = m.append_block(start_fn, "entry");
    m.position_at_end(entry);

    // Generating the drop shim mid-build is safe — `frame_drop_shim`
    // saves/restores the builder's current position itself, the same
    // convention `crate::shims` already uses for the same reason.
    let drop_fn = frame_drop_shim(m, env.layout, env.runtime, mir_fn, frame);
    let size = m.size_of(frame.ty.into());
    let drop_ptr: Value<'ctx> = drop_fn.as_global_value().as_pointer_value().into();
    let frame_ptr = m
        .call(env.runtime.alloc, &[size, drop_ptr], "frame")
        .expect("nether_rt_arc_alloc returns a payload pointer");

    let poll_field = m.struct_gep(frame.ty, frame_ptr, frame.poll_field, "poll_field");
    m.store(
        poll_field,
        poll_fn.as_global_value().as_pointer_value().into(),
    );

    let output_drop_field =
        m.struct_gep(frame.ty, frame_ptr, frame.output_drop_field, "output_drop_field");
    m.store(output_drop_field, m.const_null_ptr());

    let state_field = m.struct_gep(frame.ty, frame_ptr, frame.state_field, "state_field");
    let entry_state = frame.state_of(mir_fn.entry);
    m.store(
        state_field,
        m.const_int(m.int_type(64), entry_state, false),
    );

    // Every directly heap-kind, non-alias-kind local's field starts null
    // — `frame_drop_shim` walks and unconditionally releases exactly this
    // set if the frame is dropped while pending, and release is
    // null-safe but a garbage, never-initialized pointer is not: a local
    // that hasn't been assigned yet by the time an early suspend drops
    // the frame must read back as "nothing to release," not whatever
    // bytes `nether_rt_arc_alloc` happened to return. See
    // `frame::frame_drop_shim`'s own docs for the invariant this
    // maintains together with `function::FnCodegen::
    // null_frame_field_after_release`.
    for (i, decl) in mir_fn.locals.iter().enumerate() {
        if frame.alias_locals[i] || alloc_kind(&decl.ty, env.layout.defs) != AllocKind::Heap {
            continue;
        }
        let field_ptr = m.struct_gep(frame.ty, frame_ptr, frame.local_fields[i], "local_field");
        m.store(field_ptr, m.const_null_ptr());
    }

    let param_offset = usize::from(mir_fn.is_closure);
    for (i, &local) in mir_fn.params.iter().enumerate() {
        let param_val = m.param(start_fn, (i + param_offset) as u32);
        let field_index = frame.local_fields[local.index()];
        let field_ptr = m.struct_gep(frame.ty, frame_ptr, field_index, "param_field");
        m.store(field_ptr, param_val);
    }

    m.ret(Some(frame_ptr));
}
