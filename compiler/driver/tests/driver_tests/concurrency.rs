use super::*;

#[test]
fn thread_spawn_and_join_run_a_real_os_thread_end_to_end() {
    // `.join()` blocks until the spawned thread finishes, so printing
    // from inside the closure, joining, then printing again from `main`
    // is a deterministic ordering even though the print itself runs on
    // a genuinely different OS thread.
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!("nether_thread_spawn_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
struct Counter { value i32 }
unsafe impl Counter Send { }

fn main() {
    let counter = Counter { value = 7 };
    let handle = thread.spawn(move () => {
        println(`from thread: ${counter.value}`);
    });
    handle.join();
    println("from main");
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
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "from thread: 7\nfrom main\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn thread_spawn_rejects_a_spawned_borrow_of_a_local_end_to_end() {
    let dir = std::env::temp_dir().join(format!(
        "nether_thread_spawn_borrow_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
struct Dog { name String }
fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    let r: &Dog = dog;
    thread.spawn(move () => {
        println(r.name);
    });
}
"#,
    )
    .unwrap();
    let result = nether_driver::check(&entry).unwrap();
    assert!(
        result.diagnostics.iter().any(|d| d
            .message
            .contains("cannot spawn a thread capturing a borrow of a local owned by this function")),
        "{:?}",
        result.diagnostics
    );
    let _ = std::fs::remove_dir_all(&dir);
}
