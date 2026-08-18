use super::*;

#[test]
fn any_trait_packages_a_conforming_value_and_exposes_safe_methods() {
    assert_ok(
        r#"
trait Sound { sound(self) String; }
struct Dog;
impl Dog Sound { sound(self) String { return "woof"; } }
fn erase(value Dog) any Sound { return value; }
fn main() {
    let value any Sound = erase(Dog);
    let text String = value.sound();
}
"#,
    );
}

#[test]
fn some_trait_requires_one_hidden_concrete_type() {
    assert_ok(
        r#"
trait Sound { sound(self) String; }
struct Dog;
impl Dog Sound { sound(self) String { return "woof"; } }
fn make() some Sound { return Dog; }
fn main() { let text String = make().sound(); }
"#,
    );
    assert_err(
        r#"
trait Sound { sound(self) String; }
struct Dog;
struct Cat;
impl Dog Sound { sound(self) String { return "woof"; } }
impl Cat Sound { sound(self) String { return "meow"; } }
fn make(flag bool) some Sound {
    if flag { return Dog; }
    return Cat;
}
fn main() {}
"#,
        "all returns of an opaque `some` type must use one concrete type",
    );
}

#[test]
fn existential_safety_rejects_static_and_generic_methods() {
    assert_err(
        r#"
trait Unsafe {
    make() i32;
    map<T>(self, value T) T;
}
fn accept(value any Unsafe) {}
fn main() {}
"#,
        "not existential-safe",
    );
    assert_err(
        r#"
trait Consuming { take(: self) i32; }
fn accept(value any Consuming) {}
fn main() {}
"#,
        "owned-domain receivers cannot be stored in a reusable witness package",
    );
}

#[test]
fn existential_requires_trait_conformance() {
    assert_err(
        r#"
trait Sound { sound(self) String; }
struct Rock;
fn main() { let value any Sound = Rock; }
"#,
        "does not implement `Sound`",
    );
}
