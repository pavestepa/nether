use super::*;

#[test]
fn raw_deref_requires_unsafe_context() {
    assert_err(
        r#"
fn main() {
    let mut x = 1;
    let p = &raw mut x;
    let value = *p;
}
"#,
        "dereferencing a raw pointer is only allowed inside an `unsafe` block or function",
    );
}

#[test]
fn unsafe_fn_call_requires_unsafe_context() {
    assert_err(
        r#"
unsafe fn danger() i32 { return 1; }
fn main() { let v = danger(); }
"#,
        "calling an `unsafe fn` is only allowed inside an `unsafe` block or function",
    );
}

#[test]
fn extern_fn_call_requires_unsafe_context() {
    assert_err(
        r#"
extern "C" {
    fn abs(n i32) i32;
}
fn main() { let v = abs(-1); }
"#,
        "calling an `extern` function is only allowed inside an `unsafe` block or function",
    );
}

#[test]
fn raw_borrow_does_not_require_unsafe_context() {
    assert_ok(
        r#"
fn main() {
    let mut x = 1;
    let p = &raw mut x;
    let q = &raw const x;
}
"#,
    );
}

#[test]
fn assigning_through_a_const_raw_pointer_is_rejected() {
    assert_err(
        r#"
fn main() {
    let x = 1;
    let p = &raw const x;
    unsafe { *p = 2; }
}
"#,
        "cannot assign through a `*const` pointer — expected `*mut`",
    );
}

#[test]
fn assigning_through_a_mut_raw_pointer_inside_unsafe_type_checks_and_runs() {
    assert_ok(
        r#"
fn main() {
    let mut x = 1;
    let p = &raw mut x;
    unsafe { *p = 2; }
    let value = unsafe { *p };
}
"#,
    );
}

#[test]
fn extern_fn_rejects_generics() {
    assert_err(
        r#"
extern "C" {
    fn identity<T>(x T) T;
}
fn main() {}
"#,
        "an `extern` function cannot be generic",
    );
}

#[test]
fn extern_fn_rejects_a_non_ffi_safe_parameter_type() {
    assert_err(
        r#"
struct Point { x i32 }
extern "C" {
    fn bad(p Point) i32;
}
fn main() {}
"#,
        "is not a valid `extern \"C\"` parameter type",
    );
}

#[test]
fn extern_fn_rejects_a_bare_reference_parameter_type() {
    assert_err(
        r#"
extern "C" {
    fn bad(p :&i32) i32;
}
fn main() {}
"#,
        "is not a valid `extern \"C\"` parameter type",
    );
}

#[test]
fn extern_fn_accepts_primitives_and_raw_pointers_and_calls_cleanly_inside_unsafe() {
    assert_ok(
        r#"
extern "C" {
    fn abs(n i32) i32;
    fn store(p *mut i32, v i32);
}
fn main() {
    let value = unsafe { abs(-4) };
    println(`${value}`);
    let mut x = 1;
    let p = &raw mut x;
    unsafe { store(p, 9); }
}
"#,
    );
}

#[test]
fn mut_raw_pointer_coerces_to_const_but_not_the_reverse() {
    assert_ok(
        r#"
fn takes_const(p *const i32) {}
fn main() {
    let mut x = 1;
    let p = &raw mut x;
    takes_const(p);
}
"#,
    );
    assert_err(
        r#"
fn takes_mut(p *mut i32) {}
fn main() {
    let x = 1;
    let p = &raw const x;
    takes_mut(p);
}
"#,
        "expected",
    );
}
