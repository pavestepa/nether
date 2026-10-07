use nether_frontend::{
    modules::load,
    parser::parse,
    source::{Diagnostic, Source, SourceId},
};
use nether_semantics::OverflowChecks;
use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
fn render(sources: &[Source], errors: &[Diagnostic]) -> String {
    errors
        .iter()
        .map(|d| {
            sources
                .get(d.span.source.0)
                .map_or_else(|| format!("{}: {}", d.code, d.message), |s| s.render(d))
        })
        .collect::<Vec<_>>()
        .join("\n")
}
fn run() -> Result<(), String> {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.is_empty() || args == ["--help"] {
        println!("Nether 0.1 development\nUsage: nether <syntax|check|eval|emit-llvm|build> <file.nr> [options]\nOptions: --release --overflow-checks=on|off --package-name=app\nBuild: --emit=object -o <output>\nsyntax checks grammar only; eval is bounded reference execution.\nTarget: x86_64-unknown-linux-gnu; set NETHER_CLANG / NETHER_LINKER for toolchain paths.");
        return Ok(());
    }
    if args == ["--version"] {
        println!("nether 0.1.0 (development)");
        return Ok(());
    }
    if args.len() < 2
        || !["syntax", "check", "eval", "emit-llvm", "build"].contains(&args[0].as_str())
    {
        return Err("invalid command; use --help".into());
    }
    let mut release = false;
    let mut override_checks = None;
    let mut object = false;
    let mut output = None;
    let mut package = "app".to_owned();
    let mut index = 2;
    while index < args.len() {
        match args[index].as_str() {
            "--release" => release = true,
            "--overflow-checks=on" => override_checks = Some(OverflowChecks::Checked),
            "--overflow-checks=off" => override_checks = Some(OverflowChecks::Wrapping),
            "--emit=object" if args[0] == "build" => object = true,
            "-o" if args[0] == "build" => {
                index += 1;
                output = Some(PathBuf::from(args.get(index).ok_or("-o requires a path")?));
            }
            option if option.starts_with("--package-name=") => {
                package = option[15..].into();
                if package.is_empty() {
                    return Err("package name cannot be empty".into());
                }
            }
            option => return Err(format!("unknown option '{option}'")),
        }
        index += 1;
    }
    let checks = override_checks.unwrap_or(if release {
        OverflowChecks::Wrapping
    } else {
        OverflowChecks::Checked
    });
    if args[0] == "syntax" {
        let text = fs::read_to_string(&args[1]).map_err(|e| format!("{}: {e}", args[1]))?;
        let source = Source::new(&args[1], text);
        let parsed = parse(SourceId(0), &source.text);
        if !parsed.diagnostics.is_empty() {
            return Err(render(&[source], &parsed.diagnostics));
        }
        println!("syntax passed (not a type or ownership check)");
        return Ok(());
    }
    let loaded = load(Path::new(&args[1]), &package);
    if !loaded.diagnostics.is_empty() {
        return Err(render(&loaded.sources, &loaded.diagnostics));
    }
    let program = nether_core::check::check(&loaded.module.unwrap())
        .map_err(|d| render(&loaded.sources, &d))?;
    match args[0].as_str() {
        "check" => println!("type check passed (implemented value subset)"),
        "eval" => {
            let main = program.main.ok_or("no main function")?;
            let value =
                nether_core::interpret::execute(&program, main, Vec::new(), checks, 1_000_000)
                    .map_err(|t| {
                        render(
                            &loaded.sources,
                            &[Diagnostic::new(
                                "R0001",
                                format!("reference execution failed: {:?}", t.kind),
                                t.span,
                            )],
                        )
                    })?;
            println!("{value:?}");
        }
        command => {
            let ir = nether_core::llvm::emit_with_sources(&program, checks, &loaded.sources)
                .map_err(|d| render(&loaded.sources, &[d]))?;
            if command == "emit-llvm" {
                print!("{ir}");
            } else {
                if !object && program.main.is_none() {
                    return Err("executable requires main".into());
                }
                if !object && !cfg!(target_os = "linux") && env::var_os("NETHER_LINKER").is_none() {
                    return Err("Linux linker/sysroot required on this host; use --emit=object or configure NETHER_LINKER".into());
                }
                let input = Path::new(&args[1]);
                let output =
                    output.unwrap_or_else(|| input.with_extension(if object { "o" } else { "" }));
                let canonical_output = output.canonicalize().ok();
                if output == input
                    || loaded.sources.iter().any(|source| {
                        canonical_output
                            .as_ref()
                            .is_some_and(|path| *path == PathBuf::from(&source.name))
                    })
                {
                    return Err("output would overwrite source".into());
                }
                build(&ir, &output, object, release)?;
                println!("built {}", output.display());
            }
        }
    }
    Ok(())
}
fn build(ir: &str, output: &Path, object: bool, release: bool) -> Result<(), String> {
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = parent.join(format!(".nether-{}.o", std::process::id()));
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    let mut executable = None;
    let result = (|| {
        let compiler = env::var_os("NETHER_CLANG").unwrap_or_else(|| "clang".into());
        let mut process = Command::new(compiler)
            .args([
                "--target=x86_64-unknown-linux-gnu",
                "-x",
                "ir",
                "-c",
                if release { "-O2" } else { "-O0" },
                "-",
                "-o",
            ])
            .arg(&temporary)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot start LLVM compiler: {e}"))?;
        let write = process.stdin.take().unwrap().write_all(ir.as_bytes());
        let result = process.wait_with_output().map_err(|e| e.to_string())?;
        if !result.status.success() {
            return Err(format!(
                "LLVM compilation failed: {}",
                String::from_utf8_lossy(&result.stderr)
            ));
        }
        write.map_err(|e| e.to_string())?;
        if object {
            fs::rename(&temporary, output).map_err(|e| e.to_string())?;
        } else {
            let linker = env::var_os("NETHER_LINKER").unwrap_or_else(|| "cc".into());
            let staged = parent.join(format!(".nether-{}.exe", std::process::id()));
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&staged)
                .map_err(|e| e.to_string())?;
            executable = Some(staged.clone());
            let mut process = Command::new(linker)
                .arg(&temporary)
                .args(["-std=c11", "-x", "c", "-", "-lm", "-o"])
                .arg(&staged)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| format!("cannot start Linux linker: {e}"))?;
            let write = process
                .stdin
                .take()
                .unwrap()
                .write_all(include_bytes!("../../runtime/src/runtime.c"));
            let result = process.wait_with_output().map_err(|e| e.to_string())?;
            if !result.status.success() {
                return Err(format!(
                    "link failed: {}",
                    String::from_utf8_lossy(&result.stderr)
                ));
            }
            write.map_err(|e| e.to_string())?;
            fs::rename(&staged, output).map_err(|e| e.to_string())?;
            executable = None;
        }
        Ok(())
    })();
    if let Some(staged) = executable {
        let _ = fs::remove_file(staged);
    }
    if temporary.exists() {
        let _ = fs::remove_file(temporary);
    }
    result
}
