//! Tokio-backed scheduler boundary for compiler-generated Nether tasks.
//!
//! Every task payload begins with a poll callback. The remainder is owned by
//! the compiler-generated state-machine layout (or a native runtime task such
//! as `timer.sleep`). This keeps Tokio and Rust futures out of Nether's ABI.

use std::time::{SystemTime, UNIX_EPOCH};

type PollFn = unsafe extern "C" fn(*mut u8) -> bool;

#[repr(C)]
struct TaskHeader {
    poll: PollFn,
    output_drop: Option<extern "C" fn(*mut u8)>,
}

/// Polls a task once, returning whether its output is ready.
///
/// # Safety
/// `task` must point to a live Nether task payload whose first word is a
/// valid [`PollFn`].
#[no_mangle]
pub unsafe extern "C" fn nether_rt_task_poll(task: *mut u8) -> bool {
    if task.is_null() {
        return true;
    }
    let poll = unsafe { (*task.cast::<TaskHeader>()).poll };
    unsafe { poll(task) }
}

/// Drives a task to completion on a Tokio current-thread runtime.
///
/// # Safety
/// `task` has the same requirements as [`nether_rt_task_poll`] and must stay
/// alive for the duration of this call.
#[no_mangle]
pub unsafe extern "C" fn nether_rt_task_block_on(task: *mut u8) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("create Nether Tokio runtime");
    runtime.block_on(async move {
        while !unsafe { nether_rt_task_poll(task) } {
            tokio::task::yield_now().await;
        }
    });
}

/// Poll callback used by a task whose output was completed synchronously.
///
/// # Safety
/// The pointer is ignored and may be any task payload pointer.
#[no_mangle]
pub unsafe extern "C" fn nether_rt_task_completed_poll(_task: *mut u8) -> bool {
    true
}

/// ARC drop callback for the common task header followed by output storage.
///
/// # Safety
/// `task` must be a compiler/runtime task payload with a valid common header.
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn nether_rt_task_drop(task: *mut u8) {
    let header = unsafe { &*task.cast::<TaskHeader>() };
    if let Some(drop) = header.output_drop {
        let output = unsafe { task.add(std::mem::size_of::<TaskHeader>()) };
        drop(output);
    }
}

#[repr(C)]
struct TimerTask {
    poll: PollFn,
    output_drop: Option<extern "C" fn(*mut u8)>,
    deadline_millis: u64,
}

unsafe extern "C" fn timer_poll(task: *mut u8) -> bool {
    let timer = unsafe { &*task.cast::<TimerTask>() };
    unix_millis() >= timer.deadline_millis
}

/// Creates a native `Task<()>` which becomes ready after `millis`.
#[no_mangle]
pub extern "C" fn nether_rt_timer_sleep(millis: u64) -> *mut u8 {
    let size = std::mem::size_of::<TimerTask>() as i64;
    let payload = nether_rt_arc::nether_rt_arc_alloc(size, Some(nether_rt_task_drop));
    unsafe {
        payload.cast::<TimerTask>().write(TimerTask {
            poll: timer_poll,
            output_drop: None,
            deadline_millis: unix_millis().saturating_add(millis),
        });
    }
    payload
}

/// Wraps a task payload pointer as a `Future` so [`nether_rt_task_spawn`]
/// can hand it to `tokio::spawn` — the *only* place this crate lets Tokio
/// itself touch a Nether task, kept out of the public ABI entirely (see
/// this module's own docs).
struct SpawnedTask {
    frame: *mut u8,
}

// Two independent reasons this is now sound, either one alone would
// suffice: (1) every Nether task, spawned or not, still ever runs on the
// single `current_thread` runtime `nether_rt_task_block_on` builds —
// Tokio's current-thread scheduler never migrates a spawned task to a
// different OS thread, so this `Send` impl never actually crosses a real
// thread boundary; (2) as of Stage 6, `runtime/arc`'s strong/weak counts
// are atomic, so even a genuine cross-thread move of the frame pointer
// would no longer race the refcount. `task`/`thread` remain
// intentionally separate execution models — a `Task<T>` still cannot
// cross a real `thread.spawn` boundary (it is never `Send`, see
// `nether_typecheck::send_sync`), so reason (1) continues to hold in
// practice; reason (2) is a documented, independent safety net, not a
// premise this impl currently relies on crossing.
unsafe impl Send for SpawnedTask {}

impl std::future::Future for SpawnedTask {
    // Yields the frame pointer back out, rather than `()` — so the
    // `async move` block in `nether_rt_task_spawn` never needs to capture
    // a second, bare `*mut u8` of its own alongside `self` across the
    // `.await` (a raw pointer isn't `Send` on its own, only `SpawnedTask`
    // is, via the `unsafe impl` above).
    type Output = *mut u8;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<*mut u8> {
        if unsafe { nether_rt_task_poll(self.frame) } {
            std::task::Poll::Ready(self.frame)
        } else {
            // No real waker/reactor integration yet — a spawned task is
            // driven by cooperative re-polling, the same busy-poll
            // convention `nether_rt_task_block_on`'s own `yield_now` loop
            // already uses, rather than only waking on the specific
            // native event (a timer firing, I/O completing) it's
            // actually blocked on. Keeps it in Tokio's ready queue so the
            // scheduler revisits it promptly; a real waker integration is
            // future work, once real OS-thread parallelism makes the
            // busy-poll cost worth removing.
            cx.waker().wake_by_ref();
            std::task::Poll::Pending
        }
    }
}

/// Schedules `task` for independent background progress and returns an
/// independently owned handle to it — unlike the milestone-3-era version
/// of this function (a bare retain-and-return that relied on the caller
/// having already run the task's body eagerly), `task` may still be
/// entirely unpolled at this point, so this is the first thing that
/// actually drives it: wrapping it as a [`SpawnedTask`] and handing it to
/// `tokio::spawn`, which attaches to the ambient runtime automatically —
/// every call into this function happens from Nether-generated code
/// already running inside `nether_rt_task_block_on`'s `block_on`, so
/// `Handle::current()` always resolves. Retains twice: once for the
/// handle this function returns (the caller's own new binding, matching
/// every other "arrives already owned" call result in this ABI), once
/// for the spawned future's own independent ownership — the caller may
/// drop its own handle immediately without stopping the background task,
/// which the spawned future's own trailing release accounts for once it
/// finishes.
///
/// # Safety
/// `task` must be null or a live ARC-managed Nether task payload.
#[no_mangle]
pub unsafe extern "C" fn nether_rt_task_spawn(task: *mut u8) -> *mut u8 {
    unsafe { nether_rt_arc::nether_rt_arc_retain(task) };
    unsafe { nether_rt_arc::nether_rt_arc_retain(task) };
    let spawned = SpawnedTask { frame: task };
    tokio::spawn(async move {
        let frame = spawned.await;
        unsafe { nether_rt_arc::nether_rt_arc_release(frame) };
    });
    task
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_and_timer_tasks_are_driven_through_the_common_poll_abi() {
        let completed = nether_rt_arc::nether_rt_arc_alloc(
            std::mem::size_of::<TaskHeader>() as i64,
            Some(nether_rt_task_drop),
        );
        unsafe {
            completed.cast::<TaskHeader>().write(TaskHeader {
                poll: nether_rt_task_completed_poll,
                output_drop: None,
            });
            assert!(nether_rt_task_poll(completed));
            nether_rt_arc::nether_rt_arc_release(completed);
        }

        let timer = nether_rt_timer_sleep(1);
        unsafe {
            nether_rt_task_block_on(timer);
            assert!(nether_rt_task_poll(timer));
            nether_rt_arc::nether_rt_arc_release(timer);
        }
    }
}
