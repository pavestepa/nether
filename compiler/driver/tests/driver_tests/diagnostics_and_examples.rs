use super::*;

#[test]
fn invalid_program_diagnostics_remain_source_anchored() {
    let dir = std::env::temp_dir().join(format!("nether_diagnostics_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("broken.nr");
    std::fs::write(
        &entry,
        "fn main() {\n    let value: bool = 1;\n    println(value);\n}\n",
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
    let entry = dir.join("main.nt");
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

#[test]
fn every_checked_in_example_compiles_to_an_object() {
    let examples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let dir = std::env::temp_dir().join(format!("nether_examples_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut paths = std::fs::read_dir(examples.join("some"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "nt"))
        .collect::<Vec<_>>();
    paths.extend([
        examples.join("hello_world/main.nt"),
        examples.join("shelter/main.nt"),
        examples.join("metrics/main.nt"),
        examples.join("adventure/main.nt"),
    ]);
    paths.sort();
    assert_eq!(paths.len(), 8, "expected all checked-in user entry points");
    for (index, path) in paths.into_iter().enumerate() {
        let output = dir.join(format!("example_{index}"));
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
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn larger_example_projects_compile_link_and_run() {
    ensure_runtime_built();

    let examples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let dir =
        std::env::temp_dir().join(format!("nether_large_examples_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cases = [
        (
            "shelter",
            concat!(
                "shelter=North Star\n",
                "featured=Luna\n",
                "1. Luna (2)\n",
                "2. Milo (4)\n",
                "3. Nova (1)\n",
                "reserved until Saturday\n",
                "card: Echo, age 3\n",
                "no pet selected\n",
            ),
        ),
        (
            "metrics",
            concat!(
                "count=4, total=20, range=2..8\n",
                "count=4, total=60, range=6..24\n",
                "count=3, total=14, range=1..9\n",
                "counter=11\n",
                "metrics complete\n",
                "total=20, max=8\n",
            ),
        ),
        (
            "adventure",
            concat!(
                "hero Arin: hp=20, score=0\n",
                "active quest: Ancient Gate\n",
                "A wild shadow approaches Arin\n",
                "received 6 damage\n",
                "found treasure worth 12\n",
                "Arin: hp=14, score=27\n",
                "quest complete, reward=125\n",
                "The road continues\n",
            ),
        ),
    ];

    for (name, expected_stdout) in cases {
        let executable = dir.join(name);
        let options = CompileOptions {
            output_path: Some(executable.clone()),
            link: true,
            ..CompileOptions::default()
        };
        let result = nether_driver::compile(&examples.join(name).join("main.nt"), &options)
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
