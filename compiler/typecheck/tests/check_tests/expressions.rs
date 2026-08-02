use super::*;

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
