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

/// Creates an independently owned task handle for scheduler submission.
/// The current-thread executor polls that handle when it is awaited.
///
/// # Safety
/// `task` must be null or a live ARC-managed Nether task payload.
#[no_mangle]
pub unsafe extern "C" fn nether_rt_task_spawn(task: *mut u8) -> *mut u8 {
    unsafe { nether_rt_arc::nether_rt_arc_retain(task) };
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
