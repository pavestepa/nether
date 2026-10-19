//! A reproducible probe, not a universal performance guarantee or a timing CI gate.
use nether_core::{check::check, rust::emit};
use nether_frontend::{parser::parse, source::SourceId};
use nether_semantics::OverflowChecks;
#[test]
#[ignore = "explicit optimized comparison against equivalent handwritten Rust"]
fn arithmetic_loop_against_handwritten_rust() {
    let source = "fn kernel(var value:u64,count:u64):u64 {var i:u64=0;while i<count {value=(value ^ (value >> 13))*6364136223846793005+1;i+=1};return value}";
    let parsed = parse(SourceId(0), source);
    let program = check(&parsed.module.unwrap()).unwrap();
    let mut generated = emit(&program, OverflowChecks::Wrapping).unwrap();
    generated.push_str(r#"
#[inline(never)]
fn handwritten(mut value: u64, count: u64) -> u64 {
    let mut i = 0;
    while i < count {
        value = (value ^ (value >> 13)).wrapping_mul(6364136223846793005).wrapping_add(1);
        i += 1;
    }
    value
}
fn main() {
    use std::{hint::black_box, time::Instant};
    let mut rust_times = Vec::new();
    let mut nether_times = Vec::new();
    for round in 0..9 {
        let seed = black_box(43 + round);
        let count = black_box(2_000_000);
        let mut measure = |nether: bool| {
            let start = Instant::now();
            let result = if nether {unsafe {f0(seed,count)}.unwrap()} else {handwritten(seed,count)};
            black_box(result);
            let elapsed = start.elapsed().as_nanos();
            if nether {nether_times.push(elapsed)} else {rust_times.push(elapsed)};
            result
        };
        let (a,b) = if round%2==0 {(measure(true),measure(false))} else {(measure(false),measure(true))};
        assert_eq!(a,b);
    }
    rust_times.sort(); nether_times.sort();
    println!("median handwritten={}ns Nether={}ns ratio={:.3}",rust_times[4],nether_times[4],nether_times[4] as f64/rust_times[4] as f64);
}
"#);
    let dir = std::env::temp_dir().join(format!("nether-performance-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("probe.rs"), generated).unwrap();
    let compiled = std::process::Command::new(
        std::env::var("NETHER_RUSTC").unwrap_or_else(|_| "rustc".into()),
    )
    .args(["--edition=2021", "-C", "opt-level=3"])
    .arg(dir.join("probe.rs"))
    .arg("-o")
    .arg(dir.join("probe"))
    .output()
    .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let output = std::process::Command::new(dir.join("probe"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!("{}", String::from_utf8_lossy(&output.stdout));
    std::fs::remove_dir_all(dir).unwrap();
}
