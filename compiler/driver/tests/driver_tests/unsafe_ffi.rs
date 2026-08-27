use super::*;

#[test]
fn extern_c_declaration_calls_a_real_libc_function_end_to_end() {
    // No `#[link]` needed: libc is linked automatically by `cc` (the
    // system linker driver `link.rs` already invokes) — this proves the
    // extern-declaration/unsafe-call/raw-pointer path compiles, links, and
    // runs correctly against a real C ABI before adding `#[link]`'s own
    // complexity (that's `#[link]`'s own end-to-end test).
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!("nether_extern_libc_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
extern "C" {
    fn abs(n i32) i32;
}

fn main() {
    let a = unsafe { abs(-7) };
    println(`${a}`);

    let mut x = 3;
    let p = &raw mut x;
    unsafe { *p = 5; }
    println(`${x}`);
    println(`${unsafe { *p }}`);
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
    assert_eq!(String::from_utf8_lossy(&output.stdout), "7\n5\n5\n");
    let _ = std::fs::remove_dir_all(&dir);
}
