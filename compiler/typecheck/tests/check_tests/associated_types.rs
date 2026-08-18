use super::*;

#[test]
fn generic_associated_type_projection_type_checks() {
    assert_ok(
        r#"
trait Container { pub type Item; }
struct Box;
impl Box Container { pub type Item = i32; }
fn identity<T Container>(value T.Item) T.Item { return value; }
fn main() { let value i32 = identity<Box>(5); }
"#,
    );
}

#[test]
fn concrete_projection_and_inherited_requirement_type_check() {
    assert_ok(
        r#"
trait Parent { type Item; }
trait Child Parent {}
struct Box;
impl Box Child { type Item = i32; }
fn identity<T Child>(value T.Item) T.Item { return value; }
fn main() {
    let direct Box.Item = 4;
    let generic i32 = identity<Box>(direct);
}
"#,
    );
}

#[test]
fn associated_type_default_is_used() {
    assert_ok(
        r#"
trait Container { type Item = i32; }
struct Box;
impl Box Container {}
fn identity<T Container>(value T.Item) T.Item { return value; }
fn main() { let value i32 = identity<Box>(5); }
"#,
    );
}

#[test]
fn missing_associated_type_is_rejected() {
    assert_err(
        r#"
trait Container { type Item; }
struct Box;
impl Box Container {}
fn main() {}
"#,
        "missing associated type `Item`",
    );
}

#[test]
fn projection_requires_a_declaring_bound() {
    assert_err(
        r#"
fn invalid<T>(value T.Item) T.Item { return value; }
fn main() {}
"#,
        "has no associated type `Item` in its bounds",
    );
}
