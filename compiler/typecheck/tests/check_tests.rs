use nether_diagnostics::{Diagnostic, SourceMap};
use nether_typecheck::check;

fn check_source(source: &str) -> Vec<Diagnostic> {
    let mut map = SourceMap::new();
    let file = map.add_file("test.nr", source);
    let (module, parse_diags) = nether_parser::parse_module(source, file);
    assert!(parse_diags.is_empty(), "unexpected parse diagnostics: {parse_diags:?}");
    let (resolved, resolve_diags) = nether_resolver::resolve(&module);
    assert!(resolve_diags.is_empty(), "unexpected resolve diagnostics: {resolve_diags:?}");
    let (_tables, check_diags) = check(&module, &resolved);
    check_diags
}

fn assert_ok(source: &str) {
    let diags = check_source(source);
    assert!(diags.is_empty(), "unexpected typecheck diagnostics: {}", messages(&diags));
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
    diags.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("; ")
}

#[test]
fn canonical_spec_example_type_checks_cleanly() {
    assert_ok(
        r#"
use lang.Lang;

fn main() {
    let a = Lang.new("Bobby");
    a.set_name("Husky");
    println(a.into_string());
}

type Lang {
    name: String
}

impl Lang {
    new(name: String): Lang {
        Lang { name }
    }

    set_name(mut self, new_name: String) {
        self.name = new_name;
    }
}

impl Lang: Into<String> {
    into_string(self): String {
        `name: ${self.name}`
    }
}

interface Sound {
    sound(): String {
        "..."
    }
}

impl Lang: Sound {
    sound(): String {
        "Woof! Ruff!"
    }
}
"#,
    );
}

#[test]
fn let_infers_type_from_initializer() {
    assert_ok("fn main() { let x = 5; let y: i32 = x; }");
}

#[test]
fn let_annotation_mismatch_reports_diagnostic() {
    assert_err("fn main() { let x: bool = 5; }", "expected `bool`, found `i32`");
}

#[test]
fn mut_param_requires_mut_at_call_site() {
    assert_err(
        r#"
fn inc(mut n: i32) { n = n + 1; }
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
fn show(n: i32) { println(n); }
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
fn inc(mut n: i32) { n = n + 1; }
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
fn inc(mut n: i32) { n = n + 1; }
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
type Dog { name: String }
impl Dog {
    new(name: String): Dog {
        Dog { name }
    }
    greet(self): String {
        self.name
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
type Dog { name: String }
impl Dog {
    greet(self): String { self.name }
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
type Dog { name: String, age: i32 }
fn main() {
    let d = Dog { name: "Rex" };
}
"#,
        "missing field `age`",
    );
    assert_err(
        r#"
type Dog { name: String }
fn main() {
    let d = Dog { name: "Rex", nickname: "Rexy" };
}
"#,
        "has no field named `nickname`",
    );
}

#[test]
fn struct_literal_field_type_mismatch_reported() {
    assert_err(
        r#"
type Dog { name: String }
fn main() {
    let d = Dog { name: 5 };
}
"#,
        "found `i32`",
    );
}

#[test]
fn tuple_struct_construction_checks_arity_and_types() {
    assert_ok("type Point(i32, i32);\nfn main() { let p = Point(1, 2); }");
    assert_err("type Point(i32, i32);\nfn main() { let p = Point(1); }", "expected 2 argument(s), found 1");
    assert_err("type Point(i32, i32);\nfn main() { let p = Point(1, \"x\"); }", "found `String`");
}

#[test]
fn enum_match_exhaustiveness_is_checked() {
    assert_err(
        r#"
enum Color { Red, Green, Blue }
fn f(c: Color) {
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
fn f(c: Color) {
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
fn f(c: Color) {
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
fn f(value: Maybe): i32 {
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
fn f(value: bool): i32 {
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
fn f(value: i32): i32 {
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
fn f(value: (i32, bool)): i32 {
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
fn f(value: Left): i32 {
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
fn f(value: Maybe): i32 {
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
interface Sound { sound(): String { "..." } }
type Dog { name: String }
impl Dog: Sound { sound(): String { "Woof" } }
fn make_noise<T: Sound>(x: T): String {
    x.sound()
}
fn main() {
    let d = Dog { name: "Rex" };
    make_noise(d);
}
"#,
    );
    assert_err(
        r#"
interface Sound { sound(): String { "..." } }
type Rock { weight: i32 }
fn make_noise<T: Sound>(x: T): String {
    x.sound()
}
fn main() {
    let r = Rock { weight: 1 };
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
fn none(): Option<i32> {
    Option.None
}
fn main() {
    let direct: Option<i32> = Option.None;
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
fn opaque<T>(value: i32): i32 { value }
fn main() { opaque(1); }
"#,
        "cannot infer generic parameter `T`",
    );
    assert_err(
        r#"
fn identity<T>(value: T): T { value }
fn main() { let f = identity; }
"#,
        "generic function `identity` cannot be used as a value",
    );
}

#[test]
fn into_string_bound_is_checked_inside_generic_bodies() {
    assert_ok(
        r#"
fn stringify<T: Into<String>>(value: T): String {
    value.into_string()
}
fn main() {
    let number = stringify(42);
    let text = stringify("ready");
}
"#,
    );
    assert_err(
        r#"
fn invalid<T>(value: T) {
    println(value);
}
"#,
        "cannot be converted to `String`",
    );
}

#[test]
fn generic_interfaces_preserve_arguments_and_specialize_methods() {
    assert_ok(
        r#"
interface Convert<T> {
    convert(self): T;
}
interface Identity<T> {
    identity(self, value: T): T { value }
}
type Dog { name: String }
impl Dog: Convert<String> {
    convert(self): String { self.name }
}
impl Dog: Identity<String> {}
fn convert<U: Convert<String>>(value: U): String {
    value.convert()
}
fn identify<U: Identity<String>>(value: U): String {
    value.identity("ready")
}
fn main() {
    let dog = Dog { name: "Bobby" };
    let name: String = convert(dog);
    let state: String = identify(dog);
}
"#,
    );
    assert_err(
        r#"
interface Convert<T> { convert(self): T; }
type Dog;
impl Dog: Convert<i32> { convert(self): i32 { 1 } }
fn convert<U: Convert<String>>(value: U): String { value.convert() }
fn main() { convert(Dog); }
"#,
        "does not implement `Convert<String>`",
    );
    assert_err(
        r#"
interface Convert<T> { convert(self): T; }
type Dog;
impl Dog: Convert<String> { convert(self): i32 { 1 } }
"#,
        "does not match its declaration",
    );
    assert_ok(
        r#"
interface Read<T> { read(self): T; }
type Boxed<T> { value: T }
impl Boxed: Read<T> { read(self): T { self.value } }
fn read_text<U: Read<String>>(value: U): String { value.read() }
fn main() {
    let text = Boxed { value: "ready" };
    println(read_text(text));
}
"#,
    );
}

#[test]
fn generic_type_bounds_are_enforced_after_inference() {
    assert_ok(
        r#"
interface Sound { sound(self): String; }
type Dog;
impl Dog: Sound { sound(self): String { "woof" } }
type Cage<T: Sound> { value: T }
fn open<T: Sound>(cage: Cage<T>): String { cage.value.sound() }
fn main() {
    let cage = Cage { value: Dog };
    println(open(cage));
}
"#,
    );
    assert_err(
        r#"
interface Sound { sound(self): String; }
type Rock;
type Cage<T: Sound> { value: T }
fn main() { let cage = Cage { value: Rock }; }
"#,
        "required by this generic type",
    );
}

#[test]
fn infinitely_recursive_value_layouts_are_diagnostics_not_codegen_ices() {
    assert_err(
        "type node { next: node }\nfn main() {}",
        "infinitely recursive layout",
    );
    assert_err(
        "enum List { End, Next(List) }\nfn main() {}",
        "infinitely recursive layout",
    );
    assert_err(
        "type left { right: right }\ntype right { left: left }\nfn main() {}",
        "infinitely recursive layout",
    );
    assert_ok(
        r#"
type Node { next: Option<Node> }
type wrapper { node: Node }
fn main() {
    let node = Node { next: Option.None };
    let value = wrapper { node };
}
"#,
    );
}

#[test]
fn invalid_control_flow_and_entry_signatures_stop_before_mir() {
    assert_err(
        "fn main() { break; }",
        "`break` is only valid inside a loop",
    );
    assert_err(
        "fn main() { continue; }",
        "`continue` is only valid inside a loop",
    );
    assert_err(
        "fn main(value: i32) {}",
        "`main` must have signature `fn main()`",
    );
    assert_err(
        "interface Sound { sound(self): String; }\nfn use_it(value: Sound) {}\nfn main() {}",
        "cannot be used as a value type",
    );
    assert_ok(
        "fn main() { loop { break; } while false { continue; } }",
    );
}

#[test]
fn generic_structs_and_tuple_structs_infer_and_substitute_fields() {
    assert_ok(
        r#"
type Boxed<T> { value: T }
type pair<T, U>(T, U);
impl Boxed {
    new(value: T): Boxed<T> { Boxed { value } }
    get(self): T { self.value }
    replace<U>(self, value: U): Boxed<U> { Boxed { value } }
}
fn read_number(value: Boxed<i32>): i32 { value.value }
fn main() {
    let number = Boxed { value: 42 };
    let text = Boxed { value: "ready" };
    let both = pair(number, text);
    let n: i32 = both.0.value;
    let s: String = both.1.value;
    let built = Boxed.new(7);
    let built_value: i32 = built.get();
    let replaced: Boxed<String> = built.replace("new");
    let replaced_value: String = replaced.get();
}
"#,
    );
    assert_err(
        r#"
type Boxed<T> { value: T }
fn main() {
    let value: Boxed<i32> = Boxed { value: "wrong" };
}
"#,
        "expected `i32`, found `String`",
    );
}

#[test]
fn impl_missing_required_interface_method_reports_diagnostic() {
    assert_err(
        r#"
interface Sound {
    sound(): String;
}
type Dog { name: String }
impl Dog: Sound {
}
"#,
        "does not implement required method",
    );
}

#[test]
fn weak_must_wrap_a_heap_type() {
    assert_ok("type Node { next: weak Node }");
    assert_err("type point { x: i32 }\ntype Node { p: weak point }", "can only wrap a heap-allocated type");
}

#[test]
fn a_heap_value_coerces_into_a_weak_field_without_an_explicit_conversion() {
    assert_ok(
        r#"
type Child { name: String }
type Parent { kid: weak Child }
fn main() {
    let c = Child { name: "Rex" };
    let p = Parent { kid: c };
    let mut w: weak Child = c;
    w = c;
}
"#,
    );
    assert_err(
        r#"
type Child { name: String }
type Parent { kid: weak Child }
fn main() {
    let p = Parent { kid: 5 };
}
"#,
        "expected `weak Child`, found `i32`",
    );
}

#[test]
fn reading_a_weak_field_produces_an_option_not_a_bare_weak_value() {
    assert_ok(
        r#"
type Node { next: weak Node }
fn describe(n: Node): String {
    match n.next {
        Some(_) => "has next",
        None => "no next",
    }
}
fn main() {}
"#,
    );
    assert_err(
        r#"
type Node { name: String, next: weak Node }
fn describe(n: Node): String {
    n.next.name
}
fn main() {}
"#,
        "has no field named `name`",
    );
}

#[test]
fn arithmetic_requires_matching_numeric_operands() {
    assert_ok("fn main() { let x = 1 + 2; }");
    assert_err("fn main() { let x = 1 + true; }", "must have the same type");
    assert_err("fn main() { let x = true + false; }", "require numeric operands");
}

#[test]
fn if_else_branch_type_mismatch_reported() {
    assert_err(
        r#"
fn f(): i32 {
    if true {
        1
    } else {
        "x"
    }
}
"#,
        "incompatible types",
    );
}

#[test]
fn assigning_to_immutable_binding_is_rejected() {
    assert_err("fn main() { let x = 1; x = 2; }", "declare it with `let mut`");
    assert_ok("fn main() { let mut x = 1; x = 2; }");
}

#[test]
fn return_type_mismatch_is_reported() {
    assert_err("fn f(): i32 { return \"x\"; }", "expected return type");
}

#[test]
fn array_and_index_type_checks() {
    assert_ok("fn main() { let a = [1, 2, 3]; let x = a[0]; }");
    assert_err("fn main() { let a: [i32] = []; let x = a[\"no\"]; }", "must be an integer type");
    assert_err("fn main() { let a = []; }", "cannot infer");
}

#[test]
fn array_builtin_methods_type_check() {
    assert_ok("fn main() { let mut a = [1, 2]; a.push(3); let n: usize = a.len(); let p = a.pop(); }");
    assert_err("fn main() { let mut a = [1, 2]; a.push(\"x\"); }", "found `String`");
}
