use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn published_examples_follow_the_current_memory_profile() {
    let examples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples");
    for entry in fs::read_dir(examples).unwrap() {
        let source = entry.unwrap().path().join("main.nr");
        if !source.is_file() {
            continue;
        }
        let output = Command::new(env!("CARGO_BIN_EXE_nether"))
            .arg("check")
            .arg(&source)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}: {}",
            source.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
struct Project(PathBuf);
impl Project {
    fn new(source: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "nether-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("main.nr"), source).unwrap();
        Self(path)
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_nether"))
            .current_dir(&self.0)
            .args(args)
            .output()
            .unwrap()
    }
}
impl Drop for Project {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
#[test]
fn commands_load_imports_and_report_original_locations() {
    let project = Project::new("import {answer} from \"./lib\"\nfn main():i32 {return answer()}");
    fs::write(project.0.join("lib.nr"), "fn answer():i32 {return 42}").unwrap();
    assert!(project.run(&["check", "main.nr"]).status.success());
    let result = project.run(&["eval", "main.nr"]);
    assert!(result.status.success());
    assert!(String::from_utf8_lossy(&result.stdout).contains("42"));
    fs::write(
        project.0.join("lib.nr"),
        "fn answer():i32 {\nreturn true\n}",
    )
    .unwrap();
    let result = project.run(&["check", "main.nr"]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("lib.nr:2:"));
}
#[test]
fn build_emits_host_object_without_overwriting_imported_sources() {
    let project = Project::new("import {answer} from \"./lib\"\nfn main():i32 {return answer()}");
    fs::write(project.0.join("lib.nr"), "fn answer():i32 {return 42}").unwrap();
    let result = project.run(&["build", "main.nr", "--emit=object", "-o", "main.o"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(fs::metadata(project.0.join("main.o")).unwrap().len() > 0);
    for output in ["main.nr", "lib.nr"] {
        let result = project.run(&["build", "main.nr", "--emit=object", "-o", output]);
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("overwrite source"));
    }
}
#[test]
fn release_overflow_policy_can_be_overridden() {
    let project = Project::new("fn main():i32 {let x:i8=127;return (x+1) as i32}");
    assert!(!project.run(&["eval", "main.nr"]).status.success());
    assert!(project
        .run(&["eval", "main.nr", "--release"])
        .status
        .success());
    assert!(!project
        .run(&["eval", "main.nr", "--release", "--overflow-checks=on"])
        .status
        .success());
}

#[test]
fn rust_source_and_host_executables_are_available_without_clang() {
    let project = Project::new("fn main():i32 {let x:i8=127;return (x+1) as i32}");
    let source = project.run(&["emit-rust", "main.nr"]);
    assert!(source.status.success());
    assert!(String::from_utf8_lossy(&source.stdout).contains("fn main()"));
    assert!(!project.run(&["emit-llvm", "main.nr"]).status.success());
    for (options, expected) in [
        (vec!["build", "main.nr", "-o", "program"], 101),
        (vec!["build", "main.nr", "--release", "-o", "program"], 128),
        (
            vec![
                "build",
                "main.nr",
                "--release",
                "--overflow-checks=on",
                "-o",
                "program",
            ],
            101,
        ),
    ] {
        let built = project.run(&options);
        assert!(
            built.status.success(),
            "{}",
            String::from_utf8_lossy(&built.stderr)
        );
        let result = Command::new(project.0.join("program")).output().unwrap();
        assert_eq!(result.status.code(), Some(expected));
    }
}

#[test]
fn failed_rust_compilation_preserves_existing_output() {
    let project = Project::new("fn main():i32 {return 0}");
    fs::write(project.0.join("program"), "previous output").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_nether"))
        .current_dir(&project.0)
        .args(["build", "main.nr", "-o", "program"])
        .env("NETHER_RUSTC", project.0.join("missing-rustc"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot start Rust compiler"));
    assert_eq!(
        fs::read_to_string(project.0.join("program")).unwrap(),
        "previous output"
    );
    assert!(!fs::read_dir(&project.0).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".nether-")));
}

#[test]
fn borrow_diagnostics_show_origin_creation_and_later_use() {
    let project=Project::new("import {Cell, id} from \"./lib\"\nfn main():i32 {\nvar a=Cell {value:1}\nlet ref source=a\nlet b=id(source)\na=Cell {value:2}\nreturn b.value\n}");
    fs::write(
        project.0.join("lib.nr"),
        "struct Cell {value:i32}\nfn id<T>(x:T):T {return x}",
    )
    .unwrap();
    let output = project.run(&["check", "main.nr"]);
    assert!(!output.status.success());
    let message = String::from_utf8_lossy(&output.stderr);
    for expected in [
        "error[E0381]",
        "main.nr:3:",
        "main.nr:5:",
        "main.nr:7:",
        "borrowed target 'a' originates here",
        "this view is created here",
        "this later use keeps the view live",
    ] {
        assert!(message.contains(expected), "missing {expected}: {message}");
    }
}
