use super::*;

#[test]
fn nether_toml_local_path_dependency_compiles_links_and_runs_end_to_end() {
    // Mirrors `rust_style_mod_files_relative_roots_and_cross_module_impls_run`'s
    // own shape, one level up: two *separate* package directories linked
    // by a `Nether.toml` `path` dependency instead of a same-crate `mod`
    // declaration — the dependency gets its own independent `crate_root`
    // (`self`/`crate`/`super` resolve relative to *its own* root), the
    // same treatment the bundled stdlib already got before Stage 6.
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!(
        "nether_manifest_dep_test_{}",
        std::process::id()
    ));
    let app_dir = dir.join("app");
    let mathlib_dir = dir.join("mathlib");
    std::fs::create_dir_all(&app_dir).unwrap();
    std::fs::create_dir_all(&mathlib_dir).unwrap();

    std::fs::write(
        app_dir.join("Nether.toml"),
        r#"
[package]
name = "app"

[dependencies]
mathlib = { path = "../mathlib" }
"#,
    )
    .unwrap();
    std::fs::write(
        mathlib_dir.join("mod.nr"),
        r#"
pub fn add(a i32, b i32) i32 { return a + b; }
"#,
    )
    .unwrap();
    let entry = app_dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
use mathlib.add;
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
fn nether_toml_missing_dependency_path_is_a_source_anchored_diagnostic() {
    let dir = std::env::temp_dir().join(format!(
        "nether_manifest_missing_dep_test_{}",
        std::process::id()
    ));
    let app_dir = dir.join("app");
    std::fs::create_dir_all(&app_dir).unwrap();
    std::fs::write(
        app_dir.join("Nether.toml"),
        r#"
[package]
name = "app"

[dependencies]
ghost = { path = "../does-not-exist" }
"#,
    )
    .unwrap();
    let entry = app_dir.join("main.nr");
    std::fs::write(&entry, "use ghost.thing;\nfn main() {}\n").unwrap();

    let result = nether_driver::check(&entry).unwrap();
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.message.contains("cannot find module `ghost`")),
        "{:?}",
        result.diagnostics
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn absent_nether_toml_still_compiles_an_ordinary_single_package_program() {
    // Manifest-less compiles must keep working exactly as they always
    // have — a `Nether.toml` is opt-in, never required.
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!(
        "nether_manifest_absent_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(&entry, "fn main() { println(\"no manifest needed\"); }\n").unwrap();
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
        "no manifest needed\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
