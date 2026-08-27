use super::*;

#[test]
fn link_attribute_links_a_real_libm_function_end_to_end() {
    // Distinct from `unsafe_ffi::extern_c_declaration_calls_a_real_libc_function_end_to_end`
    // (which deliberately uses no `#[link]` at all, since libc is linked
    // automatically): this proves `#[link(name = "...")]` itself is
    // collected from the module graph and threaded into the real `cc`
    // invocation as `-l<name>` (`link::link`'s own docs).
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!("nether_link_attr_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
#[link(name = "m")]
extern "C" {
    fn sqrt(x f64) f64;
}

fn main() {
    let result = unsafe { sqrt(16.0) };
    println(`${result}`);
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
    assert_eq!(String::from_utf8_lossy(&output.stdout), "4\n");
    let _ = std::fs::remove_dir_all(&dir);
}
