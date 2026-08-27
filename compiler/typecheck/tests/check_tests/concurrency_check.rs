use super::*;

#[test]
fn unsafe_impl_send_and_sync_on_a_struct_type_checks() {
    assert_ok(
        r#"
struct Handle { value i32 }
unsafe impl Handle Send { }
unsafe impl Handle Sync { }
fn main() {}
"#,
    );
}

#[test]
fn unsafe_impl_is_rejected_for_any_trait_other_than_send_or_sync() {
    assert_err(
        r#"
struct Dog { name String }
unsafe impl Dog Eq { }
fn main() {}
"#,
        "`unsafe impl` is only allowed for the `Send`/`Sync` marker traits",
    );
}

#[test]
fn plain_impl_of_send_or_sync_without_unsafe_is_rejected() {
    assert_err(
        r#"
struct Handle { value i32 }
impl Handle Send { }
fn main() {}
"#,
        "`Send`/`Sync` can only be implemented via `unsafe impl`",
    );
}

#[test]
fn unsafe_impl_send_with_a_method_body_is_rejected() {
    assert_err(
        r#"
struct Handle { value i32 }
unsafe impl Handle Send {
    extra(self) i32 { return self.value; }
}
fn main() {}
"#,
        "must have an empty body",
    );
}

#[test]
fn thread_spawn_requires_a_move_closure_literal() {
    assert_err(
        r#"
fn main() {
    let make = () => { };
    thread.spawn(make);
}
"#,
        "`thread.spawn` requires a `move` closure literal argument",
    );
    assert_err(
        r#"
fn main() {
    thread.spawn(() => { });
}
"#,
        "a `thread.spawn` closure must be `move`",
    );
}

#[test]
fn thread_spawn_and_join_of_a_unit_closure_type_checks() {
    assert_ok(
        r#"
fn main() {
    let handle = thread.spawn(move () => { println("hi"); });
    handle.join();
}
"#,
    );
}

#[test]
fn thread_spawn_closure_must_return_unit() {
    assert_err(
        r#"
fn main() {
    thread.spawn(move () => { 5 });
}
"#,
        "a `thread.spawn` closure must return `()`",
    );
}

#[test]
fn thread_spawn_rejects_capturing_a_non_send_value() {
    assert_err(
        r#"
struct Inner { value i32 }
struct Outer { inner Inner }
fn main() {
    let outer = Outer { inner = Inner { value = 1 } };
    thread.spawn(move () => {
        println(`${outer.inner.value}`);
    });
}
"#,
        "is not `Send`",
    );
}

#[test]
fn thread_spawn_accepts_capturing_an_unsafe_impl_send_value() {
    assert_ok(
        r#"
struct Inner { value i32 }
struct Outer { inner Inner }
unsafe impl Outer Send { }
fn main() {
    let outer = Outer { inner = Inner { value = 1 } };
    thread.spawn(move () => {
        println(`${outer.inner.value}`);
    });
}
"#,
    );
}

#[test]
fn thread_spawn_rejects_capturing_a_borrow_of_a_local_owned_by_the_spawning_function() {
    assert_err(
        r#"
struct Dog { name String }
fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    let r: &Dog = dog;
    thread.spawn(move () => {
        println(r.name);
    });
}
"#,
        "cannot spawn a thread capturing a borrow of a local owned by this function",
    );
}
