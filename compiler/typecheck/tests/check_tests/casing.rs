//! language-spec §5: a type alias's casing must match its *resolved*
//! representation category, with an enum/tuple exemption.

use super::*;

#[test]
fn pascal_case_alias_for_a_heap_struct_is_accepted() {
    assert_ok(
        r#"
struct Dog {
    name String
}
type Pet = Dog;

fn main() {
}
"#,
    );
}

#[test]
fn lowercase_alias_for_an_inline_struct_is_accepted() {
    assert_ok(
        r#"
struct point {
    x f32,
    y f32
}
type coord = point;

fn main() {
}
"#,
    );
}

#[test]
fn lowercase_alias_for_a_primitive_tuple_is_accepted() {
    assert_ok(
        r#"
type color = (u32, u32, u32);

fn main() {
}
"#,
    );
}

#[test]
fn pascal_case_alias_lying_about_an_inline_struct_is_rejected() {
    assert_err(
        r#"
struct point {
    x f32,
    y f32
}
type Coord = point;

fn main() {
}
"#,
        "is PascalCase but resolves to inline/value type",
    );
}

#[test]
fn lowercase_alias_lying_about_a_heap_struct_is_rejected() {
    assert_err(
        r#"
struct Dog {
    name String
}
type pet = Dog;

fn main() {
}
"#,
        "is lowercase but resolves to heap/reference type",
    );
}

#[test]
fn allow_pascal_case_suppresses_only_the_alias_casing_diagnostic() {
    assert_ok(
        r#"
struct point {
    x f32,
    y f32
}
#[allow_pascal_case]
type Coord = point;

fn main() {
}
"#,
    );

    assert_ok(
        r#"
struct Dog {
    name String
}
#[allow_pascal_case]
type pet = Dog;

fn main() {
}
"#,
    );
}

#[test]
fn enum_aliases_are_exempt_from_casing_validation_even_when_lowercase() {
    // `Option`/`Result`-shaped enums stay PascalCase-by-convention and
    // inline regardless of an alias's own casing (the same exemption
    // `alloc_kind` already carries for enum/tuple declarations
    // themselves) — a lowercase alias for a PascalCase enum must not be
    // flagged.
    assert_ok(
        r#"
enum choice {
    Yes,
    No,
}
type pick = choice;

fn main() {
}
"#,
    );
}
