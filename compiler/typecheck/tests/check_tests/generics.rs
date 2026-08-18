use super::*;

#[test]
fn explicit_generic_call_arguments_control_specialization() {
    assert_ok(
        r#"
fn opaque<T>(value i32) i32 { return value; }
fn pair<T, U>(left T, right U) (T, U) { return (left, right); }

struct Boxed<T> { value T }
impl Boxed {
    replace<U>(self, value U) Boxed<U> { return Boxed { value }; }
}

fn main() {
    let value i32 = opaque<String>(2);
    let both (u32, String) = pair<u32, String>(2, "ready");
    let number = Boxed { value = 1 };
    let text Boxed<String> = number.replace<String>("one");
}
"#,
    );
    assert_ok(
        r#"
trait Transform {
    transform<U>(self, value U) U;
}
struct Boxed<T> { value T }
impl Boxed Transform {
    transform<U>(self, value U) U { return value; }
}
fn apply<T Transform>(value T) String {
    return value.transform<String>("ready");
}
fn main() {
    let result String = apply(Boxed { value = 1 });
}
"#,
    );
    assert_err(
        r#"
fn identity<T>(value T) T { return value; }
fn main() { identity<u32, String>(2); }
"#,
        "expected 1 explicit generic argument(s), found 2",
    );
    assert_err(
        r#"
fn identity<T>(value T) T { return value; }
fn main() { identity<String>(2); }
"#,
        "expected `String`, found `i32`",
    );
    assert_err(
        r#"
trait Sound { sound(self) String; }
fn make_noise<T Sound>(value T) String { return value.sound(); }
fn main() { make_noise<u32>(2); }
"#,
        "does not implement `Sound`",
    );
}

#[test]
fn into_string_bound_is_checked_inside_generic_bodies() {
    assert_ok(
        r#"
fn stringify<T Into<String>>(value T) String {
    return value.into_string();
}
fn main() {
    let number = stringify(42);
    let text = stringify("ready");
}
"#,
    );
    assert_err(
        r#"
fn invalid<T>(value T) {
    println(value);
}
"#,
        "cannot be converted to `String`",
    );
}

#[test]
fn generic_traits_preserve_arguments_and_specialize_methods() {
    assert_ok(
        r#"
trait Convert<T> {
    convert(self) T;
}
trait Identity<T> {
    identity(self, value T) T { return value; }
}
struct Dog Identity<String> { name String }
impl Dog Convert<String> {
    convert(self) String { return self.name; }
}
fn convert<U Convert<String>>(value U) String {
    return value.convert();
}
fn identify<U Identity<String>>(value U) String {
    return value.identity("ready");
}
fn main() {
    let dog = Dog { name = "Bobby" };
    let name String = convert(dog);
    let state String = identify(dog);
}
"#,
    );
    assert_err(
        r#"
trait Convert<T> { convert(self) T; }
struct Dog;
impl Dog Convert<i32> { convert(self) i32 { 1 } }
fn convert<U Convert<String>>(value U) String { return value.convert(); }
fn main() { convert(Dog); }
"#,
        "does not implement `Convert<String>`",
    );
    assert_err(
        r#"
trait Convert<T> { convert(self) T; }
struct Dog;
impl Dog Convert<String> { convert(self) i32 { 1 } }
"#,
        "does not match its declaration",
    );
    assert_ok(
        r#"
trait Read<T> { read(self) T; }
struct Boxed<T> { value T }
impl Boxed Read<T> { read(self) T { return self.value; } }
fn read_text<U Read<String>>(value U) String { return value.read(); }
fn main() {
    let text = Boxed { value = "ready" };
    println(read_text(text));
}
"#,
    );
}

#[test]
fn static_methods_can_be_called_through_values() {
    assert_ok(
        r#"
struct Animal { name String }
impl Animal {
    new(name String) Animal { return Animal { name }; }
    static_method() String { return "A"; }
}
fn main() {
    let animal = Animal.new("Cat");
    let value String = animal.static_method();
}
"#,
    );
    assert_ok(
        r#"
trait Static { value() String; }
struct Animal;
impl Animal Static {
    value() String { return "A"; }
}
fn read<T Static>(animal T) String {
    return animal.value();
}
fn main() {
    let value String = read(Animal);
}
"#,
    );
}

#[test]
fn trait_implementations_can_be_mixed_across_blocks() {
    assert_ok(
        r#"
trait B { b(self) String; }
trait C { c(self) String; }
struct A;
impl A B, C {}
impl A { b(self) String { return "b"; } }
impl A { c(self) String { return "c"; } }
fn use_b<T B>(value T) String { return value.b(); }
fn use_c<T C>(value T) String { return value.c(); }
fn main() {
    use_b(A);
    use_c(A);
}
"#,
    );
    assert_err(
        r#"
trait Sound { sound(self) String { return "default"; } }
struct Animal;
impl Animal Sound {}
"#,
        "must explicitly implement method",
    );
}

#[test]
fn declarations_opt_into_defaults_for_types_and_enums() {
    assert_ok(
        r#"
trait Sound { sound(self) String { return "default"; } }
struct Animal Sound { name String }
enum State Sound { Ready }
fn main() {
    Animal { name = "Cat" }.sound();
    State.Ready.sound();
}
"#,
    );
}

#[test]
fn trait_inheritance_is_transitive_and_requires_parent_methods() {
    assert_ok(
        r#"
trait Parent { parent(self) String; }
trait Child Parent { child(self) String; }
struct A;
impl A Child {}
impl A {
    parent(self) String { return "parent"; }
    child(self) String { return "child"; }
}
fn use_parent<T Parent>(value T) String { return value.parent(); }
fn main() { use_parent(A); }
"#,
    );
    assert_err(
        r#"
trait Parent { parent(self) String; }
trait Child Parent { child(self) String; }
struct A;
impl A Child { child(self) String { "child" } }
"#,
        "must explicitly implement method `parent`",
    );
}

#[test]
fn conflicting_defaults_and_trait_cycles_are_diagnostics() {
    assert_err(
        r#"
trait B { value(self) String { return "b"; } }
trait C { value(self) String { return "c"; } }
struct A B, C;
"#,
        "multiple default implementations",
    );
    assert_err(
        r#"
trait A B {}
trait B A {}
"#,
        "trait inheritance cycle",
    );
}

#[test]
fn explicit_generic_impl_block_on_a_builtin_owner_type_checks_and_runs() {
    // `impl<T> Option<T> { ... }` — the only way to write methods for a
    // compiler-builtin owner with no local declaration.
    assert_ok(
        r#"
impl<T> Option<T> {
    unwrap_or(self, fallback T) T {
        return match self {
            Some(item) => item,
            None => fallback,
        };
    }
}
fn main() {
    let value Option<i32> = Option.Some(4);
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
        "impl<T> Option<i32> { foo(self) bool { true } }\n",
        "must name each of",
    );
}

#[test]
fn explicit_impl_generics_reject_a_repeated_target_argument() {
    assert_err(
        "impl<T> Result<T, T> { foo(self) bool { true } }\n",
        "must name each of",
    );
}

#[test]
fn explicit_impl_generics_reject_an_unused_impl_parameter() {
    assert_err(
        "impl<T, U> Option<T> { foo(self) bool { true } }\n",
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
    is_some(self) bool {
        return match self {
            Some(_) => true,
            None => false,
        };
    }
}
fn main() {
    let value Option<i32> = Option.Some(4);
    println(`${value.is_some()}`);
}
"#,
    );
}

#[test]
fn explicit_impl_generics_reject_wrong_arity() {
    assert_err(
        "impl<T> Result<T> { foo(self) bool { true } }\n",
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
default impl<T> Option<T> {
    describe(self) String {
        return "generic";
    }
}
impl Option<i32> {
    describe(self) String {
        return "int";
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
    double(self) i32 {
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
    double(self) i32 { return 0; }
}
fn main() {
    let value Option<String> = Option.Some("x");
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
impl Option<i32> { double(self) i32 { 0 } }
impl Option<i32> { double(self) i32 { 1 } }
"#,
        "defined more than once for this specialization",
    );
}

#[test]
fn impl_specialization_rejects_a_signature_mismatch_with_the_generic_impl() {
    assert_err(
        r#"
default impl<T> Option<T> {
    describe(self) String { return "generic"; }
}
impl Option<i32> {
    describe(self) i32 { return 0; }
}
"#,
        "must have the same signature",
    );
}

#[test]
fn impl_specialization_rejects_static_methods() {
    assert_err(
        "impl Option<i32> { make() i32 { 0 } }\n",
        "cannot override a static method",
    );
}

#[test]
fn impl_specialization_rejects_traits() {
    assert_err(
        r#"
trait Sound { sound(self) String; }
impl Option<i32> Sound {
    sound(self) String { return "beep"; }
}
"#,
        "cannot also implement a trait",
    );
}

#[test]
fn specialization_requires_an_explicit_default_generic_impl() {
    assert_err(
        r#"
impl<T> Option<T> { describe(self) String { return "generic"; } }
impl Option<i32> { describe(self) String { return "int"; } }
"#,
        "generic implementation is not `default`",
    );
    assert_err(
        "default impl Option<i32> { describe(self) String { return \"int\"; } }",
        "must be a generic fallback implementation",
    );
}
