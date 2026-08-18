use super::*;

#[test]
fn variadic_parameter_accepts_zero_or_more_trailing_arguments() {
    assert_ok(
        r#"
fn count(items ...i32) usize {
    return items.len();
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
fn first_len(items ...i32) usize {
    let arr Array<i32> = items;
    return arr.len();
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
fn show(args ...String) {}
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
fn count(items ...i32) {}
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
fn f(a i32, rest ...i32) {}
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
trait Sound { sound(self) String; }
struct Dog;
impl Dog Sound { sound(self) String { return "woof"; } }
struct Cage<T> where T Sound { value T }
fn open<T Sound>(cage Cage<T>) String { return cage.value.sound(); }
fn main() {
    let cage = Cage { value = Dog };
    println(open(cage));
}
"#,
    );
    assert_err(
        r#"
trait Sound { sound(self) String; }
struct Rock;
struct Cage<T> where T Sound { value T }
fn main() { let cage = Cage { value = Rock }; }
"#,
        "required by this generic type",
    );
}

#[test]
fn infinitely_recursive_value_layouts_are_diagnostics_not_codegen_ices() {
    assert_err(
        "struct node { next node }\nfn main() {}",
        "infinitely recursive layout",
    );
    assert_err(
        "enum List { End, Next(List) }\nfn main() {}",
        "infinitely recursive layout",
    );
    assert_err(
        "struct left { right right }\nstruct right { left left }\nfn main() {}",
        "infinitely recursive layout",
    );
    assert_ok(
        r#"
struct Node { next Option<Node> }
struct wrapper { node Node }
fn main() {
    let node = Node { next = Option.None };
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
        "fn main(value i32) {}",
        "`main` must have signature `fn main()`",
    );
    assert_err(
        "trait Sound { sound(self) String; }\nfn use_it(value Sound) {}\nfn main() {}",
        "cannot be used as a value type",
    );
    assert_ok("fn main() { loop { break; } while false { continue; } }");
}

#[test]
fn generic_structs_and_tuple_structs_infer_and_substitute_fields() {
    assert_ok(
        r#"
struct Boxed<T> { value T }
struct pair<T, U>(T, U);
impl Boxed {
    new(value T) Boxed<T> { return Boxed { value }; }
    get(self) T { return self.value; }
    replace<U>(self, value U) Boxed<U> { return Boxed { value }; }
}
fn read_number(value Boxed<i32>) i32 { return value.value; }
fn main() {
    let number = Boxed { value = 42 };
    let text = Boxed { value = "ready" };
    let both = pair(number, text);
    let n i32 = both.0.value;
    let s String = both.1.value;
    let built = Boxed.new(7);
    let built_value i32 = built.get();
    let replaced Boxed<String> = built.replace("new");
    let replaced_value String = replaced.get();
}
"#,
    );
    assert_err(
        r#"
struct Boxed<T> { value T }
fn main() {
    let value Boxed<i32> = Boxed { value = "wrong" };
}
"#,
        "expected `i32`, found `String`",
    );
}

#[test]
fn impl_missing_required_trait_method_reports_diagnostic() {
    assert_err(
        r#"
trait Sound {
    sound() String;
}
struct Dog { name String }
impl Dog Sound {
}
"#,
        "must explicitly implement method",
    );
}

#[test]
fn weak_must_wrap_a_heap_type() {
    assert_ok("struct Node { next weak Node }");
    assert_err(
        "struct point { x i32 }\nstruct Node { p weak point }",
        "can only wrap a heap-allocated type",
    );
}

#[test]
fn a_heap_value_coerces_into_a_weak_field_without_an_explicit_conversion() {
    assert_ok(
        r#"
struct Child { name String }
struct Parent { kid weak Child }
fn main() {
    let c = Child { name = "Rex" };
    let p = Parent { kid = c };
    let mut w weak Child = c;
    w = c;
}
"#,
    );
    assert_err(
        r#"
struct Child { name String }
struct Parent { kid weak Child }
fn main() {
    let p = Parent { kid = 5 };
}
"#,
        "expected `weak Child`, found `i32`",
    );
}

#[test]
fn reading_a_weak_field_produces_an_option_not_a_bare_weak_value() {
    assert_ok(
        r#"
struct Node { next weak Node }
fn describe(n Node) String {
    return match n.next {
        Some(_) => "has next",
        None => "no next",
    };
}
fn main() {}
"#,
    );
    assert_err(
        r#"
struct Node { name String, next weak Node }
fn describe(n Node) String {
    return n.next.name;
}
fn main() {}
"#,
        "has no field named `name`",
    );
}
