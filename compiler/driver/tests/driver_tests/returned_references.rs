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
