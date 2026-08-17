use super::*;

fn run_returned_reference_program(test_name: &str, source: &str) {
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
    assert_eq!(String::from_utf8_lossy(&output.stdout), "Rex\nRex\n");
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
    );
}
