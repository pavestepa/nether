//! Eligibility is deliberately conservative: the optimized and fallback paths
//! must agree on arithmetic traps, observable identity and object lifetimes.
use nether_core::{check::check, rust};
use nether_frontend::{parser::parse, source::SourceId};
use nether_semantics::OverflowChecks;
fn emit(source: &str, checks: OverflowChecks) -> String {
    let parsed = parse(SourceId(0), source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    rust::emit(&check(&parsed.module.unwrap()).unwrap(), checks).unwrap()
}
#[test]
fn recursive_infallible_scalar_entries_use_structured_value_calls() {
    for source in [
        include_str!("../../../performance/bench/fib.nr"),
        include_str!("../../../performance/bench/collatz.nr"),
    ] {
        let fast = emit(source, OverflowChecks::Wrapping);
        assert!(fast.contains("unsafe fn fast0("));
        assert!(!fast.contains("match pc"));
        assert!(!emit(source, OverflowChecks::Checked).contains("unsafe fn fast"));
    }
}
#[test]
fn dynamic_division_and_signed_overflow_keep_fault_paths() {
    for source in [
        "fn divide(a:i32,b:i32):i32 {return a/b} fn main():i32 {return divide(1,0)}",
        "fn main():i32 {let x:i32=-2147483648;return x / -1}",
        "fn main():i32 {let x:i32=-2147483648;return x % -1}",
    ] {
        let code = emit(source, OverflowChecks::Wrapping);
        assert!(!code.contains("unsafe fn fast0("));
        assert!(code.contains("checked_div") || code.contains("checked_rem"));
    }
}
#[test]
fn frame_allocation_requires_closed_nonobservable_lifetimes() {
    let source = include_str!("../../../performance/bench/class_alloc.nr");
    assert!(emit(source, OverflowChecks::Wrapping).contains("// nonescaping frame allocation"));
    assert!(!emit(source, OverflowChecks::Checked).contains("// nonescaping frame allocation"));
    for source in [
        "class C {value:i32;constructor(n:i32) {value=n};destructor {}} fn main():i32 {let c=new C(7);return c.value}",
        "class C {} fn main():i32 {let a=new C();let b=new C();return if same_identity(a,b) {1} else {0}}",
        "class C {value:i32;constructor(n:i32) {value=n}} fn make():C {return new C(7)} fn main():i32 {let c=make();return c.value}",
        "#[Copy] class C {value:i32;constructor(n:i32) {value=n}} fn main():i32 {let a=new C(3);let b=a;return b.value}",
    ] {
        assert!(!emit(source,OverflowChecks::Wrapping).contains("// nonescaping frame allocation"),"{source}");
    }
}
#[test]
fn live_owner_across_loop_prevents_slot_reuse() {
    let source="class C {value:i32;constructor(n:i32) {value=n}} fn main():i32 {var c=new C(1);var n=0;while n<3 {c=new C(n);n+=1};return c.value}";
    let code = emit(source, OverflowChecks::Wrapping);
    // Initial allocation can use the frame, but the replacement site cannot:
    // allocation precedes cleanup of the old owner on each iteration.
    assert_eq!(code.matches("// nonescaping frame allocation").count(), 1);
}

#[test]
fn frame_objects_execute_without_heap_allocations_at_o0_and_o3() {
    use std::{fs, process::Command};
    let source="class C {value:i32;constructor(n:i32) {value=n};fn read(this):i32 {return value}} fn main():i32 {var n=0;var sum=0;while n<10 {let c=new C(n);sum+=c.read();n+=1};return sum}";
    let parsed = parse(SourceId(0), source);
    let p = check(&parsed.module.unwrap()).unwrap();
    let mut code = rust::emit(&p, OverflowChecks::Wrapping)
        .unwrap()
        .replace("fn main() {", "fn unused_main() {");
    code.push_str(
        r#"
struct Counting;
static ALLOCATIONS:std::sync::atomic::AtomicUsize=std::sync::atomic::AtomicUsize::new(0);
unsafe impl std::alloc::GlobalAlloc for Counting {
    unsafe fn alloc(&self,l:std::alloc::Layout)->*mut u8 {
        ALLOCATIONS.fetch_add(1,std::sync::atomic::Ordering::Relaxed);
        std::alloc::GlobalAlloc::alloc(&std::alloc::System,l)
    }
    unsafe fn dealloc(&self,p:*mut u8,l:std::alloc::Layout) {
        std::alloc::GlobalAlloc::dealloc(&std::alloc::System,p,l)
    }
}
#[global_allocator] static ALLOCATOR:Counting=Counting;
"#,
    );
    code.push_str(&format!("fn main() {{ let before=ALLOCATIONS.load(std::sync::atomic::Ordering::Relaxed);let value=unsafe{{f{}(&mut Context{{clock:0,panicking:false}})}}.unwrap();let after=ALLOCATIONS.load(std::sync::atomic::Ordering::Relaxed);assert_eq!(value,45);assert_eq!(after-before,0);}}",p.main.unwrap()));
    let dir = std::env::temp_dir().join(format!("nether-frame-allocation-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("main.rs"), code).unwrap();
    let compiler = std::env::var("NETHER_RUSTC").unwrap_or_else(|_| "rustc".into());
    for opt in ["opt-level=0", "opt-level=3"] {
        let result = Command::new(&compiler)
            .args(["--edition=2021", "-C", opt])
            .arg(dir.join("main.rs"))
            .arg("-o")
            .arg(dir.join("main"))
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let result = Command::new(dir.join("main")).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    fs::remove_dir_all(dir).unwrap();
}
