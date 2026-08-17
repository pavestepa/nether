use super::*;

/// Language-spec §8.4: a method name may carry one ARC-domain body
/// (`self`/`mut self`) and one owned-domain body (`: self`/`: &self`/
/// `: &mut self`) on the same type, resolved by the receiver's actual
/// ownership domain at each call site — previously a hard "defined more
/// than once" error (Stage 2's `ReceiverDomain` fix).

#[test]
fn arc_and_owned_domain_overloads_of_one_method_name_coexist() {
    assert_ok(
        r#"
struct Dog { name String }
impl Dog {
    greet(self) String { return self.name; }
    greet(: self) String { return self.name; }
}
fn main() {
    let d = Dog { name = "Rex" };
    let g String = d.greet();

    let owned: Dog = :Dog { name = "Buddy" };
    let g2 String = owned.greet();
}
"#,
    );
}

#[test]
fn owned_ref_receiver_overload_resolves_and_is_callable() {
    // `:&T`/`:&mut T` have no expression-level construction syntax yet
    // (language-spec §3.1 — reference codegen/return position is Stage 2's
    // still-deferred remainder), so this only exercises an `: &self`
    // receiver with an ordinary return type — the domain-overload
    // resolution itself, not reference-returning.
    assert_ok(
        r#"
struct Dog { name String }
impl Dog {
    get_name(: &self) String { return self.name; }
}
fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    let n String = dog.get_name();
}
"#,
    );
}

#[test]
fn same_domain_redefinition_is_still_a_duplicate_error() {
    assert_err(
        r#"
struct Dog { name String }
impl Dog {
    greet(self) String { return self.name; }
    greet(self) String { return self.name; }
}
"#,
        "is defined more than once",
    );
}

#[test]
fn static_and_instance_method_of_the_same_name_still_conflict() {
    assert_err(
        r#"
struct Dog { name String }
impl Dog {
    greet() String { return "static"; }
    greet(self) String { return self.name; }
}
"#,
        "is defined more than once",
    );
}

#[test]
fn trait_default_is_satisfied_only_by_a_matching_domain_implementation() {
    // The trait requires an ARC-domain (`self`) `sound` — an owned-domain
    // (`: self`) implementation doesn't satisfy it, so the ARC default
    // still applies and the type remains free to *also* add its own
    // owned-domain overload independently.
    assert_ok(
        r#"
trait Sound { sound(self) String { return "..."; } }
struct Dog Sound {
    name String
}
impl Dog {
    sound(: self) String { return self.name; }
}
fn main() {
    let d = Dog { name = "Rex" };
    let s String = d.sound();

    let owned: Dog = :Dog { name = "Buddy" };
    let s2 String = owned.sound();
}
"#,
    );
}
