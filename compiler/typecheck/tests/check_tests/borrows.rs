use super::*;

/// Stage 2, slice 2: callable `:&T`/`:&mut T` parameters (language-spec
/// §8.1) — an owned (`:T`) local satisfies a reference parameter by name,
/// without consuming it, with call-scoped exclusivity (no double-mutable-
/// or mixed-mutable/shared borrow of the same local within one call).

#[test]
fn owned_local_satisfies_a_ref_param_and_is_not_consumed() {
    assert_ok(
        r#"
struct Dog { name String }
fn show(d: &Dog) { println(d.name); }
fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    show(dog);
    show(dog);
}
"#,
    );
}

#[test]
fn owned_local_satisfies_a_mut_ref_param_when_mut() {
    assert_ok(
        r#"
struct Dog { name String }
fn show(d: &mut Dog) { println(d.name); }
fn main() {
    let mut dog: Dog = :Dog { name = "Rex" };
    show(dog);
}
"#,
    );
}

#[test]
fn non_mut_binding_cannot_satisfy_a_mut_ref_param() {
    assert_err(
        r#"
struct Dog { name String }
fn show(d: &mut Dog) { println(d.name); }
fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    show(dog);
}
"#,
        "cannot borrow an immutable binding as `:&mut`",
    );
}

#[test]
fn arc_domain_value_cannot_satisfy_a_ref_param() {
    assert_err(
        r#"
struct Dog { name String }
fn show(d: &Dog) { println(d.name); }
fn main() {
    let dog = Dog { name = "Rex" };
    show(dog);
}
"#,
        "expected an owned",
    );
}

#[test]
fn a_temporary_cannot_satisfy_a_ref_param() {
    assert_err(
        r#"
struct Dog { name String }
fn show(d: &Dog) { println(d.name); }
fn main() {
    show(:Dog { name = "Rex" });
}
"#,
        "must be a plain local variable",
    );
}

#[test]
fn double_mutable_borrow_in_one_call_is_rejected() {
    assert_err(
        r#"
struct Dog { name String }
fn swap_names(a: &mut Dog, b: &mut Dog) { println(a.name); println(b.name); }
fn main() {
    let mut dog: Dog = :Dog { name = "Rex" };
    swap_names(dog, dog);
}
"#,
        "cannot borrow a value as mutable more than once",
    );
}

#[test]
fn mixed_mutable_and_shared_borrow_in_one_call_is_rejected() {
    assert_err(
        r#"
struct Dog { name String }
fn look(a: &Dog, b: &mut Dog) { println(a.name); println(b.name); }
fn main() {
    let mut dog: Dog = :Dog { name = "Rex" };
    look(dog, dog);
}
"#,
        "cannot borrow a value as mutable more than once",
    );
}

#[test]
fn two_shared_borrows_of_the_same_local_in_one_call_are_ok() {
    assert_ok(
        r#"
struct Dog { name String }
fn compare(a: &Dog, b: &Dog) { println(a.name); println(b.name); }
fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    compare(dog, dog);
}
"#,
    );
}

#[test]
fn field_access_through_a_ref_param_works() {
    assert_ok(
        r#"
struct Dog { name String }
fn show(d: &Dog) String { return d.name; }
fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    let n String = show(dog);
    println(n);
}
"#,
    );
}

#[test]
fn borrowing_an_already_moved_local_is_rejected() {
    assert_err(
        r#"
struct Dog { name String }
fn consume(d: Dog) { println(d.name); }
fn show(d: &Dog) { println(d.name); }
fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    consume(dog);
    show(dog);
}
"#,
        "use of a value after it was moved",
    );
}

#[test]
fn borrowing_then_moving_still_works() {
    assert_ok(
        r#"
struct Dog { name String }
fn show(d: &Dog) { println(d.name); }
fn consume(d: Dog) { println(d.name); }
fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    show(dog);
    consume(dog);
}
"#,
    );
}

/// Stage 2, slice 3: calling a method (not just reading a field) through a
/// `:&T`/`:&mut T` receiver — `: &self`/`: &mut self` bodies are reachable
/// this way, `: self` (consuming) ones are not (unsound: you'd be
/// consuming a value you only borrowed).

#[test]
fn borrowing_method_resolves_and_is_callable_through_a_ref_param() {
    assert_ok(
        r#"
struct Dog { name String }
impl Dog {
    get_name(: &self) String { return self.name; }
}
fn show(d: &Dog) String { return d.get_name(); }
fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    println(show(dog));
}
"#,
    );
}

#[test]
fn mut_borrowing_method_resolves_and_is_callable_through_a_mut_ref_param() {
    assert_ok(
        r#"
struct Dog { name String }
impl Dog {
    rename_ref(: &mut self, new_name String) { self.name = new_name; }
}
fn rename(d: &mut Dog) { d.rename_ref("Buddy"); }
fn main() {
    let mut dog: Dog = :Dog { name = "Rex" };
    rename(dog);
}
"#,
    );
}

#[test]
fn mut_borrowing_method_is_rejected_through_a_shared_ref_param() {
    assert_err(
        r#"
struct Dog { name String }
impl Dog {
    rename_ref(: &mut self, new_name String) { self.name = new_name; }
}
fn rename(d: &Dog) { d.rename_ref("Buddy"); }
fn main() {
    let mut dog: Dog = :Dog { name = "Rex" };
    rename(dog);
}
"#,
        "cannot call a `mut self` method",
    );
}

#[test]
fn consuming_method_is_rejected_through_a_ref_param() {
    assert_err(
        r#"
struct Dog { name String }
impl Dog {
    destroy(: self) { println(self.name); }
}
fn show(d: &Dog) { d.destroy(); }
fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    show(dog);
}
"#,
        "cannot call a consuming (`: self`) method through a borrowed reference",
    );
}

#[test]
fn consuming_method_is_rejected_through_a_mut_ref_param() {
    assert_err(
        r#"
struct Dog { name String }
impl Dog {
    destroy(: self) { println(self.name); }
}
fn show(d: &mut Dog) { d.destroy(); }
fn main() {
    let mut dog: Dog = :Dog { name = "Rex" };
    show(dog);
}
"#,
        "cannot call a consuming (`: self`) method through a borrowed reference",
    );
}

#[test]
fn mut_borrowing_method_remains_callable_directly_on_a_bare_owned_local() {
    // Confirms the widened `ByMutRef | OwnedMutRef` mutability check
    // didn't regress the already-working non-reference path.
    assert_ok(
        r#"
struct Dog { name String }
impl Dog {
    rename_ref(: &mut self, new_name String) { self.name = new_name; }
}
fn main() {
    let mut dog: Dog = :Dog { name = "Rex" };
    dog.rename_ref("Buddy");
}
"#,
    );
}
