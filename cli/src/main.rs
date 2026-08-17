//! `nether` — the compiler's command-line entry point
//! (`docs/architecture/crates.md` § `cli`).
//!
//! Current scope: both subcommands run the full pipeline
//! (`docs/architecture/overview.md` §8): parse, resolve, typecheck, lower
//! to HIR, monomorphize, build MIR + insert ARC, generate + emit an LLVM
//! object file, and link it against `runtime/*` into a native executable
//! (gracefully skipped if those static libraries haven't been built
//! locally yet — see `nether_driver::link`'s own module docs). `check
//! <file>` reports success/diagnostics; `ast <file>` additionally
//! pretty-prints the parsed AST, the original bootstrap smoke test.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let (command, path_arg) = match (args.next(), args.next()) {
        (Some(cmd), Some(path)) if cmd == "check" || cmd == "build" || cmd == "ast" => (cmd, path),
        _ => {
            print_usage();
            return ExitCode::FAILURE;
        }
    };

    let path = PathBuf::from(path_arg);
    let mut options = nether_driver::CompileOptions::default();
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

    match nether_driver::compile(&path, &options) {
        Ok(result) => {
            let has_error = result.diagnostics.iter().any(|d| d.is_error());
            for diag in &result.diagnostics {
                eprint!("{}", nether_diagnostics::render(diag, &result.source_map));
            }
            if !has_error {
                if command == "ast" {
                    println!("{:#?}", result.module);
                } else {
                    let status = if let Some(exe) = &result.executable_path {
                        format!("ok (parsed, resolved, type-checked, lowered, monomorphized, mir-built, linked: {})", exe.display())
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
                    };
                    println!("{status}: {}", path.display());
                }
            }
            if has_error {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(e) => {
            eprintln!("error: {e} ({})", path.display());
            ExitCode::FAILURE
        }
    }
}

fn print_usage() {
    eprintln!(
        "usage: nether <check|build|ast> <file.nr> [--target <triple>] [-O0|-O1|-O2|-O3] [-o <path>] [--emit-object]"
    );
}
