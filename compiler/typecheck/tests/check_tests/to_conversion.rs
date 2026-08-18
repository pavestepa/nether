use super::*;

/// Stage 2, slice 4: `to(value)`, the universal ownership-domain
/// conversion (language-spec §9), with inferred or explicit target. Three
/// of the four transitions are sound pure relabeling
/// (`:T -> T`, `:t -> t`, `t -> :t`); `T -> :T` (heap) stays rejected
/// until `Clone` exists (spec §10, Stage 3).

#[test]
fn owned_to_arc_conversion_consumes_the_source() {
    assert_err(
        r#"
struct Dog { name String }
fn main() {
    let d: Dog = :Dog { name = "Rex" };
    let arc_d Dog = to(d);
    println(arc_d.name);
    println(d.name);
}
"#,
        "use of a value after it was moved",
    );
}

#[test]
fn owned_to_arc_conversion_type_checks_and_runs() {
    assert_ok(
        r#"
struct Dog { name String }
fn main() {
    let d: Dog = :Dog { name = "Rex" };
    let arc_d Dog = to(d);
    println(arc_d.name);
}
"#,
    );
}

#[test]
fn owned_inline_to_ordinary_inline_conversion_works() {
    assert_ok(
        r#"
fn main() {
    let n: i32 = 10;
    let m i32 = to(n);
    println(`${m}`);
}
"#,
    );
}

#[test]
fn ordinary_inline_to_owned_inline_conversion_does_not_consume_the_source() {
    assert_ok(
        r#"
fn main() {
    let n = 10;
    let owned: i32 = to(n);
    println(`${n}`);
    println(`${owned}`);
}
"#,
    );
}

#[test]
fn ordinary_heap_to_owned_heap_conversion_is_rejected() {
    assert_err(
        r#"
struct Dog { name String }
fn main() {
    let d = Dog { name = "Rex" };
    let owned: Dog = to(d);
    println(owned.name);
}
"#,
        "requires `Clone`",
    );
}

#[test]
fn to_with_no_inferrable_target_is_rejected() {
    assert_err(
        r#"
struct Dog { name String }
fn main() {
    let d: Dog = :Dog { name = "Rex" };
    println(to(d).name);
}
"#,
        "cannot infer the target type",
    );
}

#[test]
fn to_between_two_arc_values_is_rejected() {
    assert_err(
        r#"
struct Dog { name String }
fn main() {
    let d = Dog { name = "Rex" };
    let d2 Dog = to(d);
    println(d2.name);
}
"#,
        "must convert between the owned and ordinary form",
    );
}

#[test]
fn to_between_two_owned_values_is_rejected() {
    assert_err(
        r#"
struct Dog { name String }
fn main() {
    let d: Dog = :Dog { name = "Rex" };
    let d2: Dog = to(d);
    println(d2.name);
}
"#,
        "must convert between the owned and ordinary form",
    );
}

#[test]
fn explicit_generic_to_uses_its_target_without_context() {
    assert_ok(
        r#"
struct Dog { name String }
fn main() {
    let d: Dog = :Dog { name = "Rex" };
    let d2 = to<Dog>(d);
    println(d2.name);
}
"#,
    );
}

#[test]
fn explicit_owned_inline_target_is_supported() {
    assert_ok(
        r#"
fn main() {
    let n = 4;
    let owned = to<:i32>(n);
    let plain i32 = to<i32>(owned);
    println(`${plain}`);
}
"#,
    );
}

#[test]
fn explicit_to_target_must_match_context() {
    assert_err(
        r#"
fn main() {
    let n = 4;
    let bad bool = to<:i32>(n);
    println(`${bad}`);
}
"#,
        "does not match expected type",
    );
}

#[test]
fn to_rejects_more_than_one_explicit_target() {
    assert_err(
        r#"
fn main() {
    let n = 4;
    let bad = to<i32, bool>(n);
    println(`${bad}`);
}
"#,
        "takes exactly 1 type argument",
    );
}
