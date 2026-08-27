use super::*;

#[test]
fn a_program_written_with_no_semicolons_at_all_compiles_links_and_runs_end_to_end() {
    // Stage 7's newline-aware optional semicolons (language-spec §2.5) —
    // every statement here relies on ASI instead of an explicit `;`,
    // except the one documented gap: a struct literal ends in `}`, which
    // is not an ASI trigger, so that one `let` keeps its `;`.
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!(
        "nether_asi_no_semicolons_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
struct Counter {
    value i32
}

impl Counter {
    increment(mut self) {
        self.value = self.value + 1
    }
}

fn main() {
    // A struct literal ends in `}`, which is deliberately not an ASI
    // trigger (language-spec §2.5) — this one `let` still needs its `;`.
    let mut counter = Counter { value = 1 };
    counter.increment()
    counter.increment()
    println(`${counter.value}`)
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
    assert_eq!(String::from_utf8_lossy(&output.stdout), "3\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn block_comments_run_end_to_end() {
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!(
        "nether_block_comment_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
/* This whole program is documented with block comments. */
fn add(a i32, /* first operand */ b i32) i32 {
    /*
     * multi-line block comment
     */
    return a + b;
}
fn main() {
    println(`${add(2, 3)}`);
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
    assert_eq!(String::from_utf8_lossy(&output.stdout), "5\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_mut_closure_mutating_a_captured_inline_local_writes_back_end_to_end() {
    // Mutable closure captures (language-spec §16, Stage 7): `mut (...) =>
    // {...}` captures a `mut`-declared inline local by reference, so
    // mutating it from inside the closure body is visible to the outer
    // binding once the closure returns — real compiled/executed proof,
    // not just a type-shape check, that the write actually lands in the
    // *same* storage rather than a private copy.
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!(
        "nether_mut_closure_capture_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
fn main() {
    let mut count = 0;
    let mut increment = mut () => {
        count = count + 1;
    };
    increment();
    increment();
    increment();
    println(`${count}`);
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
    assert_eq!(String::from_utf8_lossy(&output.stdout), "3\n");
    let _ = std::fs::remove_dir_all(&dir);
}
