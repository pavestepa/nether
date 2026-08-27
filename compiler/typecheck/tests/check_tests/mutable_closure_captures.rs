use super::*;

#[test]
fn mut_closure_capturing_an_inline_mut_local_type_checks() {
    assert_ok(
        r#"
fn main() {
    let mut count = 0;
    let mut inc = mut () => {
        count = count + 1;
    };
    inc();
    println(`${count}`);
}
"#,
    );
}

#[test]
fn mut_closure_capturing_a_heap_typed_mut_local_still_type_checks() {
    // A heap-category `mut` capture silently stays `ByValue` rather than
    // being rejected — field/method mutation through the shared pointer
    // already works without a by-reference capture at all (§16 v1 scope,
    // documented in the language spec and roadmap).
    assert_ok(
        r#"
struct Counter { value i32 }
impl Counter {
    increment(mut self) {
        self.value = self.value + 1;
    }
}
fn main() {
    let mut c = Counter { value = 1 };
    let mut bump = mut () => {
        c.increment();
    };
    bump();
    println(`${c.value}`);
}
"#,
    );
}

#[test]
fn returning_a_mut_closure_that_captures_a_local_owned_by_this_function_is_rejected() {
    assert_err(
        r#"
fn make_counter() () => i32 {
    let mut count = 0;
    return mut () => {
        count = count + 1;
        return count;
    };
}
fn main() {
    let inc = make_counter();
    println(`${inc()}`);
}
"#,
        "cannot return a closure that mutably captures a local owned by this function",
    );
}

#[test]
fn mut_closure_capturing_only_immutable_locals_does_not_trigger_the_escape_check() {
    // No local here is declared `mut`, so nothing gets promoted to
    // `CaptureMode::ByRef` — returning the closure directly is fine.
    assert_ok(
        r#"
fn make_adder(base i32) () => i32 {
    return mut () => { base + 1 };
}
fn main() {
    let add = make_adder(1);
    println(`${add()}`);
}
"#,
    );
}

#[test]
fn ordinary_non_mut_closure_capturing_a_mut_local_does_not_write_back() {
    // Without the `mut` modifier, capture-by-value is unchanged from
    // before Stage 7: mutating the captured copy inside the closure never
    // affects the outer binding.
    assert_ok(
        r#"
fn main() {
    let mut count = 0;
    let inc = () => {
        count = count + 1;
    };
    inc();
    println(`${count}`);
}
"#,
    );
}
