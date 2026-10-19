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
        .map(|d| d.render(sources))
        .collect::<Vec<_>>()
        .join("\n")
}
fn run() -> Result<(), String> {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.is_empty() || args == ["--help"] {
        println!("Nether 0.1 development\nUsage: nether <syntax|check|eval|emit-rust|build> <file.nr> [options]\nOptions: --release --overflow-checks=on|off --package-name=app\nBuild: --emit=object -o <output>\nsyntax checks grammar only; eval is bounded reference execution.\nTarget: host (64-bit); set NETHER_RUSTC for the Rust compiler path.");
        return Ok(());
    }
    if args == ["--version"] {
        println!("nether 0.1.0 (development)");
        return Ok(());
    }
    if args.len() < 2
        || !["syntax", "check", "eval", "emit-rust", "build"].contains(&args[0].as_str())
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
            let ir = nether_core::rust::emit_with_sources(&program, checks, &loaded.sources)
                .map_err(|d| render(&loaded.sources, &[d]))?;
            if command == "emit-rust" {
                print!("{ir}");
            } else {
                if !object && program.main.is_none() {
                    return Err("executable requires main".into());
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
fn build(source: &str, output: &Path, object: bool, release: bool) -> Result<(), String> {
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = parent.join(format!(".nether-{}.output", std::process::id()));
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        let compiler = env::var_os("NETHER_RUSTC").unwrap_or_else(|| "rustc".into());
        let mut process = Command::new(compiler)
            .args([
                "--edition=2021",
                "--crate-name=nether_generated",
                "-C",
                if release {
                    "opt-level=3"
                } else {
                    "opt-level=0"
                },
            ])
            .args(if object {
                vec!["--crate-type=lib", "--emit=obj"]
            } else {
                vec![]
            })
            .args(["-", "-o"])
            .arg(&temporary)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot start Rust compiler: {e}"))?;
        let write = process.stdin.take().unwrap().write_all(source.as_bytes());
        let result = process.wait_with_output().map_err(|e| e.to_string())?;
        if !result.status.success() {
            return Err(format!(
                "Rust compilation failed: {}",
                String::from_utf8_lossy(&result.stderr)
            ));
        }
        write.map_err(|e| e.to_string())?;
        fs::rename(&temporary, output).map_err(|e| e.to_string())
    })();
    let _ = fs::remove_file(&temporary);
    result
}
