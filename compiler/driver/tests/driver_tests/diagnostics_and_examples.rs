use super::*;

#[test]
fn invalid_program_diagnostics_remain_source_anchored() {
    let dir = std::env::temp_dir().join(format!("nether_diagnostics_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("broken.nr");
    std::fs::write(
        &entry,
        "fn main() {\n    let value bool = 1;\n    println(value);\n}\n",
    )
    .unwrap();
    let result = nether_driver::check(&entry).unwrap();
    assert!(result
        .diagnostics
        .iter()
        .any(nether_diagnostics::Diagnostic::is_error));
    let rendered = result
        .diagnostics
        .iter()
        .map(|diagnostic| nether_diagnostics::render(diagnostic, &result.source_map))
        .collect::<String>();
    assert!(rendered.contains("broken.nr:2:"), "{rendered}");
    assert!(
        rendered.contains("expected `bool`, found `i32`"),
        "{rendered}"
    );
    assert!(
        result.object_path.is_none(),
        "front-end errors must stop before codegen"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn compile_options_control_output_optimization_and_linking() {
    let dir = std::env::temp_dir().join(format!("nether_options_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    let output = dir.join("custom-binary");
    std::fs::write(&entry, "fn main() { println(\"ok\"); }\n").unwrap();
    let options = CompileOptions {
        target_triple: nether_llvm::Codegen::host_triple(),
        opt_level: 2,
        output_path: Some(output),
        link: false,
    };
    let result = nether_driver::compile(&entry, &options).unwrap();
    assert!(result
        .diagnostics
        .iter()
        .all(|diagnostic| !diagnostic.is_error()));
    let object = result.object_path.expect("expected object output");
    assert_eq!(object, dir.join("custom-binary.o"));
    assert!(object.exists());
    assert!(result.executable_path.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Stage 1's checked-in examples are deliberately just the one canonical
/// program (`examples/hello_world/main.nr`, mirroring language-spec.md
/// §23) rather than the pre-rewrite MVP's larger showcase set
/// (`shelter`/`metrics`/`adventure`/`some/*`), which used old syntax
/// throughout and was removed as part of this migration (see
/// `docs/architecture/roadmap.md`). Restoring a larger example corpus is
/// future work, not a Stage 1 requirement.
#[test]
fn every_checked_in_example_compiles_to_an_object() {
    let examples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let dir = std::env::temp_dir().join(format!("nether_examples_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = examples.join("hello_world/main.nr");
    let output = dir.join("example_0");
    let options = CompileOptions {
        output_path: Some(output),
        link: false,
        ..CompileOptions::default()
    };
    let result = nether_driver::compile(&path, &options).unwrap();
    assert!(
        result
            .diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.is_error()),
        "{} failed: {:?}",
        path.display(),
        result.diagnostics
    );
    assert!(result.object_path.is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn larger_example_projects_compile_link_and_run() {
    ensure_runtime_built();

    let examples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let dir =
        std::env::temp_dir().join(format!("nether_large_examples_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cases = [(
        "hello_world",
        concat!(
            "Woof, I am Rex!\n",
            "sum of coordinates: 0\n",
            "Buddy\n",
            "count = 3\n",
            "owned_count checks out\n",
        ),
    )];

    for (name, expected_stdout) in cases {
        let executable = dir.join(name);
        let options = CompileOptions {
            output_path: Some(executable.clone()),
            link: true,
            ..CompileOptions::default()
        };
        let result = nether_driver::compile(&examples.join(name).join("main.nr"), &options)
            .unwrap_or_else(|error| panic!("{name} failed to compile: {error}"));
        assert!(
            result
                .diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.is_error()),
            "{name} diagnostics: {:?}",
            result.diagnostics
        );
        let output = Command::new(&executable)
            .output()
            .unwrap_or_else(|error| panic!("failed to run {name}: {error}"));
        assert!(
            output.status.success(),
            "{name} exited with {}",
            output.status
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            expected_stdout,
            "{name} produced unexpected output"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}
