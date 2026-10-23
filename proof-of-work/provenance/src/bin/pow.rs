//! CLI: `pow <file.nr> [--naive]` печатает выведенные контракты и диагностики;
//! `pow --experiments` запускает все эксперименты и печатает отчёт.

use provenance::infer::{infer_joint, infer_naive, source_order};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--experiments") {
        print!("{}", provenance::experiments::report());
        return;
    }
    let Some(path) = args.iter().find(|a| !a.starts_with("--")) else {
        eprintln!("usage: pow <file.nr> [--naive] | pow --experiments");
        std::process::exit(2);
    };
    let src = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("{path}: {e}");
        std::process::exit(2)
    });
    let bodies = match provenance::compile(&src) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
    };
    let naive = args.iter().any(|a| a == "--naive");
    let out = if naive {
        infer_naive(&bodies)
    } else {
        infer_joint(&bodies, &source_order(bodies.len()))
    };
    println!(
        "mode: {} ({} function analyses)",
        if naive { "naive" } else { "joint" },
        out.analyses
    );
    for b in &bodies {
        println!("  {}", out.contracts[&b.name].render(&b.name));
    }
    let mut ds = out.diags.clone();
    ds.sort_by_key(|d| (d.line, d.code));
    for d in &ds {
        println!("{}:{}: {} in {}: {}", path, d.line, d.code, d.func, d.msg);
    }
    if ds.is_empty() {
        println!("ok");
    } else {
        std::process::exit(1);
    }
}
