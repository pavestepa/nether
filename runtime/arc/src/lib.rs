//! # nether_rt_arc
//!
//! Purpose: the ARC allocator for ordinary heap values plus the
//! unrefcounted allocator/promotion bridge for unique heap values
//! (`docs/architecture/crates.md` § `runtime/arc`,
//! `docs/architecture/arc-model.md`).
//!
//! Responsibilities:
//! - Allocate `:T` behind a count-free [`UniqueHeader`], destroy its sole
//!   ownership with `nether_rt_unique_free`, and promote its payload into
//!   ARC with `nether_rt_unique_promote` for `:T -> T`.
//! - Allocate a heap object as a hidden [`Header`] (a strong count, a
//!   weak count, the original size, and an optional drop callback)
//!   immediately followed by the caller-requested payload bytes; return
//!   a pointer to the *payload* (the header is never visible to
//!   `nether_codegen` or Nether source — there are no raw pointers in
//!   the language at all).
//! - `nether_rt_arc_retain`/`nether_rt_arc_release` mutate only the
//!   *strong* count; `release` additionally runs the stored drop
//!   callback once it reaches zero (releasing the object's own ARC'd
//!   fields — see below), and frees the underlying memory block once
//!   *both* counts have reached zero.
//! - `nether_rt_arc_weak_retain`/`nether_rt_arc_weak_release` mutate only
//!   the *weak* count — `weak T` never affects the strong count (spec
//!   §13) — and `nether_rt_arc_weak_upgrade` is the only safe way to read
//!   through a weak reference: it checks the strong count and, if still
//!   alive, retains and hands back a real, strong-owned pointer.
//!
//! Why two separate counts: a weak reference must never dereference
//! freed memory (there are no unsafe references in this language at
//! all), but it also must never keep the referent's *fields* alive once
//! its strong count hits zero (that's what "weak" means). So dropping to
//! a strong count of zero runs the drop callback immediately — the
//! object is semantically dead, and any `weak_upgrade` from then on
//! correctly reports "gone" — but the header block itself (specifically,
//! its strong-count field, which `weak_upgrade` still needs to read)
//! isn't actually freed until the weak count *also* reaches zero. This
//! is the same two-count design Rust's own `Arc`/`Weak` and Swift's ARC
//! use, for the same reason.
//!
//! Why a drop callback instead of this crate knowing how to recursively
//! release nested fields itself: this crate is generic, plain Rust with
//! no knowledge of Nether's type system. `nether_codegen` is the one
//! place that knows a given heap type's field layout, so it generates a
//! small drop-shim function per heap-kind type (releasing that type's
//! own ARC'd fields, strong *or* weak) and hands its pointer to
//! `nether_rt_arc_alloc` at construction time — see
//! `nether_codegen::shims`'s module docs. `runtime/string` and
//! `runtime/array` are the two heap types whose drop callback is instead
//! a fixed function defined in this workspace (freeing their own
//! internal buffer), never one `codegen` generates.
//!
//! Dependencies: none beyond `std`.
//!
//! Consumers: `runtime/string`, `runtime/array`, and every heap-typed
//! value's generated code (via `nether_codegen`'s declared external
//! symbols).
//!
//! Invariants: retain/release (strong or weak) are the only operations
//! that change their respective count; single-threaded (Nether has no
//! concurrency, spec §14), so no atomics are needed. A null payload
//! pointer is a no-op for both (defensive — nothing in this workspace
//! currently produces one, but it's a cheap guard against a real crash
//! class rather than a hypothetical one).
//!
//! Future extension points: a cycle-detection lint (spec §14) would be a
//! separate static-analysis tool, not a change here — this crate stays a
//! minimal, correct refcounting primitive.

use std::alloc::{alloc, dealloc, handle_alloc_error, Layout};

/// Every allocation is aligned to this boundary — large enough for any
/// primitive or pointer field a Nether struct can contain on
/// `aarch64-apple-darwin`.
const ALIGN: usize = 16;

/// The hidden bookkeeping block placed immediately before every payload
/// this crate hands out.
#[repr(C)]
struct Header {
    strong: i64,
    weak: i64,
    /// The payload size passed to [`nether_rt_arc_alloc`] — freeing the
    /// block needs it back to reconstruct the exact [`Layout`] `dealloc`
    /// requires.
    size: i64,
    drop: Option<extern "C" fn(*mut u8)>,
}

/// Header for uniquely owned heap objects. It deliberately contains no
/// reference counts; only destruction metadata needed for the sole owner.
#[repr(C)]
struct UniqueHeader {
    size: i64,
    drop: Option<extern "C" fn(*mut u8)>,
}

const HEADER_SIZE: usize = std::mem::size_of::<Header>();
const UNIQUE_HEADER_SIZE: usize = std::mem::size_of::<UniqueHeader>();

/// # Safety
/// `payload` must be a pointer previously returned by
/// [`nether_rt_arc_alloc`] and not yet freed (both counts zero).
unsafe fn header_of(payload: *mut u8) -> *mut Header {
    payload.sub(HEADER_SIZE).cast::<Header>()
}

fn layout_for(payload_size: i64) -> Layout {
    let total = HEADER_SIZE + usize::try_from(payload_size).unwrap_or(0);
    Layout::from_size_align(total, ALIGN).expect("nether_rt_arc: invalid allocation size")
}

fn unique_layout_for(payload_size: i64) -> Layout {
    let total = UNIQUE_HEADER_SIZE + usize::try_from(payload_size).unwrap_or(0);
    Layout::from_size_align(total, ALIGN).expect("nether_rt_arc: invalid unique allocation size")
}

unsafe fn unique_header_of(payload: *mut u8) -> *mut UniqueHeader {
    unsafe { payload.sub(UNIQUE_HEADER_SIZE).cast::<UniqueHeader>() }
}

#[no_mangle]
pub extern "C" fn nether_rt_unique_alloc(
    size: i64,
    drop: Option<extern "C" fn(*mut u8)>,
) -> *mut u8 {
    let layout = unique_layout_for(size);
    unsafe {
        let base = alloc(layout);
        if base.is_null() {
            handle_alloc_error(layout);
        }
        base.cast::<UniqueHeader>()
            .write(UniqueHeader { size, drop });
        base.add(UNIQUE_HEADER_SIZE)
    }
}

#[no_mangle]
pub unsafe extern "C" fn nether_rt_unique_free(payload: *mut u8) {
    if payload.is_null() {
        return;
    }
    unsafe {
        let header = unique_header_of(payload);
        if let Some(drop_fn) = (*header).drop {
            drop_fn(payload);
        }
        dealloc(header.cast::<u8>(), unique_layout_for((*header).size));
    }
}

/// Converts the sole-owner allocation into an ARC allocation without
/// cloning its fields: bytes move to a new ARC block and the old unique
/// block is deallocated without running its payload drop callback.
#[no_mangle]
pub unsafe extern "C" fn nether_rt_unique_promote(payload: *mut u8) -> *mut u8 {
    if payload.is_null() {
        return payload;
    }
    unsafe {
        let header = unique_header_of(payload);
        let size = (*header).size;
        let drop = (*header).drop;
        let promoted = nether_rt_arc_alloc(size, drop);
        std::ptr::copy_nonoverlapping(payload, promoted, usize::try_from(size).unwrap_or(0));
        dealloc(header.cast::<u8>(), unique_layout_for(size));
        promoted
    }
}

/// # Safety
/// `header` must point at a header whose strong *and* weak counts are
/// both already zero, not yet freed.
unsafe fn dealloc_header(header: *mut Header) {
    unsafe {
        let layout = layout_for((*header).size);
        dealloc(header.cast::<u8>(), layout);
    }
}

/// Allocates a refcounted block of `size` payload bytes at strong count
/// 1 and weak count 0, with `drop` recorded to run (if present) when the
/// strong count reaches zero. Returns a pointer to the payload, not the
/// header.
#[no_mangle]
pub extern "C" fn nether_rt_arc_alloc(size: i64, drop: Option<extern "C" fn(*mut u8)>) -> *mut u8 {
    let layout = layout_for(size);
    unsafe {
        let base = alloc(layout);
        if base.is_null() {
            handle_alloc_error(layout);
        }
        base.cast::<Header>().write(Header {
            strong: 1,
            weak: 0,
            size,
            drop,
        });
        base.add(HEADER_SIZE)
    }
}

/// Increments `payload`'s strong count by one.
///
/// # Safety
/// `payload` must be null, or a pointer previously returned by
/// [`nether_rt_arc_alloc`] and not yet freed.
#[no_mangle]
pub unsafe extern "C" fn nether_rt_arc_retain(payload: *mut u8) {
    if payload.is_null() {
        return;
    }
    unsafe {
        let header = header_of(payload);
        (*header).strong += 1;
    }
}

/// Decrements `payload`'s strong count; at zero, runs its drop callback
/// (if any), then frees the block too if the weak count is *also* zero
/// (see this crate's own module docs on why the two counts are tracked
/// separately).
///
/// # Safety
/// Same precondition as [`nether_rt_arc_retain`].
#[no_mangle]
pub unsafe extern "C" fn nether_rt_arc_release(payload: *mut u8) {
    if payload.is_null() {
        return;
    }
    unsafe {
        let header = header_of(payload);
        (*header).strong -= 1;
        if (*header).strong == 0 {
            if let Some(drop_fn) = (*header).drop {
                drop_fn(payload);
            }
            if (*header).weak == 0 {
                dealloc_header(header);
            }
        }
    }
}

/// Increments `payload`'s *weak* count by one — never its strong count
/// (spec §13: "`weak T` never affects the retain count of its
/// referent"). Called whenever a fresh `weak T` is constructed from a
/// `T` (`nether_mir::Instr::WeakRetain`).
///
/// # Safety
/// Same precondition as [`nether_rt_arc_retain`].
#[no_mangle]
pub unsafe extern "C" fn nether_rt_arc_weak_retain(payload: *mut u8) {
    if payload.is_null() {
        return;
    }
    unsafe {
        let header = header_of(payload);
        (*header).weak += 1;
    }
}

/// Decrements `payload`'s weak count; frees the block if the strong
/// count has *also* already reached zero (the drop callback, if any,
/// already ran when the strong count itself hit zero — this only ever
/// frees the raw memory block, never re-runs it).
///
/// # Safety
/// Same precondition as [`nether_rt_arc_retain`].
#[no_mangle]
pub unsafe extern "C" fn nether_rt_arc_weak_release(payload: *mut u8) {
    if payload.is_null() {
        return;
    }
    unsafe {
        let header = header_of(payload);
        (*header).weak -= 1;
        if (*header).weak == 0 && (*header).strong == 0 {
            dealloc_header(header);
        }
    }
}

/// The only safe way to read through a `weak T`: if `payload`'s referent
/// is still alive (strong count > 0), retains it (a fresh, independently
/// -owned strong reference — the caller now owns exactly one, same as
/// any other call result, `arc-model.md` §3.4) and writes `payload` back
/// through `out_payload`, returning `1`; otherwise leaves `out_payload`
/// untouched and returns `0`. Shaped this way (an `i8` boolean plus an
/// out-parameter) so `nether_codegen` can build an `Option<T>`
/// construction directly from the result, exactly like
/// `nether_rt_array_pop`.
///
/// # Safety
/// `payload` must be null, or a pointer previously returned by
/// [`nether_rt_arc_alloc`] and not yet freed (its weak count must still
/// be at least 1, since the caller is only ever reading through an
/// already-weak-retained reference); `out_payload` must point at a
/// writable pointer-sized slot.
#[no_mangle]
pub unsafe extern "C" fn nether_rt_arc_weak_upgrade(
    payload: *mut u8,
    out_payload: *mut *mut u8,
) -> u8 {
    if payload.is_null() {
        return 0;
    }
    unsafe {
        let header = header_of(payload);
        if (*header).strong == 0 {
            return 0;
        }
        (*header).strong += 1;
        *out_payload = payload;
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    static DROPS: AtomicUsize = AtomicUsize::new(0);
    static DROP_TEST_LOCK: Mutex<()> = Mutex::new(());

    extern "C" fn record_drop(_payload: *mut u8) {
        DROPS.fetch_add(1, Ordering::SeqCst);
    }

    #[test]
    fn alloc_then_release_once_runs_drop() {
        let _guard = DROP_TEST_LOCK.lock().unwrap();
        let before = DROPS.load(Ordering::SeqCst);
        let p = nether_rt_arc_alloc(8, Some(record_drop));
        unsafe {
            *p.cast::<i64>() = 42;
            nether_rt_arc_release(p);
        }
        assert_eq!(DROPS.load(Ordering::SeqCst), before + 1);
    }

    #[test]
    fn unique_free_runs_drop_exactly_once() {
        let _guard = DROP_TEST_LOCK.lock().unwrap();
        let before = DROPS.load(Ordering::SeqCst);
        let p = nether_rt_unique_alloc(8, Some(record_drop));
        unsafe {
            *p.cast::<i64>() = 42;
            nether_rt_unique_free(p);
        }
        assert_eq!(DROPS.load(Ordering::SeqCst), before + 1);
    }

    #[test]
    fn unique_promotion_preserves_payload_and_defers_drop_to_arc() {
        let _guard = DROP_TEST_LOCK.lock().unwrap();
        let before = DROPS.load(Ordering::SeqCst);
        let unique = nether_rt_unique_alloc(8, Some(record_drop));
        unsafe {
            *unique.cast::<i64>() = 42;
            let arc = nether_rt_unique_promote(unique);
            assert_eq!(*arc.cast::<i64>(), 42);
            assert_eq!(DROPS.load(Ordering::SeqCst), before);
            nether_rt_arc_release(arc);
        }
        assert_eq!(DROPS.load(Ordering::SeqCst), before + 1);
    }

    #[test]
    fn retain_delays_the_drop_until_the_matching_release() {
        let _guard = DROP_TEST_LOCK.lock().unwrap();
        let before = DROPS.load(Ordering::SeqCst);
        let p = nether_rt_arc_alloc(8, Some(record_drop));
        unsafe {
            nether_rt_arc_retain(p);
            nether_rt_arc_release(p);
        }
        assert_eq!(
            DROPS.load(Ordering::SeqCst),
            before,
            "still one live reference"
        );
        unsafe { nether_rt_arc_release(p) };
        assert_eq!(DROPS.load(Ordering::SeqCst), before + 1);
    }

    #[test]
    fn no_drop_callback_just_frees_silently() {
        let p = nether_rt_arc_alloc(8, None);
        unsafe { nether_rt_arc_release(p) };
    }

    #[test]
    fn null_payload_is_a_no_op() {
        unsafe {
            nether_rt_arc_retain(std::ptr::null_mut());
            nether_rt_arc_release(std::ptr::null_mut());
            nether_rt_arc_weak_retain(std::ptr::null_mut());
            nether_rt_arc_weak_release(std::ptr::null_mut());
            let mut out = std::ptr::null_mut();
            assert_eq!(
                nether_rt_arc_weak_upgrade(std::ptr::null_mut(), &mut out),
                0
            );
        }
    }

    #[test]
    fn weak_upgrade_succeeds_while_the_referent_is_still_alive() {
        let p = nether_rt_arc_alloc(8, None);
        unsafe {
            nether_rt_arc_weak_retain(p);
            let mut out: *mut u8 = std::ptr::null_mut();
            let ok = nether_rt_arc_weak_upgrade(p, &mut out);
            assert_eq!(ok, 1);
            assert_eq!(out, p);
            // The upgrade itself retained (strong count now 2) — release
            // both the upgrade's own reference and the original.
            nether_rt_arc_release(p);
            nether_rt_arc_release(p);
            nether_rt_arc_weak_release(p);
        }
    }

    #[test]
    fn weak_upgrade_fails_once_the_referent_is_dropped_but_the_header_survives() {
        let _guard = DROP_TEST_LOCK.lock().unwrap();
        let before = DROPS.load(Ordering::SeqCst);
        let p = nether_rt_arc_alloc(8, Some(record_drop));
        unsafe {
            nether_rt_arc_weak_retain(p);
            // Strong count hits zero: drop callback runs, but the header
            // block itself must survive (weak count is still 1) so this
            // upgrade attempt can safely read it rather than touching
            // freed memory.
            nether_rt_arc_release(p);
            assert_eq!(
                DROPS.load(Ordering::SeqCst),
                before + 1,
                "drop callback should have run already"
            );
            let mut out: *mut u8 = std::ptr::null_mut();
            let ok = nether_rt_arc_weak_upgrade(p, &mut out);
            assert_eq!(
                ok, 0,
                "the referent is gone, upgrade must report failure, not read freed memory"
            );
            nether_rt_arc_weak_release(p);
        }
    }
}
