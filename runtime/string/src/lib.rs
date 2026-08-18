//! # nether_rt_string
//!
//! Purpose: the `String` heap type's storage and operations
//! (`docs/architecture/crates.md` § `runtime/string`).
//!
//! Responsibilities: own a UTF-8 byte buffer behind
//! [`nether_rt_arc`]'s refcounted header, concatenation (used by
//! desugared string interpolation, per `hir`), and the numeric/`bool`/
//! `char` → `String` conversions `nether_codegen`'s `ToString` rvalue
//! calls into.
//!
//! A `String`'s own [`Header`] drop callback (`drop_buffer`, below) is a
//! fixed function defined in this crate, never one `nether_codegen`
//! generates — unlike a user-defined heap `struct`, a `String`'s payload
//! (a raw byte buffer) has no ARC'd sub-fields for a generated shim to
//! walk.
//!
//! Dependencies: `nether_rt_arc`.
//!
//! Consumers: generated code for any `String`-typed expression;
//! `nether_rt_io` (reads a `String`'s buffer to print it).
//!
//! Invariants: every `String` this crate produces holds valid UTF-8 — the
//! one entry point that accepts arbitrary bytes ([`nether_rt_string_from_utf8`])
//! validates them; every other constructor builds its bytes from `Rust`'s
//! own UTF-8-guaranteeing formatting.

use nether_rt_arc::nether_rt_arc_alloc;

/// A `String`'s payload: a pointer to `len` UTF-8 bytes it owns
/// exclusively (not itself refcounted — the `String` object as a whole
/// is, via the `nether_rt_arc` header this payload sits behind).
#[repr(C)]
struct StringRepr {
    ptr: *mut u8,
    len: i64,
}

extern "C" fn drop_buffer(payload: *mut u8) {
    unsafe {
        let repr = payload.cast::<StringRepr>();
        drop(Vec::from_raw_parts(
            (*repr).ptr,
            (*repr).len as usize,
            (*repr).len as usize,
        ));
    }
}

fn alloc_string(bytes: &[u8]) -> *mut u8 {
    let mut buf = bytes.to_vec().into_boxed_slice();
    let ptr = buf.as_mut_ptr();
    let len = buf.len();
    std::mem::forget(buf);
    let payload = nether_rt_arc_alloc(std::mem::size_of::<StringRepr>() as i64, Some(drop_buffer));
    unsafe {
        payload.cast::<StringRepr>().write(StringRepr {
            ptr,
            len: len as i64,
        });
    }
    payload
}

/// # Safety
/// `payload` must point at a live `String` object (one previously
/// returned by a function in this crate, not yet released to zero).
unsafe fn bytes_of<'a>(payload: *mut u8) -> &'a [u8] {
    unsafe {
        let repr = payload.cast::<StringRepr>();
        std::slice::from_raw_parts((*repr).ptr, (*repr).len as usize)
    }
}

/// Builds a new `String` by copying `len` bytes from `bytes`, which must
/// be valid UTF-8 (a lexer/codegen invariant for string literals — this
/// is the one boundary where that invariant is actually checked).
///
/// # Safety
/// `bytes` must point at `len` readable, valid UTF-8 bytes.
#[no_mangle]
pub unsafe extern "C" fn nether_rt_string_from_utf8(bytes: *const u8, len: i64) -> *mut u8 {
    let slice = unsafe { std::slice::from_raw_parts(bytes, len as usize) };
    std::str::from_utf8(slice).expect("nether_rt_string_from_utf8: input was not valid UTF-8");
    alloc_string(slice)
}

/// Builds a new `String` holding `a`'s bytes followed by `b`'s. Does not
/// take ownership of (or release) `a`/`b` — the new `String` copies
/// their bytes rather than sharing a reference to either, so both remain
/// exactly as owned by their existing bindings as before this call (per
/// `nether_mir`'s own ARC insertion: `Concat`'s operands are plain reads,
/// never wrapped in a call-argument retain/release, since nothing here
/// needs one).
///
/// # Safety
/// `a` and `b` must each point at a live `String` object.
#[no_mangle]
pub unsafe extern "C" fn nether_rt_string_concat(a: *mut u8, b: *mut u8) -> *mut u8 {
    let (a_bytes, b_bytes) = unsafe { (bytes_of(a), bytes_of(b)) };
    let mut combined = Vec::with_capacity(a_bytes.len() + b_bytes.len());
    combined.extend_from_slice(a_bytes);
    combined.extend_from_slice(b_bytes);
    alloc_string(&combined)
}

#[no_mangle]
pub extern "C" fn nether_rt_i64_to_string(v: i64) -> *mut u8 {
    alloc_string(v.to_string().as_bytes())
}

#[no_mangle]
pub extern "C" fn nether_rt_f64_to_string(v: f64) -> *mut u8 {
    alloc_string(v.to_string().as_bytes())
}

/// `v` is a C `_Bool`-shaped byte (0 or 1) — see `nether_codegen`'s own
/// note on why `bool` crosses this ABI boundary as `i8`, not LLVM's
/// native `i1`.
#[no_mangle]
pub extern "C" fn nether_rt_bool_to_string(v: u8) -> *mut u8 {
    alloc_string(if v != 0 { b"true" } else { b"false" })
}

/// `codepoint` is a Unicode scalar value; an invalid one (should never
/// happen — `char` is validated at parse time) falls back to U+FFFD
/// rather than producing undefined behavior.
#[no_mangle]
pub extern "C" fn nether_rt_char_to_string(codepoint: u32) -> *mut u8 {
    let c = char::from_u32(codepoint).unwrap_or('\u{FFFD}');
    let mut buf = [0u8; 4];
    alloc_string(c.encode_utf8(&mut buf).as_bytes())
}

/// Read-only accessors for `nether_rt_io` (and any other crate that needs
/// to inspect a `String`'s bytes without reaching into its private
/// layout).
///
/// # Safety
/// `payload` must point at a live `String` object.
#[no_mangle]
pub unsafe extern "C" fn nether_rt_string_bytes(payload: *mut u8) -> *const u8 {
    unsafe { (*payload.cast::<StringRepr>()).ptr }
}

/// # Safety
/// `payload` must point at a live `String` object.
#[no_mangle]
pub unsafe extern "C" fn nether_rt_string_len(payload: *mut u8) -> i64 {
    unsafe { (*payload.cast::<StringRepr>()).len }
}

/// Returns a deterministic, unkeyed hash of the String's UTF-8 bytes.
/// This intentionally shares the compiler's structural-hash constants;
/// it is suitable for language-level Hash containers, not adversarial input.
///
/// # Safety
/// `payload` must point at a live `String` object.
#[no_mangle]
pub unsafe extern "C" fn nether_rt_string_hash(payload: *mut u8) -> u64 {
    unsafe { bytes_of(payload) }
        .iter()
        .fold(1_469_598_103_934_665_603_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(1_099_511_628_211)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nether_rt_arc::nether_rt_arc_release;

    unsafe fn as_str<'a>(payload: *mut u8) -> &'a str {
        unsafe { std::str::from_utf8(bytes_of(payload)).unwrap() }
    }

    #[test]
    fn from_utf8_round_trips() {
        unsafe {
            let s = nether_rt_string_from_utf8(b"hello".as_ptr(), 5);
            assert_eq!(as_str(s), "hello");
            nether_rt_arc_release(s);
        }
    }

    #[test]
    fn concat_joins_bytes_and_leaves_its_inputs_untouched() {
        unsafe {
            let a = nether_rt_string_from_utf8(b"foo".as_ptr(), 3);
            let b = nether_rt_string_from_utf8(b"bar".as_ptr(), 3);
            let joined = nether_rt_string_concat(a, b);
            assert_eq!(as_str(joined), "foobar");
            assert_eq!(
                as_str(a),
                "foo",
                "concat must not consume/release its inputs"
            );
            nether_rt_arc_release(joined);
            nether_rt_arc_release(a);
            nether_rt_arc_release(b);
        }
    }

    #[test]
    fn numeric_and_bool_and_char_conversions() {
        unsafe {
            let i = nether_rt_i64_to_string(-7);
            assert_eq!(as_str(i), "-7");
            nether_rt_arc_release(i);

            let f = nether_rt_f64_to_string(1.5);
            assert_eq!(as_str(f), "1.5");
            nether_rt_arc_release(f);

            let t = nether_rt_bool_to_string(1);
            assert_eq!(as_str(t), "true");
            nether_rt_arc_release(t);

            let f_bool = nether_rt_bool_to_string(0);
            assert_eq!(as_str(f_bool), "false");
            nether_rt_arc_release(f_bool);

            let c = nether_rt_char_to_string('λ' as u32);
            assert_eq!(as_str(c), "λ");
            nether_rt_arc_release(c);
        }
    }

    #[test]
    fn string_hash_is_deterministic_and_content_based() {
        unsafe {
            let a = nether_rt_string_from_utf8(b"same".as_ptr(), 4);
            let b = nether_rt_string_from_utf8(b"same".as_ptr(), 4);
            let c = nether_rt_string_from_utf8(b"different".as_ptr(), 9);
            assert_eq!(nether_rt_string_hash(a), nether_rt_string_hash(b));
            assert_ne!(nether_rt_string_hash(a), nether_rt_string_hash(c));
            nether_rt_arc_release(a);
            nether_rt_arc_release(b);
            nether_rt_arc_release(c);
        }
    }
}
