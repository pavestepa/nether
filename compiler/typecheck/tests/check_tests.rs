use nether_diagnostics::{Diagnostic, SourceMap};
use nether_typecheck::check;

#[path = "check_tests/borrows.rs"]
mod borrows;
#[path = "check_tests/casing.rs"]
mod casing;
#[path = "check_tests/expressions.rs"]
mod expressions;
#[path = "check_tests/generics.rs"]
mod generics;
#[path = "check_tests/moves.rs"]
mod moves;
#[path = "check_tests/self_overloads.rs"]
mod self_overloads;
#[path = "check_tests/to_conversion.rs"]
mod to_conversion;
#[path = "check_tests/variadics_and_layout.rs"]
mod variadics_and_layout;

/// `Option`/`Result`/`Array` are ordinary prelude declarations now
/// (`stdlib/option.nr`/`result.nt`/`array.nt`), not compiler builtins —
/// `check_source` resolves a bare, self-contained `Module` directly
/// (`nether_resolver::resolve`, no driver, no prelude loading), so every
/// test source is prepended with a stand-in declaration to keep every
/// existing test's use of bare `Option`/`Result`/`Array` working
/// unchanged. Genuinely testing the bundled prelude itself belongs in
/// `nether_driver`'s own integration tests (e.g.
/// `bundled_option_and_result_stdlib_runs_end_to_end`).
const PRELUDE_STUB: &str = r#"
enum Option<T> {
    Some(T),
    None,
}
enum Result<T, E> {
    Ok(T),
    Error(E),
}
struct Array<T>;
"#;

fn check_source(source: &str) -> Vec<Diagnostic> {
    let source = format!("{PRELUDE_STUB}\n{source}");
    let source = source.as_str();
    let mut map = SourceMap::new();
    let file = map.add_file("test.nr", source);
    let (module, parse_diags) = nether_parser::parse_module(source, file);
    assert!(
        parse_diags.is_empty(),
        "unexpected parse diagnostics: {parse_diags:?}"
    );
    let (resolved, resolve_diags) = nether_resolver::resolve(&module);
    assert!(
        resolve_diags.is_empty(),
        "unexpected resolve diagnostics: {resolve_diags:?}"
    );
    let (_tables, check_diags) = check(&module, &resolved);
    check_diags
}

fn assert_ok(source: &str) {
    let diags = check_source(source);
    assert!(
        diags.is_empty(),
        "unexpected typecheck diagnostics: {}",
        messages(&diags)
    );
}

fn assert_err(source: &str, needle: &str) {
    let diags = check_source(source);
    assert!(
        diags.iter().any(|d| d.message.contains(needle)),
        "expected a diagnostic containing {needle:?}, got: {}",
        messages(&diags)
    );
}

fn messages(diags: &[Diagnostic]) -> String {
    diags
        .iter()
        .map(|d| d.message.clone())
        .collect::<Vec<_>>()
        .join("; ")
}

#[test]
fn canonical_spec_example_type_checks_cleanly() {
    assert_ok(
        r#"
use lang.Lang;

fn main() {
    let mut a = Lang.new("Bobby");
    a.set_name("Husky");
    println(a.into_string());
}

struct Lang {
    name String
}

impl Lang {
    new(name String) Lang {
        return Lang { name };
    }

    set_name(mut self, new_name String) {
        self.name = new_name;
    }
}

impl Lang Into<String> {
    into_string(self) String {
        return `name: ${self.name}`;
    }
}

trait Sound {
    sound() String {
        return "...";
    }
}

impl Lang Sound {
    sound() String {
        return "Woof! Ruff!";
    }
}
"#,
    );
}

#[test]
fn let_infers_type_from_initializer() {
    assert_ok("fn main() { let x = 5; let y i32 = x; }");
}

#[test]
fn let_annotation_mismatch_reports_diagnostic() {
    assert_err(
        "fn main() { let x bool = 5; }",
        "expected `bool`, found `i32`",
    );
}

#[test]
fn mut_param_requires_mut_at_call_site() {
    assert_err(
        r#"
fn inc(n mut i32) { n = n + 1; }
fn main() {
    let x = 4;
    inc(x);
}
"#,
        "the caller must also write `mut`",
    );
}

#[test]
fn mut_arg_on_non_mut_param_is_rejected() {
    assert_err(
        r#"
fn show(n i32) { println(n); }
fn main() {
    let mut x = 4;
    show(mut x);
}
"#,
        "only valid for arguments passed to a `mut` parameter",
    );
}

#[test]
fn mut_call_site_succeeds_with_mut_binding() {
    assert_ok(
        r#"
fn inc(n mut i32) { n = n + 1; }
fn main() {
    let mut x = 4;
    inc(mut x);
}
"#,
    );
}

#[test]
fn mut_call_site_rejects_immutable_binding() {
    assert_err(
        r#"
fn inc(n mut i32) { n = n + 1; }
fn main() {
    let x = 4;
    inc(mut x);
}
"#,
        "declare it with `let mut`",
    );
}

#[test]
fn static_and_instance_method_calls_type_check() {
    assert_ok(
        r#"
struct Dog { name String }
impl Dog {
    new(name String) Dog {
        return Dog { name };
    }
    greet(self) String {
        return self.name;
    }
}
fn main() {
    let d = Dog.new("Rex");
    let g = d.greet();
}
"#,
    );
}

#[test]
fn calling_instance_method_as_static_is_rejected() {
    assert_err(
        r#"
struct Dog { name String }
impl Dog {
    greet(self) String { return self.name; }
}
fn main() {
    let g = Dog.greet();
}
"#,
        "instance method",
    );
}

#[test]
fn struct_literal_checks_missing_and_unknown_fields() {
    assert_err(
        r#"
struct Dog { name String, age i32 }
fn main() {
    let d = Dog { name = "Rex" };
}
"#,
        "missing field `age`",
    );
    assert_err(
        r#"
struct Dog { name String }
fn main() {
    let d = Dog { name = "Rex", nickname = "Rexy" };
}
"#,
        "has no field named `nickname`",
    );
}

#[test]
fn struct_literal_field_type_mismatch_reported() {
    assert_err(
        r#"
struct Dog { name String }
fn main() {
    let d = Dog { name = 5 };
}
"#,
        "found `i32`",
    );
}

#[test]
fn tuple_struct_construction_checks_arity_and_types() {
    assert_ok("struct Point(i32, i32);\nfn main() { let p = Point(1, 2); }");
    assert_err(
        "struct Point(i32, i32);\nfn main() { let p = Point(1); }",
        "expected 2 argument(s), found 1",
    );
    assert_err(
        "struct Point(i32, i32);\nfn main() { let p = Point(1, \"x\"); }",
        "found `String`",
    );
}

#[test]
fn enum_match_exhaustiveness_is_checked() {
    assert_err(
        r#"
enum Color { Red, Green, Blue }
fn f(c Color) {
    match c {
        Color.Red => 1,
        Color.Green => 2,
    };
}
"#,
        "not exhaustive",
    );
    assert_ok(
        r#"
enum Color { Red, Green, Blue }
fn f(c Color) {
    match c {
        Color.Red => 1,
        Color.Green => 2,
        Color.Blue => 3,
    };
}
"#,
    );
    assert_ok(
        r#"
enum Color { Red, Green, Blue }
fn f(c Color) {
    match c {
        Color.Red => 1,
        _ => 0,
    };
}
"#,
    );
    assert_err(
        r#"
enum Maybe { Some(i32), None }
fn f(value Maybe) i32 {
    match value {
        Maybe.Some(x) => x,
    }
}
"#,
        "missing variant(s) None",
    );
}

#[test]
fn enum_variant_payload_checked() {
    assert_ok(
        r#"
enum Shape { Circle(i32), Empty }
fn f() {
    let s = Shape.Circle(4);
}
"#,
    );
    assert_err(
        r#"
enum Shape { Circle(i32), Empty }
fn f() {
    let s = Shape.Circle("x");
}
"#,
        "found `String`",
    );
}

#[test]
fn invalid_patterns_are_rejected_before_mir_lowering() {
    assert_err(
        r#"
fn f(value bool) i32 {
    match value {
        1 => 1,
        _ => 0,
    }
}
"#,
        "literal pattern is incompatible",
    );
    assert_err(
        r#"
fn f(value i32) i32 {
    match value {
        (x, y) => x,
        _ => 0,
    }
}
"#,
        "tuple pattern requires a tuple",
    );
    assert_err(
        r#"
fn f(value (i32, bool)) i32 {
    match value {
        (x,) => x,
        _ => 0,
    }
}
"#,
        "tuple pattern has 1 element(s)",
    );
    assert_err(
        r#"
enum Left { Item(i32) }
enum Right { Item2(i32) }
fn f(value Left) i32 {
    match value {
        Right.Item2(x) => x,
        _ => 0,
    }
}
"#,
        "variant pattern from enum `Right` cannot match `Left`",
    );
    assert_err(
        r#"
enum Maybe { Some(i32), None }
fn f(value Maybe) i32 {
    match value {
        Maybe.Some => 1,
        _ => 0,
    }
}
"#,
        "variant pattern expects 1 payload field(s), found 0",
    );
}

#[test]
fn generic_bound_satisfied_and_unsatisfied() {
    assert_ok(
        r#"
trait Sound { sound() String { return "..."; } }
struct Dog { name String }
impl Dog Sound { sound() String { return "Woof"; } }
fn make_noise<T: Sound>(x T) String {
    return x.sound();
}
fn main() {
    let d = Dog { name = "Rex" };
    make_noise(d);
}
"#,
    );
    assert_err(
        r#"
trait Sound { sound() String { return "..."; } }
struct Rock { weight i32 }
fn make_noise<T: Sound>(x T) String {
    return x.sound();
}
fn main() {
    let r = Rock { weight = 1 };
    make_noise(r);
}
"#,
        "does not implement `Sound`",
    );
}

#[test]
fn option_unit_variant_uses_context_and_never_leaks_an_unknown_type() {
    assert_ok(
        r#"
fn none() Option<i32> {
    return Option.None;
}
fn main() {
    let direct Option<i32> = Option.None;
    let conditional = if true { Option.None } else { Option.Some(1) };
}
"#,
    );
    assert_err(
        "fn main() { let unknown = Option.None; }",
        "cannot infer all generic type arguments",
    );
}

#[test]
fn generic_calls_require_every_parameter_to_be_inferred() {
    assert_err(
        r#"
fn opaque<T>(value i32) i32 { return value; }
fn main() { opaque(1); }
"#,
        "cannot infer generic parameter `T`",
    );
    assert_err(
        r#"
fn identity<T>(value T) T { return value; }
fn main() { let f = identity; }
"#,
        "generic function `identity` cannot be used as a value",
    );
}
