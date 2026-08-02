use nether_diagnostics::{Diagnostic, SourceMap};
use nether_typecheck::check;

/// `Option`/`Result`/`Array` are ordinary prelude declarations now
/// (`stdlib/option.nt`/`result.nt`/`array.nt`), not compiler builtins —
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
type Array<T>;
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
    assert_err(
        "fn main() { let x: bool = 5; }",
        "expected `bool`, found `i32`",
    );
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
    assert_err(
        "type Point(i32, i32);\nfn main() { let p = Point(1); }",
        "expected 2 argument(s), found 1",
    );
    assert_err(
        "type Point(i32, i32);\nfn main() { let p = Point(1, \"x\"); }",
        "found `String`",
    );
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
fn explicit_generic_call_arguments_control_specialization() {
    assert_ok(
        r#"
fn opaque<T>(value: i32): i32 { value }
fn pair<T, U>(left: T, right: U): (T, U) { (left, right) }

type Boxed<T> { value: T }
impl Boxed {
    replace<U>(self, value: U): Boxed<U> { Boxed { value } }
}

fn main() {
    let value: i32 = opaque<String>(2);
    let both: (u32, String) = pair<u32, String>(2, "ready");
    let number = Boxed { value: 1 };
    let text: Boxed<String> = number.replace<String>("one");
}
"#,
    );
    assert_ok(
        r#"
interface Transform {
    transform<U>(self, value: U): U;
}
type Boxed<T> { value: T }
impl Boxed: Transform {
    transform<U>(self, value: U): U { value }
}
fn apply<T: Transform>(value: T): String {
    value.transform<String>("ready")
}
fn main() {
    let result: String = apply(Boxed { value: 1 });
}
"#,
    );
    assert_err(
        r#"
fn identity<T>(value: T): T { value }
fn main() { identity<u32, String>(2); }
"#,
        "expected 1 explicit generic argument(s), found 2",
    );
    assert_err(
        r#"
fn identity<T>(value: T): T { value }
fn main() { identity<String>(2); }
"#,
        "expected `String`, found `i32`",
    );
    assert_err(
        r#"
interface Sound { sound(self): String; }
fn make_noise<T: Sound>(value: T): String { value.sound() }
fn main() { make_noise<u32>(2); }
"#,
        "does not implement `Sound`",
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
type Dog: Identity<String> { name: String }
impl Dog: Convert<String> {
    convert(self): String { self.name }
}
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
fn static_methods_can_be_called_through_values() {
    assert_ok(
        r#"
type Animal { name: String }
impl Animal {
    new(name: String): Animal { Animal { name } }
    static_method(): String { "A" }
}
fn main() {
    let animal = Animal.new("Cat");
    let value: String = animal.static_method();
}
"#,
    );
    assert_ok(
        r#"
interface Static { value(): String; }
type Animal;
impl Animal: Static {
    value(): String { "A" }
}
fn read<T: Static>(animal: T): String {
    animal.value()
}
fn main() {
    let value: String = read(Animal);
}
"#,
    );
}

#[test]
fn interface_implementations_can_be_mixed_across_blocks() {
    assert_ok(
        r#"
interface B { b(self): String; }
interface C { c(self): String; }
type A;
impl A: B, C {}
impl A { b(self): String { "b" } }
impl A { c(self): String { "c" } }
fn use_b<T: B>(value: T): String { value.b() }
fn use_c<T: C>(value: T): String { value.c() }
fn main() {
    use_b(A);
    use_c(A);
}
"#,
    );
    assert_err(
        r#"
interface Sound { sound(self): String { "default" } }
type Animal;
impl Animal: Sound {}
"#,
        "must explicitly implement method",
    );
}

#[test]
fn declarations_opt_into_defaults_for_types_and_enums() {
    assert_ok(
        r#"
interface Sound { sound(self): String { "default" } }
type Animal: Sound { name: String }
enum State: Sound { Ready }
fn main() {
    Animal { name: "Cat" }.sound();
    State.Ready.sound();
}
"#,
    );
}

#[test]
fn interface_inheritance_is_transitive_and_requires_parent_methods() {
    assert_ok(
        r#"
interface Parent { parent(self): String; }
interface Child: Parent { child(self): String; }
type A;
impl A: Child {}
impl A {
    parent(self): String { "parent" }
    child(self): String { "child" }
}
fn use_parent<T: Parent>(value: T): String { value.parent() }
fn main() { use_parent(A); }
"#,
    );
    assert_err(
        r#"
interface Parent { parent(self): String; }
interface Child: Parent { child(self): String; }
type A;
impl A: Child { child(self): String { "child" } }
"#,
        "must explicitly implement method `parent`",
    );
}

#[test]
fn conflicting_defaults_and_interface_cycles_are_diagnostics() {
    assert_err(
        r#"
interface B { value(self): String { "b" } }
interface C { value(self): String { "c" } }
type A: B, C;
"#,
        "multiple default implementations",
    );
    assert_err(
        r#"
interface A: B {}
interface B: A {}
"#,
        "interface inheritance cycle",
    );
}

#[test]
fn explicit_generic_impl_block_on_a_builtin_owner_type_checks_and_runs() {
    // `impl<T> Option<T> { ... }` — the only way to write methods for a
    // compiler-builtin owner with no local declaration.
    assert_ok(
        r#"
impl<T> Option<T> {
    unwrap_or(self, fallback: T): T {
        match self {
            Some(item) => item,
            None => fallback,
        }
    }
}
fn main() {
    let value: Option<i32> = Option.Some(4);
    println(`${value.unwrap_or(0)}`);
}
"#,
    );
}

#[test]
fn explicit_impl_generics_reject_a_concrete_target_argument_when_impl_has_its_own_generics() {
    // `impl<T> Option<i32>` mixes the explicit-generic form (`<T>` after
    // `impl`) with a concrete target argument — neither a pure rename nor
    // a concrete specialization (which never writes `impl<...>` at all;
    // see `impl_concrete_specialization_type_checks_and_runs` below), so
    // it stays rejected.
    assert_err(
        "impl<T> Option<i32> { foo(self): bool { true } }\n",
        "must name each of",
    );
}

#[test]
fn explicit_impl_generics_reject_a_repeated_target_argument() {
    assert_err(
        "impl<T> Result<T, T> { foo(self): bool { true } }\n",
        "must name each of",
    );
}

#[test]
fn explicit_impl_generics_reject_an_unused_impl_parameter() {
    assert_err(
        "impl<T, U> Option<T> { foo(self): bool { true } }\n",
        "must appear",
    );
}

#[test]
fn explicit_impl_generic_owner_infers_from_receiver_with_no_other_use_of_t() {
    // Regression: `is_some` doesn't mention `T` in its params/return, so
    // its only source for `T` is the receiver's own concrete type
    // (`Option<i32>` -> `T = i32`), not structural inference over the
    // call's ordinary arguments (there are none here).
    assert_ok(
        r#"
impl<T> Option<T> {
    is_some(self): bool {
        match self {
            Some(_) => true,
            None => false,
        }
    }
}
fn main() {
    let value: Option<i32> = Option.Some(4);
    println(`${value.is_some()}`);
}
"#,
    );
}

#[test]
fn explicit_impl_generics_reject_wrong_arity() {
    assert_err(
        "impl<T> Result<T> { foo(self): bool { true } }\n",
        "takes 2 type argument(s), found 1",
    );
}

#[test]
fn impl_concrete_specialization_type_checks_and_runs() {
    // `impl Option<i32> { ... }` — a concrete specialization, coexisting
    // with the generic `impl<T> Option<T> { ... }` version. Positional
    // renaming (Phase 1) rejected this shape; specialization (Phase 3)
    // accepts it as long as the specialized method's signature matches
    // the generic one once substituted (below).
    assert_ok(
        r#"
impl<T> Option<T> {
    describe(self): String {
        "generic"
    }
}
impl Option<i32> {
    describe(self): String {
        "int"
    }
}
fn main() {
    println(Option.Some(4).describe());
    println(Option.Some("text").describe());
}
"#,
    );
}

#[test]
fn impl_specialization_only_method_is_visible_only_on_its_own_concrete_type() {
    // A method that exists *only* as a specialization (no generic
    // counterpart) is a legal, narrower inherent method — but only
    // callable on that exact concrete instantiation.
    assert_ok(
        r#"
impl Option<i32> {
    double(self): i32 {
        match self {
            Some(item) => item * 2,
            None => 0,
        }
    }
}
fn main() {
    println(`${Option.Some(4).double()}`);
}
"#,
    );
    assert_err(
        r#"
impl Option<i32> {
    double(self): i32 { 0 }
}
fn main() {
    let value: Option<String> = Option.Some("x");
    value.double();
}
"#,
        "has no method named `double`",
    );
}

#[test]
fn impl_specialization_rejects_duplicate_registration() {
    assert_err(
        r#"
impl Option<i32> { double(self): i32 { 0 } }
impl Option<i32> { double(self): i32 { 1 } }
"#,
        "defined more than once for this specialization",
    );
}

#[test]
fn impl_specialization_rejects_a_signature_mismatch_with_the_generic_impl() {
    assert_err(
        r#"
impl<T> Option<T> {
    describe(self): String { "generic" }
}
impl Option<i32> {
    describe(self): i32 { 0 }
}
"#,
        "must have the same signature",
    );
}

#[test]
fn impl_specialization_rejects_static_methods() {
    assert_err(
        "impl Option<i32> { make(): i32 { 0 } }\n",
        "cannot override a static method",
    );
}

#[test]
fn impl_specialization_rejects_interfaces() {
    assert_err(
        r#"
interface Sound { sound(self): String; }
impl Option<i32>: Sound {
    sound(self): String { "beep" }
}
"#,
        "cannot also implement an interface",
    );
}

#[test]
fn variadic_parameter_accepts_zero_or_more_trailing_arguments() {
    assert_ok(
        r#"
fn count(items: ...i32): usize {
    items.len()
}
fn main() {
    let a = count();
    let b = count(1);
    let c = count(1, 2, 3);
}
"#,
    );
}

#[test]
fn variadic_element_type_is_array_inside_the_function_body() {
    assert_ok(
        r#"
fn first_len(items: ...i32): usize {
    let arr: Array<i32> = items;
    arr.len()
}
"#,
    );
}

#[test]
fn variadic_string_element_accepts_any_into_string_value() {
    // Mirrors template-string interpolation's coercion — a variadic
    // `...String` parameter isn't limited to literal `String` arguments.
    assert_ok(
        r#"
fn show(args: ...String) {}
fn main() {
    show(1, true, "text");
}
"#,
    );
}

#[test]
fn variadic_non_string_element_still_requires_a_compatible_type() {
    assert_err(
        r#"
fn count(items: ...i32) {}
fn main() {
    count("not a number");
}
"#,
        "expected `i32`, found `String`",
    );
}

#[test]
fn variadic_call_requires_at_least_the_fixed_argument_count() {
    assert_err(
        r#"
fn f(a: i32, rest: ...i32) {}
fn main() {
    f();
}
"#,
        "expected at least 1 argument(s), found 0",
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
    assert_ok("fn main() { loop { break; } while false { continue; } }");
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
        "must explicitly implement method",
    );
}

#[test]
fn weak_must_wrap_a_heap_type() {
    assert_ok("type Node { next: weak Node }");
    assert_err(
        "type point { x: i32 }\ntype Node { p: weak point }",
        "can only wrap a heap-allocated type",
    );
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
    assert_err(
        "fn main() { let x = true + false; }",
        "require numeric operands",
    );
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
    assert_err(
        "fn main() { let x = 1; x = 2; }",
        "declare it with `let mut`",
    );
    assert_ok("fn main() { let mut x = 1; x = 2; }");
}

#[test]
fn field_assignment_through_an_immutable_binding_is_rejected() {
    // Rust-like mutation enforcement: a binding must be `mut` to mutate
    // through it — directly, via a field (at any depth), or via a `mut
    // self` method call — for both stack and heap types, with no
    // heap-only exemption.
    assert_err(
        r#"
type Dog { name: String }
impl Dog {
    rename(mut self, new_name: String) {
        self.name = new_name;
    }
}
fn main() {
    let d = Dog { name: "Rex" };
    d.name = "Buddy";
}
"#,
        "declare it with `let mut`",
    );
    assert_ok(
        r#"
type Dog { name: String }
fn main() {
    let mut d = Dog { name: "Rex" };
    d.name = "Buddy";
}
"#,
    );
}

#[test]
fn calling_a_mut_self_method_through_an_immutable_receiver_is_rejected() {
    assert_err(
        r#"
type Dog { name: String }
impl Dog {
    rename(mut self, new_name: String) {
        self.name = new_name;
    }
}
fn main() {
    let d = Dog { name: "Rex" };
    d.rename("Buddy");
}
"#,
        "cannot call a `mut self` method",
    );
    assert_ok(
        r#"
type Dog { name: String }
impl Dog {
    rename(mut self, new_name: String) {
        self.name = new_name;
    }
}
fn main() {
    let mut d = Dog { name: "Rex" };
    d.rename("Buddy");
}
"#,
    );
}

#[test]
fn mut_self_method_call_on_a_field_of_mut_self_is_allowed() {
    // A nested field-path rooted at a `mut self` receiver is itself a
    // mutable place — `self.dog.rename()` is legal inside a `mut self`
    // method, matching `self.field = x`'s own root-local rule.
    assert_ok(
        r#"
type Dog { name: String }
impl Dog {
    rename(mut self, new_name: String) {
        self.name = new_name;
    }
}
type Holder { dog: Dog }
impl Holder {
    rename_dog(mut self, new_name: String) {
        self.dog.rename(new_name);
    }
}
fn main() {
    let mut h = Holder { dog: Dog { name: "Rex" } };
    h.rename_dog("Buddy");
}
"#,
    );
}

#[test]
fn array_push_and_pop_require_a_mutable_binding() {
    assert_err(
        "fn main() { let a = [1, 2]; a.push(3); }",
        "cannot call a `mut self` method",
    );
    assert_err(
        "fn main() { let a = [1, 2]; a.pop(); }",
        "cannot call a `mut self` method",
    );
    assert_ok("fn main() { let mut a = [1, 2]; a.push(3); a.pop(); }");
    // `len` doesn't mutate, so it stays legal on a non-`mut` binding.
    assert_ok("fn main() { let a = [1, 2]; let n = a.len(); }");
}

#[test]
fn return_type_mismatch_is_reported() {
    assert_err("fn f(): i32 { return \"x\"; }", "expected return type");
}

#[test]
fn block_with_no_tail_but_a_diverging_last_statement_types_as_never() {
    // A block with no explicit tail expression used to always type as
    // `()`, even when its last (or only) statement unconditionally
    // diverges — `return`/`break`/`continue` written with a trailing
    // semicolon are ordinary `Stmt::Expr`s, whose `Type::Never` was
    // computed and then discarded.
    assert_ok("fn foo(a: i32): i32 { return a; }");
    // There's no dead-code diagnostic in this language, so a diverging
    // statement isn't necessarily the last one — the block must still
    // type as `Never`, not fall back to `()`, when an earlier statement
    // diverges.
    assert_ok("fn foo(a: i32): i32 { return a; let y = 1; }");
    // Both `if`/`else` branches diverging, as ordinary statements inside
    // a fn body with no tail.
    assert_ok(
        r#"
fn foo(a: i32): i32 {
    if a > 0 {
        return a;
    } else {
        return 0 - a;
    }
}
"#,
    );
    // A diverging `let` initializer also propagates.
    assert_ok("fn foo(a: i32): i32 { let x = return a; }");
}

#[test]
fn array_and_index_type_checks() {
    assert_ok("fn main() { let a = [1, 2, 3]; let x = a[0]; }");
    assert_err(
        "fn main() { let a: [i32] = []; let x = a[\"no\"]; }",
        "must be an integer type",
    );
    assert_err("fn main() { let a = []; }", "cannot infer");
}

#[test]
fn array_builtin_methods_type_check() {
    assert_ok(
        "fn main() { let mut a = [1, 2]; a.push(3); let n: usize = a.len(); let p = a.pop(); }",
    );
    assert_err(
        "fn main() { let mut a = [1, 2]; a.push(\"x\"); }",
        "found `String`",
    );
}

#[test]
fn user_defined_impl_on_array_type_checks_and_self_sees_builtin_operations() {
    // `Array<T>` is an ordinary generic type declared in the bundled
    // prelude (`stdlib/array.nt`) now, not a closed compiler builtin — a
    // user `impl<T> Array<T>` block's `self` types as `Array<T>` and can
    // freely mix a user-defined method (`sum`, calling itself indirectly
    // via `for_each`) with the still-runtime-backed builtins (`len`,
    // indexing).
    assert_ok(
        r#"
impl<T> Array<T> {
    for_each(mut self, f: (T) => ()) {
        let mut i: usize = 0;
        while i < self.len() {
            f(self[i]);
            i = i + 1;
        }
    }
}

fn main() {
    let mut a = [1, 2, 3];
    a.for_each((v: i32) => {
        println(`${v}`);
    });
}
"#,
    );
}

#[test]
fn user_defined_array_method_rejects_a_receiver_type_mismatch() {
    // The user method's own parameter types are still checked normally —
    // `Array<T>`'s builtin fast path (`len`/`push`/`pop`/indexing) staying
    // hardcoded doesn't exempt everything else from ordinary type checking.
    assert_err(
        r#"
impl<T> Array<T> {
    first_or(self, fallback: T): T {
        if self.len() > 0 {
            self[0]
        } else {
            fallback
        }
    }
}

fn main() {
    let a = [1, 2, 3];
    let x = a.first_or("nope");
}
"#,
        "found `String`",
    );
}
