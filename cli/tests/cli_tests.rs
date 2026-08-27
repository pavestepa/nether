//! End-to-end tests for the `nether` binary itself (Stage 6's CLI
//! overhaul) — invokes the real compiled executable via
//! `env!("CARGO_BIN_EXE_nether")`, the standard way to test a binary
//! crate, rather than calling `nether_driver` directly (that's what
//! `compiler/driver/tests/driver_tests.rs` already does).

use std::process::{Command, Stdio};
use std::sync::OnceLock;

fn ensure_runtime_built() {
    static RUNTIME_BUILT: OnceLock<()> = OnceLock::new();
    RUNTIME_BUILT.get_or_init(|| {
        let status = Command::new("cargo")
            .args([
                "build",
                "--release",
                "-p",
                "nether-rt-arc",
                "-p",
                "nether-rt-string",
                "-p",
                "nether-rt-array",
                "-p",
                "nether-rt-io",
                "-p",
                "nether-rt-task",
                "-p",
                "nether-rt-thread",
            ])
            .status()
            .expect("failed to invoke cargo to build the runtime crates");
        assert!(status.success(), "building runtime/* failed");
    });
}

fn nether() -> Command {
    Command::new(env!("CARGO_BIN_EXE_nether"))
}

fn write_source(dir_name: &str, source: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("{dir_name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(&entry, source).unwrap();
    entry
}

#[test]
fn run_compiles_and_executes_passing_stdout_and_exit_code_through() {
    ensure_runtime_built();
    let entry = write_source(
        "nether_cli_run_test",
        "fn main() { println(\"hello from run\"); }\n",
    );
    let output = nether()
        .args(["run", entry.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "hello from run\n"
    );
    let _ = std::fs::remove_dir_all(entry.parent().unwrap());
}

#[test]
fn test_command_reports_ok_and_exit_zero_for_a_passing_program() {
    ensure_runtime_built();
    let entry = write_source("nether_cli_test_ok", "fn main() { println(\"ok\"); }\n");
    let output = nether()
        .args(["test", entry.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ok\n"));
    assert!(stdout.contains("... ok"), "{stdout}");
    let _ = std::fs::remove_dir_all(entry.parent().unwrap());
}

#[test]
fn test_command_reports_failed_and_a_nonzero_exit_for_a_crashing_program() {
    ensure_runtime_built();
    let entry = write_source(
        "nether_cli_test_fail",
        "fn main() { let arr = [1, 2, 3]; println(`${arr[10]}`); }\n",
    );
    let output = nether()
        .args(["test", entry.to_str().unwrap()])
        .stderr(Stdio::null())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("... FAILED"), "{stdout}");
    let _ = std::fs::remove_dir_all(entry.parent().unwrap());
}

#[test]
fn release_flag_still_compiles_and_runs_correctly() {
    ensure_runtime_built();
    let entry = write_source(
        "nether_cli_release_test",
        "fn main() { println(`${2 + 2}`); }\n",
    );
    let output = nether()
        .args(["run", entry.to_str().unwrap(), "--release"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "4\n");
    let _ = std::fs::remove_dir_all(entry.parent().unwrap());
}

#[test]
fn emit_flags_each_produce_a_real_sibling_file() {
    ensure_runtime_built();
    let entry = write_source(
        "nether_cli_emit_test",
        "fn main() { println(\"emit check\"); }\n",
    );
    let status = nether()
        .args([
            "build",
            entry.to_str().unwrap(),
            "--emit-ast",
            "--emit-hir",
            "--emit-mir",
            "--emit-llvm",
        ])
        .status()
        .unwrap();
    assert!(status.success());
    for ext in ["ast", "hir", "mir", "ll"] {
        let sibling = entry.with_extension(ext);
        let contents = std::fs::read_to_string(&sibling)
            .unwrap_or_else(|e| panic!("expected {}: {e}", sibling.display()));
        assert!(
            !contents.trim().is_empty(),
            "{} was empty",
            sibling.display()
        );
    }
    let ll = std::fs::read_to_string(entry.with_extension("ll")).unwrap();
    assert!(ll.contains("ModuleID"), "{ll}");
    let _ = std::fs::remove_dir_all(entry.parent().unwrap());
}

#[test]
fn unknown_command_reports_usage_and_fails() {
    let output = nether().args(["frobnicate", "x.nr"]).output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown command"));
}

#[test]
fn ast_command_pretty_prints_the_parsed_module() {
    let entry = write_source("nether_cli_ast_test", "fn main() { }\n");
    let output = nether()
        .args(["ast", entry.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Fn"));
    let _ = std::fs::remove_dir_all(entry.parent().unwrap());
}
