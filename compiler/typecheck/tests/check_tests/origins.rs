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

#[test]
fn returned_reference_infers_one_parameter_origin() {
    assert_ok(
        r#"
struct Dog { name String }
fn identity(d: &Dog): &Dog { return d; }
fn choose(flag bool, d: &Dog): &Dog {
    return if flag { d } else { d };
}
fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    println(identity(dog).name);
    println(choose(true, dog).name);
}
"#,
    );
}

#[test]
fn returning_local_borrow_is_rejected() {
    assert_err(
        r#"
struct Dog { name String }
fn invalid(): &Dog {
    let dog: Dog = :Dog { name = "Rex" };
    let view: &Dog = dog;
    return view;
}
"#,
        "borrowed from a local owned value",
    );
}

#[test]
fn returned_reference_with_multiple_parameter_origins_is_ambiguous() {
    assert_err(
        r#"
struct Dog { name String }
fn choose(flag bool, left: &Dog, right: &Dog): &Dog {
    return if flag { left } else { right };
}
"#,
        "ambiguous origins",
    );
}

#[test]
fn returned_reference_origin_flows_through_call_chains_and_into_let() {
    assert_ok(
        r#"
struct Dog { name String }
fn identity(d: &Dog): &Dog { return d; }
fn middle(d: &Dog): &Dog { return identity(d); }
fn outer(d: &Dog): &Dog { return middle(d); }
fn consume(d: Dog) {}
fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    if true {
        let view = outer(dog);
        println(view.name);
    }
    consume(dog);
}
"#,
    );
}

#[test]
fn stored_call_result_keeps_its_argument_borrow_live() {
    assert_err(
        r#"
struct Dog { name String }
fn identity(d: &Dog): &Dog { return d; }
fn middle(d: &Dog): &Dog { return identity(d); }
fn consume(d: Dog) {}
fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    let view: &Dog = middle(dog);
    consume(dog);
    println(view.name);
}
"#,
        "while it is borrowed",
    );
}

#[test]
fn ambiguous_origin_propagates_through_callers() {
    assert_err(
        r#"
struct Dog { name String }
fn choose(flag bool, left: &Dog, right: &Dog): &Dog {
    return if flag { left } else { right };
}
fn forwarded(flag bool, left: &Dog, right: &Dog): &Dog {
    return choose(flag, left, right);
}
"#,
        "ambiguous origins",
    );
}
