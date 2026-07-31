//! # nether_rt_io
//!
//! Purpose: backing implementation for `print`/`println`
//! (`docs/architecture/crates.md` § `runtime/io`).
//!
//! Responsibilities: write a `String`'s UTF-8 bytes to stdout.
//! `print`/`println` are built-in symbols (spec §12); this crate is what
//! their generated calls ultimately reach.
//!
//! Dependencies: `nether_rt_string` (reads a `String`'s buffer through
//! its own accessor functions, never reaching into its private layout).
//!
//! Consumers: generated code for any `print`/`println` call expression.
//!
//! Invariants: no buffering surprises — both functions flush immediately
//! (`Write::flush`), matching the simplicity bar of the rest of the MVP.

use std::io::Write;

use nether_rt_string::{nether_rt_string_bytes, nether_rt_string_len};

fn bytes_of(s: *mut u8) -> &'static [u8] {
    unsafe {
        std::slice::from_raw_parts(nether_rt_string_bytes(s), nether_rt_string_len(s) as usize)
    }
}

#[no_mangle]
pub extern "C" fn nether_rt_io_print(s: *mut u8) {
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let _ = lock.write_all(bytes_of(s));
    let _ = lock.flush();
}

#[no_mangle]
pub extern "C" fn nether_rt_io_println(s: *mut u8) {
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let _ = lock.write_all(bytes_of(s));
    let _ = lock.write_all(b"\n");
    let _ = lock.flush();
}

#[cfg(test)]
mod tests {
    use super::*;
    use nether_rt_string::nether_rt_string_from_utf8;

    #[test]
    fn print_and_println_do_not_panic_on_a_real_string() {
        let s = unsafe { nether_rt_string_from_utf8(b"hello".as_ptr(), 5) };
        nether_rt_io_print(s);
        nether_rt_io_println(s);
    }
}
