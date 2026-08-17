use super::*;

#[test]
fn shared_borrow_can_be_stored_and_read() {
    assert_ok(
        r#"
struct Dog { name String }
fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    let view: &Dog = dog;
    println(view.name);
    println(dog.name);
}
"#,
    );
}

#[test]
fn mutable_stored_borrow_is_exclusive_and_can_mutate() {
    assert_ok(
        r#"
struct Dog { name String }
fn main() {
    let mut dog: Dog = :Dog { name = "Rex" };
    let view: &mut Dog = dog;
    view.name = "Buddy";
    println(view.name);
}
"#,
    );
    assert_err(
        r#"
struct Dog { name String }
fn main() {
    let mut dog: Dog = :Dog { name = "Rex" };
    let view: &mut Dog = dog;
    println(dog.name);
    println(view.name);
}
"#,
        "mutably borrowed",
    );
}

#[test]
fn live_stored_borrow_prevents_move_and_conflicting_borrow() {
    assert_err(
        r#"
struct Dog { name String }
fn consume(d: Dog) {}
fn main() {
    let mut dog: Dog = :Dog { name = "Rex" };
    let first: &Dog = dog;
    consume(dog);
    println(first.name);
}

"#,
        "while it is borrowed",
    );
    assert_err(
        r#"
struct Dog { name String }
fn main() {
    let mut dog: Dog = :Dog { name = "Rex" };
    let first: &Dog = dog;
    let second: &mut Dog = dog;
    println(first.name);
    println(second.name);
}
"#,
        "already-live stored borrow",
    );
}

#[test]
fn live_borrow_prevents_direct_mutation_and_conflicting_call_borrow() {
    assert_err(
        r#"
struct Dog { name String }
fn main() {
    let mut dog: Dog = :Dog { name = "Rex" };
    let view: &Dog = dog;
    dog.name = "Buddy";
    println(view.name);
}
"#,
        "cannot mutate a value while it is borrowed",
    );
    assert_err(
        r#"
struct Dog { name String }
fn rename(d: &mut Dog) { d.name = "Buddy"; }
fn main() {
    let mut dog: Dog = :Dog { name = "Rex" };
    let view: &Dog = dog;
    rename(dog);
    println(view.name);
}
"#,
        "call borrow conflicts",
    );
}

#[test]
fn borrow_ends_at_lexical_scope_boundary() {
    assert_ok(
        r#"
struct Dog { name String }
fn consume(d: Dog) {}
fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    if true {
        let view: &Dog = dog;
        println(view.name);
    }
    consume(dog);
}
"#,
    );
}
