//! `nether` — the compiler's command-line entry point
//! (`docs/architecture/crates.md` § `cli`).
//!
//! Every subcommand runs the full pipeline
//! (`docs/architecture/overview.md` §8): parse, resolve, typecheck, lower
//! to HIR, monomorphize, build MIR + insert ARC, generate + emit an LLVM
//! object file, and link it against `runtime/*` into a native executable
//! (gracefully skipped if those static libraries haven't been built
//! locally yet — see `nether_driver::link`'s own module docs).
//!
//! - `check <file>` reports success/diagnostics only.
//! - `build <file>` is identical to `check` (kept as its own command for
//!   the more conventional name).
//! - `ast <file>` additionally pretty-prints the parsed AST to stdout,
//!   the original bootstrap smoke test.
//! - `run <file>` (Stage 6) additionally executes the linked binary,
//!   passing its stdout/stderr/exit code straight through — `cargo run`'s
//!   own convention.
//! - `test <file>` (Stage 6) compiles and runs like `run`, framing the
//!   result as `test <path> ... ok`/`FAILED` by the process's own exit
//!   code. No in-source `#[test]` discovery — deliberately out of this
//!   stage's scope (see `docs/architecture/roadmap.md`'s Stage 6 entry).
//!
//! `--release` is sugar for `-O3`. `--emit-ast`/`--emit-hir`/`--emit-mir`/
//! `--emit-llvm` each write a debug dump to a sibling file (`<output or
//! source>.ast`/`.hir`/`.mir`/`.ll`, mirroring `--emit-object`'s own
//! sibling-naming convention) — available data only: `--emit-hir`/
//! `--emit-mir` are silent no-ops if an earlier stage never ran.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use nether_driver::CheckResult;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        print_usage();
        return ExitCode::FAILURE;
    };
    if !matches!(command.as_str(), "check" | "build" | "ast" | "run" | "test") {
        eprintln!("error: unknown command `{command}`");
        print_usage();
        return ExitCode::FAILURE;
    }
    let Some(path_arg) = args.next() else {
        print_usage();
        return ExitCode::FAILURE;
    };
    let path = PathBuf::from(path_arg);

    let mut options = nether_driver::CompileOptions::default();
    let mut emit_ast = false;
    let mut emit_hir = false;
    let mut emit_mir = false;
    let mut emit_llvm = false;
    let remaining: Vec<String> = args.collect();
    let mut index = 0;
    while index < remaining.len() {
        match remaining[index].as_str() {
            "--target" if index + 1 < remaining.len() => {
                options.target_triple = remaining[index + 1].clone();
                index += 2;
            }
            "-o" if index + 1 < remaining.len() => {
                options.output_path = Some(PathBuf::from(&remaining[index + 1]));
                index += 2;
            }
            "--emit-object" => {
                options.link = false;
                index += 1;
            }
            "--release" => {
                options.opt_level = 3;
                index += 1;
            }
            "--emit-ast" => {
                emit_ast = true;
                index += 1;
            }
            "--emit-hir" => {
                emit_hir = true;
                index += 1;
            }
            "--emit-mir" => {
                emit_mir = true;
                index += 1;
            }
            "--emit-llvm" => {
                emit_llvm = true;
                index += 1;
            }
            flag if flag.len() == 3 && flag.starts_with("-O") => {
                options.opt_level = match flag[2..].parse::<u8>() {
                    Ok(level @ 0..=3) => level,
                    _ => {
                        eprintln!("error: optimization level must be -O0, -O1, -O2, or -O3");
                        return ExitCode::FAILURE;
                    }
                };
                index += 1;
            }
            other => {
                eprintln!("error: unknown or incomplete option `{other}`");
                print_usage();
                return ExitCode::FAILURE;
            }
        }
    }

    // Every `--emit-*` sibling file is named off this same base, mirroring
    // `nether_driver::compile`'s own `<output-or-source>.o` convention.
    let emit_base = options.output_path.clone().unwrap_or_else(|| path.clone());
    if emit_llvm {
        options.emit_llvm_path = Some(emit_base.with_extension("ll"));
    }

    let result = match nether_driver::compile(&path, &options) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("error: {e} ({})", path.display());
            return ExitCode::FAILURE;
        }
    };

    let has_error = result.diagnostics.iter().any(|d| d.is_error());
    for diag in &result.diagnostics {
        eprint!("{}", nether_diagnostics::render(diag, &result.source_map));
    }

    if emit_ast
        && write_emit_dump(
            &emit_base.with_extension("ast"),
            format!("{:#?}", result.module),
        )
        .is_err()
    {
        return ExitCode::FAILURE;
    }
    if emit_hir {
        // `HirModule::signatures` (`nether_typecheck::Signatures`) has no
        // `Debug` impl (and adding one would cascade across most of that
        // crate's own types) — dump the function bodies alone, the part
        // someone reaching for `--emit-hir` actually wants to read.
        if let Some(hir) = &result.hir {
            if write_emit_dump(&emit_base.with_extension("hir"), format!("{:#?}", hir.fns)).is_err() {
                return ExitCode::FAILURE;
            }
        }
    }
    if emit_mir {
        if let Some(mir) = &result.mir {
            if write_emit_dump(&emit_base.with_extension("mir"), format!("{mir:#?}")).is_err() {
                return ExitCode::FAILURE;
            }
        }
    }

    if has_error {
        return ExitCode::FAILURE;
    }

    match command.as_str() {
        "ast" => {
            println!("{:#?}", result.module);
            ExitCode::SUCCESS
        }
        "run" | "test" => run_or_test(command == "test", &path, &result),
        _ => {
            println!("{}: {}", status_line(&result), path.display());
            ExitCode::SUCCESS
        }
    }
}

fn write_emit_dump(path: &Path, contents: String) -> std::io::Result<()> {
    std::fs::write(path, contents).map_err(|e| {
        eprintln!("error: failed to write `{}`: {e}", path.display());
        e
    })
}

fn status_line(result: &CheckResult) -> String {
    if let Some(exe) = &result.executable_path {
        format!(
            "ok (parsed, resolved, type-checked, lowered, monomorphized, mir-built, linked: {})",
            exe.display()
        )
    } else if let Some(obj) = &result.object_path {
        format!(
            "ok (parsed, resolved, type-checked, lowered, monomorphized, mir-built, object emitted: {} — not linked, build runtime/* first)",
            obj.display()
        )
    } else if result.hir.is_some() {
        "ok (parsed, resolved, type-checked, lowered)".to_string()
    } else if result.resolved.is_some() {
        "ok (parsed, resolved)".to_string()
    } else {
        "ok (parsed)".to_string()
    }
}

/// `run`/`test` (Stage 6) — executes the linked binary, stdout/stderr
/// inherited (passed straight through, `Command::status`'s own default).
/// `is_test` additionally frames the result as `test <path> ... ok`/
/// `FAILED` after the child finishes, but always propagates the child's
/// own exit code either way — a CI script gating on `nether test`'s exit
/// status doesn't need to parse this framing at all.
fn run_or_test(is_test: bool, path: &Path, result: &CheckResult) -> ExitCode {
    let Some(exe) = &result.executable_path else {
        eprintln!(
            "error: nothing to run — no executable was produced (is `runtime/*` built locally?)"
        );
        return ExitCode::FAILURE;
    };
    let status = match Command::new(exe).status() {
        Ok(status) => status,
        Err(e) => {
            eprintln!("error: failed to run {}: {e}", exe.display());
            return ExitCode::FAILURE;
        }
    };
    if is_test {
        if status.success() {
            println!("test {} ... ok", path.display());
        } else {
            println!("test {} ... FAILED", path.display());
        }
    }
    match status.code() {
        Some(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        None => ExitCode::FAILURE,
    }
}

fn print_usage() {
    eprintln!(
        "usage: nether <check|build|ast|run|test> <file.nr> \
         [--target <triple>] [-O0|-O1|-O2|-O3] [--release] [-o <path>] \
         [--emit-object] [--emit-ast] [--emit-hir] [--emit-mir] [--emit-llvm]"
    );
}
