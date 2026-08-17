use super::*;

#[test]
fn reference_returned_from_one_parameter_runs_end_to_end() {
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!(
        "nether_returned_reference_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
struct Dog { name String }
fn identity(d: &Dog): &Dog { return d; }
fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    println(identity(dog).name);
    println(dog.name);
}
"#,
    )
    .unwrap();
    let result = nether_driver::check(&entry).unwrap();
    assert!(
        !result
            .diagnostics
            .iter()
            .any(nether_diagnostics::Diagnostic::is_error),
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
