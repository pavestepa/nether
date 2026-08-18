use super::*;

fn run_returned_reference_program(test_name: &str, source: &str, expected_stdout: &str) {
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!("{test_name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(&entry, source).unwrap();
    let result = nether_driver::check(&entry).unwrap();
    assert!(
        result
            .diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.is_error()),
        "unexpected diagnostics: {:?}",
        result.diagnostics
    );
    let output = Command::new(result.executable_path.unwrap())
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), expected_stdout);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reference_returned_from_one_parameter_runs_end_to_end() {
    run_returned_reference_program(
        "nether_returned_reference_test",
        r#"
struct Dog { name String }
fn identity(d: &Dog): &Dog { return d; }
fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    println(identity(dog).name);
    println(dog.name);
}
"#,
        "Rex\nRex\n",
    );
}

#[test]
fn reference_returned_through_call_chain_can_be_stored() {
    run_returned_reference_program(
        "nether_stored_returned_reference_test",
        r#"
struct Dog { name String }
fn identity(d: &Dog): &Dog { return d; }
fn forward(d: &Dog): &Dog { return identity(d); }
fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    let view = forward(dog);
    println(view.name);
    println(dog.name);
}
"#,
        "Rex\nRex\n",
    );
}

#[test]
fn method_and_closure_summaries_work_with_nll_end_to_end() {
    run_returned_reference_program(
        "nether_method_closure_nll_test",
        r#"
struct Dog { name String }
impl Dog { view(: &self): &Dog { return self; } }
fn main() {
    let mut dog: Dog = :Dog { name = "Rex" };
    let first = dog.view();
    println(first.name);
    dog.name = "Buddy";
    let identity = (d: &Dog) => d;
    let second = identity(dog);
    println(second.name);
    dog.name = "Max";
    println(dog.name);
}
"#,
        "Rex\nBuddy\nMax\n",
    );
}

#[test]
fn inline_references_are_addressed_and_dereferenced_end_to_end() {
    run_returned_reference_program(
        "nether_inline_reference_test",
        r#"
fn identity(n: &i32): &i32 { return n; }
fn read(n: &i32) i32 { return n; }
fn main() {
    let number: i32 = 41;
    let view: &i32 = number;
    let forwarded = identity(view);
    println(`${read(forwarded)}`);
    let plain i32 = forwarded;
    println(`${plain + 1}`);
}
"#,
        "41\n42\n",
    );
}

#[test]
fn mutable_inline_reference_writes_through_to_its_origin() {
    run_returned_reference_program(
        "nether_mut_inline_reference_test",
        r#"
fn set(value: &mut i32) { value = 42; }
fn main() {
    let mut number: i32 = 1;
    set(number);
    let plain i32 = to(number);
    println(`${plain}`);
}
"#,
        "42\n",
    );
}

#[test]
fn clone_to_unique_allocates_an_independent_outer_object() {
    run_returned_reference_program(
        "nether_clone_to_unique_test",
        r#"
Clone struct Dog { name String }
fn main() {
    let original = Dog { name = "Rex" };
    let mut owned: Dog = to(original);
    owned.name = "Max";
    println(original.name);
    println(owned.name);
}
"#,
        "Rex\nMax\n",
    );
}

#[test]
fn clone_to_unique_recursively_clones_unique_fields() {
    run_returned_reference_program(
        "nether_recursive_clone_to_unique_test",
        r#"
Clone struct Collar { size i32 }
Clone struct Dog { collar: Collar }
fn main() {
    let original = Dog { collar = :Collar { size = 4 } };
    let mut owned: Dog = to(original);
    owned.collar.size = 7;
    println(`${original.collar.size}`);
    println(`${owned.collar.size}`);
}
"#,
        "4\n7\n",
    );
}

#[test]
fn to_unique_uses_a_user_defined_clone_override() {
    run_returned_reference_program(
        "nether_user_clone_to_unique_test",
        r#"
Clone struct Dog { name String }
impl Dog {
    clone(: &self): Dog {
        return :Dog { name = `copy of ${self.name}` };
    }
}
fn main() {
    let original = Dog { name = "Rex" };
    let owned: Dog = to(original);
    println(original.name);
    println(owned.name);
}
"#,
        "Rex\ncopy of Rex\n",
    );
}

#[test]
fn move_closure_owns_and_drops_a_unique_heap_capture() {
    run_returned_reference_program(
        "nether_move_closure_unique_capture_test",
        r#"
struct Dog { name String }
fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    let name = move () => dog.name;
    println(name());
}
"#,
        "Rex\n",
    );
}

#[test]
fn derived_eq_compares_values_instead_of_outer_object_addresses() {
    run_returned_reference_program(
        "nether_derived_eq_test",
        r#"
Eq struct Point { x i32, y i32 }
Eq struct BoxedPoint { point: Point }
fn main() {
    let a = Point { x = 1, y = 2 };
    let b = Point { x = 1, y = 2 };
    let c = Point { x = 1, y = 3 };
    println(`${a == b}`);
    println(`${a != c}`);

    let boxed_a = BoxedPoint { point = :Point { x = 4, y = 5 } };
    let boxed_b = BoxedPoint { point = :Point { x = 4, y = 5 } };
    println(`${boxed_a == boxed_b}`);

    let owned_a: Point = :Point { x = 8, y = 9 };
    let owned_b: Point = :Point { x = 8, y = 9 };
    println(`${owned_a == owned_b}`);
}
"#,
        "true\ntrue\ntrue\ntrue\n",
    );
}

#[test]
fn derived_hash_is_structural_for_arc_nested_and_unique_values() {
    run_returned_reference_program(
        "nether_derived_hash_test",
        r#"
Hash struct Point { x i32, y i32 }
Hash struct BoxedPoint { point: Point }
fn main() {
    let a = Point { x = 1, y = 2 };
    let b = Point { x = 1, y = 2 };
    let c = Point { x = 1, y = 3 };
    println(`${hash(a) == hash(b)}`);
    println(`${hash(a) != hash(c)}`);

    let boxed_a = BoxedPoint { point = :Point { x = 4, y = 5 } };
    let boxed_b = BoxedPoint { point = :Point { x = 4, y = 5 } };
    println(`${hash(boxed_a) == hash(boxed_b)}`);

    let owned_a: Point = :Point { x = 8, y = 9 };
    let owned_b: Point = :Point { x = 8, y = 9 };
    println(`${hash(owned_a) == hash(owned_b)}`);
}
"#,
        "true\ntrue\ntrue\ntrue\n",
    );
}
