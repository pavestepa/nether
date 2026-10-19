//! Trusted Rust primitives for compiler-owned slots. These do not infer ownership.
//! All linked records must stay at stable addresses until unlinked.
use std::{
    alloc::{alloc, dealloc, Layout},
    ptr,
};

#[derive(Default)]
pub struct Panic {
    pub active: bool,
}
pub type DropFn = unsafe fn(&mut Panic, *mut u8) -> bool;
pub struct Scope {
    last: *mut Root,
}
impl Default for Scope {
    fn default() -> Self {
        Self {
            last: ptr::null_mut(),
        }
    }
}
pub struct Root {
    scope: *mut Scope,
    previous: *mut Root,
    next: *mut Root,
    drop: DropFn,
    value: *mut u8,
}
impl Root {
    pub fn new(drop: DropFn, value: *mut u8) -> Self {
        Self {
            scope: ptr::null_mut(),
            previous: ptr::null_mut(),
            next: ptr::null_mut(),
            drop,
            value,
        }
    }
}

/// Allocate target storage, returning null for invalid layout or OOM.
pub fn allocate(size: usize, alignment: usize) -> *mut u8 {
    let Ok(layout) = Layout::from_size_align(size, alignment) else {
        return ptr::null_mut();
    };
    if size == 0 {
        return alignment as *mut u8;
    }
    // SAFETY: layout was validated, and size is nonzero.
    let pointer = unsafe { alloc(layout) };
    #[cfg(feature = "diagnostic")]
    if !pointer.is_null() {
        registry::insert(pointer, size, alignment);
    }
    pointer
}
/// # Safety
/// Pointer must be a live allocation from allocate with the identical layout,
/// and no live access may remain after this call. ZST sentinels are not freed.
pub unsafe fn deallocate(pointer: *mut u8, size: usize, alignment: usize) {
    if size == 0 {
        return;
    }
    let layout = Layout::from_size_align(size, alignment).expect("invalid allocation layout");
    #[cfg(feature = "diagnostic")]
    registry::remove(pointer, size, alignment);
    unsafe {
        dealloc(pointer, layout);
    }
}

/// # Safety
/// Root and its linked neighbors/scope must be live, stable, uniquely accessed
/// runtime records. The target value is not read or destroyed.
pub unsafe fn forget(root: *mut Root) {
    unsafe {
        let scope = (*root).scope;
        if scope.is_null() {
            return;
        }
        if !(*root).previous.is_null() {
            (*(*root).previous).next = (*root).next;
        }
        if !(*root).next.is_null() {
            (*(*root).next).previous = (*root).previous;
        } else {
            (*scope).last = (*root).previous;
        }
        (*root).scope = ptr::null_mut();
        (*root).previous = ptr::null_mut();
        (*root).next = ptr::null_mut();
    }
}
/// # Safety
/// Root is inactive, scope and root stay at stable live addresses while linked.
/// The callback and value must satisfy their destruction contract.
pub unsafe fn push(scope: *mut Scope, root: *mut Root) {
    unsafe {
        assert!((*root).scope.is_null(), "root already linked");
        (*root).scope = scope;
        (*root).previous = (*scope).last;
        (*root).next = ptr::null_mut();
        if !(*scope).last.is_null() {
            (*(*scope).last).next = root;
        }
        (*scope).last = root;
    }
}
fn result(panic: &Panic, unwinding: bool, success: bool) -> bool {
    if !success && (unwinding || !panic.active) {
        std::process::abort();
    }
    success
}
/// # Safety
/// Root meets forget's contract and its callback may destroy its live value.
/// Callback must set active on failure and must not unwind with Rust panic.
pub unsafe fn cleanup_root(panic: &mut Panic, root: *mut Root) -> bool {
    unsafe {
        if (*root).scope.is_null() {
            return true;
        }
        forget(root);
        let unwinding = panic.active;
        let success = ((*root).drop)(panic, (*root).value);
        result(panic, unwinding, success)
    }
}
/// # Safety
/// All linked roots meet cleanup_root's contract and remain live until unlinked.
pub unsafe fn cleanup_scope(panic: &mut Panic, scope: *mut Scope) -> bool {
    let mut success = true;
    unsafe {
        while !(*scope).last.is_null() {
            success &= cleanup_root(panic, (*scope).last);
        }
    }
    success
}
/// # Safety
/// Same as cleanup_scope; selected pointers identify records in this scope or
/// inactive records. Callbacks must not invalidate other root records.
pub unsafe fn cleanup_selected(
    panic: &mut Panic,
    scope: *mut Scope,
    selected: &[*mut Root],
) -> bool {
    let mut success = true;
    unsafe {
        let mut root = (*scope).last;
        while !root.is_null() {
            let previous = (*root).previous;
            if selected.contains(&root) {
                success &= cleanup_root(panic, root);
            }
            root = previous;
        }
    }
    success
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Empty,
    Live,
    Finalizing,
}
pub struct Slot {
    value: *mut u8,
    drop: DropFn,
    root: *mut Root,
    owner: *mut Scope,
    state: State,
}
impl Slot {
    pub fn new(value: *mut u8, drop: DropFn, root: *mut Root, owner: *mut Scope) -> Self {
        Self {
            value,
            drop,
            root,
            owner,
            state: State::Empty,
        }
    }
}
/// # Safety
/// Slot is stable/live; its root meets forget's contract. All aliases must refer
/// to this same capability, and ownership is transferred exactly once.
pub unsafe fn slot_forget(slot: *mut Slot) {
    unsafe {
        if (*slot).state == State::Finalizing {
            std::process::abort();
        }
        (*slot).state = State::Empty;
        if !(*slot).root.is_null() {
            forget((*slot).root);
        }
    }
}
/// # Safety
/// Slot is empty with fully initialized payload; root/owner are valid stable
/// records, or both null for an inline field. Slot remains stable while live.
pub unsafe fn slot_initialize(slot: *mut Slot) {
    unsafe {
        if (*slot).state != State::Empty {
            std::process::abort();
        }
        (*slot).state = State::Live;
        if !(*slot).root.is_null() {
            assert!(!(*slot).owner.is_null());
            (*(*slot).root).drop = slot_drop;
            (*(*slot).root).value = slot.cast();
            push((*slot).owner, (*slot).root);
        }
    }
}
/// # Safety
/// Opaque points to a live stable Slot. Its callback obeys cleanup_root's
/// contract; it may mutate/drop fields but cannot replace this finalizing slot.
pub unsafe fn slot_drop(panic: &mut Panic, opaque: *mut u8) -> bool {
    unsafe {
        let slot = opaque.cast::<Slot>();
        if (*slot).state == State::Empty {
            return true;
        }
        slot_forget(slot);
        (*slot).state = State::Finalizing;
        let unwinding = panic.active;
        let success = ((*slot).drop)(panic, (*slot).value);
        (*slot).state = State::Empty;
        result(panic, unwinding, success)
    }
}
#[cfg(feature = "diagnostic")]
mod registry {
    use std::{collections::BTreeMap, sync::Mutex};
    static LIVE: Mutex<BTreeMap<usize, (usize, usize)>> = Mutex::new(BTreeMap::new());
    pub fn insert(pointer: *mut u8, size: usize, align: usize) {
        assert!(LIVE
            .lock()
            .unwrap()
            .insert(pointer as usize, (size, align))
            .is_none());
    }
    pub fn remove(pointer: *mut u8, size: usize, align: usize) {
        if LIVE.lock().unwrap().remove(&(pointer as usize)) != Some((size, align)) {
            std::process::abort();
        }
    }
    pub fn count() -> usize {
        LIVE.lock().unwrap().len()
    }
}
#[cfg(feature = "diagnostic")]
pub fn live_allocations() -> usize {
    registry::count()
}
