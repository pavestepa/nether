use super::*;

/// Stage 2's move checker (language-spec §3): flow-sensitive use-after-
/// move / double-move detection for `:T`/`:t` locals. Only whole-value
/// reads of a `Type::Unique`-typed *local binding* are tracked — reading a
/// field/element through one is never itself a move (language-spec §4.2:
/// fields/elements are always ordinary-typed), and neither is a
/// `: &self`/`: &mut self` (borrowing) method receiver.

#[test]
fn simple_use_after_move_is_rejected() {
    assert_err(
        r#"
struct Dog { name String }
fn consume(d: Dog) { println(d.name); }
fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    consume(dog);
    consume(dog);
}
"#,
        "use of a value after it was moved",
    );
}

#[test]
fn move_then_reassign_then_use_is_ok() {
    assert_ok(
        r#"
struct Dog { name String }
fn consume(d: Dog) { println(d.name); }
fn main() {
    let mut dog: Dog = :Dog { name = "Rex" };
    consume(dog);
    dog = :Dog { name = "Buddy" };
    consume(dog);
}
"#,
    );
}

#[test]
fn owned_inline_locals_are_tracked_the_same_as_owned_structs() {
    assert_err(
        r#"
fn consume(x: i32) { println(`${x}`); }
fn main() {
    let n: i32 = 10;
    consume(n);
    consume(n);
}
"#,
        "use of a value after it was moved",
    );
}

#[test]
fn move_in_exactly_one_if_arm_then_used_after_is_rejected() {
    assert_err(
        r#"
struct Dog { name String }
fn consume(d: Dog) { println(d.name); }
fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    if true {
        consume(dog);
    }
    consume(dog);
}
"#,
        "use of a value after it was moved",
    );
}

#[test]
fn move_in_neither_if_arm_is_ok() {
    assert_ok(
        r#"
struct Dog { name String }
fn consume(d: Dog) { println(d.name); }
fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    if true {
        println("skip");
    } else {
        println("also skip");
    }
    consume(dog);
}
"#,
    );
}

#[test]
fn move_in_both_if_arms_is_ok_afterward_but_rejected_if_used_again() {
    assert_err(
        r#"
struct Dog { name String }
fn consume(d: Dog) { println(d.name); }
fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    if true {
        consume(dog);
    } else {
        consume(dog);
    }
    consume(dog);
}
"#,
        "use of a value after it was moved",
    );
}

#[test]
fn move_in_a_diverging_if_arm_does_not_count_against_the_other_arm() {
    // The `then` arm unconditionally returns, so it can never reach the
    // code after the `if` — its own move of `dog` shouldn't be merged in.
    assert_ok(
        r#"
struct Dog { name String }
fn consume(d: Dog) { println(d.name); }
fn describe(dog: Dog) String {
    if true {
        consume(dog);
        return "consumed";
    }
    println(dog.name);
    return "not consumed";
}
fn main() {}
"#,
    );
}

#[test]
fn move_unconditionally_inside_a_while_loop_body_is_rejected() {
    assert_err(
        r#"
struct Dog { name String }
fn consume(d: Dog) { println(d.name); }
fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    let mut i = 0;
    while i < 3 {
        consume(dog);
        i = i + 1;
    }
}
"#,
        "use of a value after it was moved",
    );
}

#[test]
fn a_fresh_let_inside_a_loop_body_moving_its_own_value_each_iteration_is_ok() {
    // `dog` is declared *inside* the loop body — each pass gets a fresh
    // binding, unlike a value captured from outside the loop.
    assert_ok(
        r#"
struct Dog { name String }
fn consume(d: Dog) { println(d.name); }
fn main() {
    let mut i = 0;
    while i < 3 {
        let dog: Dog = :Dog { name = "Rex" };
        consume(dog);
        i = i + 1;
    }
}
"#,
    );
}

#[test]
fn repeated_field_reads_do_not_consume_the_owning_local() {
    assert_ok(
        r#"
struct Dog { name String }
fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    println(dog.name);
    println(dog.name);
    println(dog.name);
}
"#,
    );
}

#[test]
fn field_reads_do_not_block_a_later_whole_value_move() {
    assert_ok(
        r#"
struct Dog { name String }
fn consume(d: Dog) { println(d.name); }
fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    println(dog.name);
    println(dog.name);
    consume(dog);
}
"#,
    );
}

#[test]
fn owned_consuming_method_call_moves_the_receiver() {
    assert_err(
        r#"
struct Dog { name String }
impl Dog {
    destroy(: self) { println(self.name); }
}
fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    dog.destroy();
    dog.destroy();
}
"#,
        "use of a value after it was moved",
    );
}

#[test]
fn owned_borrowing_method_call_does_not_move_the_receiver() {
    assert_ok(
        r#"
struct Dog { name String }
impl Dog {
    get_name(: &self) String { return self.name; }
}
fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    println(dog.get_name());
    println(dog.get_name());
}
"#,
    );
}

#[test]
fn a_fresh_owned_literal_passed_directly_is_never_flagged() {
    // Not a named local — nothing to track.
    assert_ok(
        r#"
struct Dog { name String }
fn consume(d: Dog) { println(d.name); }
fn main() {
    consume(:Dog { name = "Rex" });
    consume(:Dog { name = "Buddy" });
}
"#,
    );
}

#[test]
fn closure_literal_moves_a_captured_owned_local_even_if_never_called() {
    assert_err(
        r#"
struct Dog { name String }
fn consume(d: Dog) { println(d.name); }
fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    let f = () => { consume(dog); };
    println(dog.name);
}
"#,
        "use of a value after it was moved",
    );
}

#[test]
fn double_move_reports_both_the_original_move_and_the_reuse() {
    let diags = check_source(
        r#"
struct Dog { name String }
fn consume(d: Dog) { println(d.name); }
fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    consume(dog);
    consume(dog);
}
"#,
    );
    let diag = diags
        .iter()
        .find(|d| d.message.contains("use of a value after it was moved"))
        .expect("expected a use-after-move diagnostic");
    assert_eq!(
        diag.labels.len(),
        2,
        "expected two labels (moved here / used again here), got: {:?}",
        diag.labels
    );
}
