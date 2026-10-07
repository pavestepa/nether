use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
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
fn build_emits_elf_without_overwriting_imported_sources() {
    let project = Project::new("import {answer} from \"./lib\"\nfn main():i32 {return answer()}");
    fs::write(project.0.join("lib.nr"), "fn answer():i32 {return 42}").unwrap();
    let result = project.run(&["build", "main.nr", "--emit=object", "-o", "main.o"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(fs::read(project.0.join("main.o"))
        .unwrap()
        .starts_with(b"\x7fELF"));
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
