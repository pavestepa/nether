//! # nether_rt_thread
//!
//! Purpose: the runtime half of `thread.spawn(...)` (language-spec §19,
//! Stage 6) — real, parallel OS threads via `std::thread::spawn`,
//! architecturally independent of `runtime/task`'s cooperative,
//! single-`current_thread`-Tokio-runtime `task.spawn`/`await` model (see
//! that crate's own module docs on why its `SpawnedTask` frame can never
//! cross a real thread boundary). `thread.spawn`'s captured values, by
//! contrast, genuinely do cross one — sound only because
//! `nether_typecheck::send_sync` already rejects any capture that isn't
//! `Send` before this code ever runs, and because `runtime/arc`'s
//! strong/weak counts are atomic (Stage 6) so a captured ARC value's
//! refcount survives real concurrent retain/release across the two
//! threads involved.
//!
//! Responsibilities:
//! - `nether_rt_thread_spawn`: spawns a real OS thread running a Nether
//!   closure's own code pointer against its (already-constructed,
//!   ARC-managed) environment, and returns an opaque, ARC-managed handle
//!   (`Thread<()>`'s own runtime representation).
//! - `nether_rt_thread_join`: blocks the calling thread until the
//!   spawned one finishes.
//!
//! v1 scope decision (mirrors `nether_typecheck::check::expr`'s own
//! `check_thread_spawn`): a spawned closure must return `()` — there is
//! no result to box/hand back, so the closure's code pointer ABI is
//! simply `extern "C" fn(*mut u8)`, the same "one env pointer in, no
//! return value" shape every zero-parameter Nether closure with a unit
//! body already compiles to. Supporting an arbitrary return type would
//! need a per-call-site result-boxing wrapper function generated in
//! `nether_codegen` (mirroring `Task<T>`'s own frame machinery) — kept
//! out of this stage's scope.
//!
//! Dependencies: `nether_rt_arc`.
//!
//! Consumers: `nether_codegen`'s generated code (`__thread_spawn`/
//! `__thread_join` builtin calls, `compiler/codegen/src/function/runtime.rs`).

use std::sync::Mutex;
use std::thread::JoinHandle;

/// The payload a `Thread<()>` value's ARC allocation holds: a single
/// pointer to this heap-boxed state (not the state inline — `JoinHandle`
/// isn't `Copy`/plain-bytes-safe to store directly in an ARC payload the
/// way this crate's sibling runtimes store scalars).
struct ThreadState {
    handle: Mutex<Option<JoinHandle<()>>>,
}

extern "C" fn drop_thread_state(payload: *mut u8) {
    unsafe {
        let state = payload.cast::<*mut ThreadState>().read();
        drop(Box::from_raw(state));
    }
}

/// A closure environment pointer crossing into a real
/// `std::thread::spawn` closure. Sound only because Nether's own
/// typecheck already guarantees every value captured into it is `Send`
/// (`nether_typecheck::send_sync`) — Rust itself has no way to see
/// through the raw pointer to verify this, exactly like `runtime/task`'s
/// own `unsafe impl Send for SpawnedTask`.
struct SendEnv(*mut u8);
unsafe impl Send for SendEnv {}

/// Spawns `code(env)` on a real OS thread and returns an opaque,
/// ARC-managed thread handle. `env` is a Nether closure's own
/// already-constructed environment pointer; `code` is that same
/// environment's own field 0 (its code pointer), decomposed by the
/// caller so this crate never needs to know the environment's layout
/// (mirrors `nether_codegen`'s existing dynamic-closure-call technique,
/// `CallTarget::Dynamic`).
///
/// `env` arrives under this crate's *transient*-argument convention (a
/// native/`CallBuiltin` call's arguments are retained before the call
/// and released again right after by the caller — `nether_mir::arc`'s
/// own `releases_after_call` docs), so this function retains its own,
/// independent copy of `env`'s credit for the spawned thread to release
/// once it finishes running — the incoming credit does not itself
/// survive past this call.
///
/// # Safety
/// `env` must be a live ARC payload pointer (or null); `code` must be a
/// real `extern "C" fn(*mut u8)` matching the Nether closure's own
/// zero-parameter, unit-return ABI (typecheck's own v1 restriction,
/// language-spec §19).
#[no_mangle]
pub unsafe extern "C" fn nether_rt_thread_spawn(env: *mut u8, code: *mut u8) -> *mut u8 {
    unsafe { nether_rt_arc::nether_rt_arc_retain(env) };
    let code_fn: extern "C" fn(*mut u8) = unsafe { std::mem::transmute(code) };
    let send_env = SendEnv(env);
    let handle = std::thread::spawn(move || {
        let send_env = send_env;
        code_fn(send_env.0);
        unsafe { nether_rt_arc::nether_rt_arc_release(send_env.0) };
    });
    let state = Box::into_raw(Box::new(ThreadState {
        handle: Mutex::new(Some(handle)),
    }));
    let payload = nether_rt_arc::nether_rt_arc_alloc(
        std::mem::size_of::<*mut ThreadState>() as i64,
        Some(drop_thread_state),
    );
    unsafe { payload.cast::<*mut ThreadState>().write(state) };
    payload
}

/// Blocks the calling thread until the spawned one finishes. A second
/// `.join()` on the same handle — not preventable at the Nether type
/// level, `Thread<T>` is an ordinary ARC value, never a linear/affine
/// one — is a silent no-op rather than a panic or UB. A spawned closure
/// that itself panics propagates that panic to the joining thread,
/// matching `std::thread::JoinHandle::join`'s own behavior.
///
/// # Safety
/// `payload` must be a live pointer returned by [`nether_rt_thread_spawn`].
#[no_mangle]
pub unsafe extern "C" fn nether_rt_thread_join(payload: *mut u8) {
    unsafe {
        let state = *payload.cast::<*mut ThreadState>();
        let handle = (*state)
            .handle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(handle) = handle {
            handle.join().expect("a spawned Nether thread panicked");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI64, Ordering};

    extern "C" fn increment(env: *mut u8) {
        unsafe {
            (*env.cast::<*const AtomicI64>())
                .as_ref()
                .unwrap()
                .fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn spawn_then_join_runs_the_closure_exactly_once() {
        let counter = AtomicI64::new(0);
        // A real ARC allocation, per this module's own documented
        // preconditions — `env` holds one word, itself a pointer back to
        // `counter` on this thread's stack (outliving the spawned
        // thread, which this test joins before returning).
        let env = nether_rt_arc::nether_rt_arc_alloc(8, None);
        unsafe { *env.cast::<*const AtomicI64>() = std::ptr::from_ref(&counter) };
        let handle = unsafe { nether_rt_thread_spawn(env, increment as *mut u8) };
        unsafe { nether_rt_thread_join(handle) };
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        unsafe { nether_rt_arc::nether_rt_arc_release(env) };
    }

    #[test]
    fn joining_twice_is_a_no_op_not_a_panic() {
        let env = nether_rt_arc::nether_rt_arc_alloc(8, None);
        let handle = unsafe { nether_rt_thread_spawn(env, no_op as *mut u8) };
        unsafe {
            nether_rt_thread_join(handle);
            nether_rt_thread_join(handle);
        }
        unsafe { nether_rt_arc::nether_rt_arc_release(env) };
    }

    extern "C" fn no_op(_env: *mut u8) {}
}
