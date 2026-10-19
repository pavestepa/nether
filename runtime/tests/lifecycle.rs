use nether_runtime::*;
use std::ptr;
struct Event {
    log: *mut Vec<u32>,
    id: u32,
    fail: bool,
}
unsafe fn drop_event(panic: &mut Panic, value: *mut u8) -> bool {
    unsafe {
        let event = &mut *value.cast::<Event>();
        (*event.log).push(event.id);
        if event.fail {
            panic.active = true;
            return false;
        }
    }
    true
}
#[test]
fn root_replacement_selection_and_panic_keep_dynamic_order() {
    let mut log = Vec::new();
    let mut events: Vec<_> = (0..3)
        .map(|id| Event {
            log: &mut log,
            id,
            fail: false,
        })
        .collect();
    let mut roots: Vec<_> = events
        .iter_mut()
        .map(|e| Root::new(drop_event, (e as *mut Event).cast()))
        .collect();
    let roots: Vec<_> = roots.iter_mut().map(|r| r as *mut Root).collect();
    let mut scope = Scope::default();
    let scope = &mut scope as *mut Scope;
    let mut panic = Panic::default();
    unsafe {
        for &root in &roots {
            push(scope, root);
        }
        forget(roots[0]);
        push(scope, roots[0]);
        assert!(cleanup_selected(
            &mut panic,
            scope,
            &[roots[1], roots[0], roots[0]]
        ));
        assert_eq!(log, [0, 1]);
        assert!(cleanup_scope(&mut panic, scope));
        assert_eq!(log, [0, 1, 2]);
        log.clear();
        events[1].fail = true;
        for &root in &roots {
            push(scope, root);
        }
        assert!(!cleanup_scope(&mut panic, scope));
        assert_eq!(log, [2, 1, 0]);
        assert!(panic.active);
    }
}
#[test]
fn slot_repeated_replacement_and_partial_construction() {
    let mut log = Vec::new();
    let mut event = Event {
        log: &mut log,
        id: 7,
        fail: false,
    };
    let mut root = Root::new(drop_event, ptr::null_mut());
    let mut owner = Scope::default();
    let mut slot = Slot::new(
        (&mut event as *mut Event).cast(),
        drop_event,
        &mut root,
        &mut owner,
    );
    let slot = &mut slot as *mut Slot;
    let mut panic = Panic::default();
    unsafe {
        for _ in 0..10_000 {
            slot_initialize(slot);
            assert!(slot_drop(&mut panic, slot.cast()));
        }
        assert_eq!(log.len(), 10_000);
        assert!(cleanup_scope(&mut panic, &mut owner));
        assert!(slot_drop(&mut panic, slot.cast()));
        let mut empty = Slot::new(
            ptr::null_mut(),
            drop_event,
            ptr::null_mut(),
            ptr::null_mut(),
        );
        assert!(slot_drop(&mut panic, (&mut empty as *mut Slot).cast()));
        slot_initialize(slot);
        slot_forget(slot);
        assert!(cleanup_scope(&mut panic, &mut owner));
        assert_eq!(log.len(), 10_000);
    }
}
#[test]
fn allocator_preserves_alignment_and_releases_storage() {
    for alignment in [1, 2, 8, 64, 4096] {
        let pointer = allocate(31, alignment);
        assert!(!pointer.is_null());
        assert_eq!(pointer as usize % alignment, 0);
        unsafe {
            pointer.write_bytes(0xab, 31);
            deallocate(pointer, 31, alignment);
        }
        let empty = allocate(0, alignment);
        assert_eq!(empty as usize, alignment);
        unsafe {
            deallocate(empty, 0, alignment);
        }
    }
    assert!(allocate(1, 3).is_null());
    assert!(allocate(usize::MAX, 8).is_null());
    #[cfg(feature = "diagnostic")]
    assert_eq!(live_allocations(), 0);
}
unsafe fn invalid_callback(panic: &mut Panic, opaque: *mut u8) -> bool {
    let slot = opaque.cast::<Slot>();
    unsafe {
        match std::env::var("NETHER_BAD_LIFECYCLE").unwrap().as_str() {
            "initialize" => slot_initialize(slot),
            "forget" => slot_forget(slot),
            "drop" => {
                slot_drop(panic, opaque);
            }
            _ => {
                panic.active = true;
                return false;
            }
        }
    }
    true
}
#[test]
fn abort_child() {
    let Ok(mode) = std::env::var("NETHER_BAD_LIFECYCLE") else {
        return;
    };
    let mut slot = Slot::new(
        ptr::null_mut(),
        invalid_callback,
        ptr::null_mut(),
        ptr::null_mut(),
    );
    // Callback needs the slot itself; construct once more before publishing it.
    let pointer = &mut slot as *mut Slot;
    slot = Slot::new(
        pointer.cast(),
        invalid_callback,
        ptr::null_mut(),
        ptr::null_mut(),
    );
    let pointer = &mut slot as *mut Slot;
    let mut panic = Panic {
        active: mode == "double",
    };
    unsafe {
        slot_initialize(pointer);
        slot_drop(&mut panic, pointer.cast());
    }
    panic!("invalid lifecycle was accepted");
}
#[test]
fn invalid_lifecycles_abort_instead_of_restarting_destruction() {
    for mode in ["initialize", "forget", "drop", "double"] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "abort_child", "--nocapture"])
            .env("NETHER_BAD_LIFECYCLE", mode)
            .output()
            .unwrap();
        assert!(!output.status.success());
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(output.status.signal(), Some(6), "{mode}: {:?}", output);
        }
    }
}
