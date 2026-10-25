use nether_core::{check::check, hir, interpret, mir};
use nether_frontend::{parser::parse, source::SourceId};
use nether_semantics::OverflowChecks;

fn checked(source: &str) -> Result<hir::Program, Vec<String>> {
    let parsed = parse(SourceId(0), source);
    assert!(
        parsed.diagnostics.is_empty(),
        "{source}: {:?}",
        parsed.diagnostics
    );
    check(&parsed.module.unwrap())
        .map_err(|errors| errors.into_iter().map(|e| e.code.into()).collect())
}

#[test]
fn unsafe_calls_require_a_lexical_block_and_keep_nmm_checks() {
    for (source, code) in [
        ("unsafe fn f():i32 {return 7} fn main():i32 {return f()}", "E0500"),
        ("unsafe fn f():i32 {return 7} unsafe fn g():i32 {return f()} fn main():i32 {return 0}", "E0500"),
        ("unsafe fn f<T>(x:T):T {return x} fn main():i32 {return f(7)}", "E0500"),
        ("struct S {unsafe fn f(this):i32 {return 7}} fn main():i32 {let s=S {};return s.f()}", "E0500"),
        ("unsafe fn f():i32 {return 7} fn main():i32 {unsafe {let fptr=f};return 0}", "E0501"),
        ("unsafe fn f<T>(x:T):T {return x} fn main():i32 {unsafe {let fptr:(i32)=>i32=f};return 0}", "E0501"),
        ("interface I {fn f(this):i32} struct S:I {unsafe fn f(this):i32 {return 1}} fn main():i32 {return 0}", "E0401"),
        ("class A {virtual fn f(this):i32 {return 1}} class B:A {override unsafe fn f(this):i32 {return 2}} fn main():i32 {return 0}", "E0411"),
        ("unsafe fn main():i32 {return 0}", "E0303"),
        ("struct R {x:i32} fn main():i32 {let a=R {x:1};unsafe {var b=a;b.x=2};return 0}", "E0317"),
        ("struct R {x:i32} fn main():i32 {var a=R {x:1};let r=a;unsafe {a.x=2};return r.x}", "E0381"),
        ("struct R {x:i32} struct H {r:R} fn store(r:R):H {return H {r:r}} fn main():i32 {let a=R {x:1};let r=a;unsafe {let h=store(r)};return 0}", "NMM_NO_OWNER_TOKEN"),
        ("unsafe fn f():i32 {return 7} fn main():i32 {unsafe {let n=f()};return f()}", "E0500"),
    ] {
        let errors = checked(source).expect_err(source);
        assert!(errors.iter().any(|c| c == code), "{source}: {errors:?}");
    }
}

#[test]
fn unsafe_blocks_preserve_return_loop_and_cleanup_lowering() {
    for (source, expected) in [
        ("unsafe fn f():i32 {return 7} fn main():i32 {unsafe {return f()}}", 7),
        ("unsafe fn f<T>(x:T):T {return x} fn main():i32 {unsafe {unsafe {return f(8)}}}", 8),
        ("unsafe fn f():i32 {return 7} unsafe fn g():i32 {unsafe {return f()}} fn main():i32 {unsafe {return g()}}", 7),
        ("struct S {unsafe fn f(this):i32 {return 9}} fn main():i32 {let s=S {};unsafe {return s.f()}}", 9),
        ("fn main():i32 {var n=0;while n<10 {unsafe {n+=1;if n==3 {break};continue}};return n}", 3),
        ("struct R {x:i32;destructor {}} fn main():i32 {unsafe {let r=R {x:4};return r.x}}", 4),
    ] {
        let program = checked(source).unwrap_or_else(|e| panic!("{source}: {e:?}"));
        let cfg = mir::lower(&program);
        let a = interpret::execute(&program, program.main.unwrap(), vec![], OverflowChecks::Checked, 10_000).unwrap();
        let b = mir::interpret::execute(&cfg, cfg.main.unwrap(), vec![], OverflowChecks::Checked, 10_000).unwrap();
        assert_eq!(a, b, "{source}");
        let hir::Value::Integer(n) = a else {panic!()};
        assert_eq!(n.signed_value(), Some(expected), "{source}");
        nether_core::rust::emit(&program, OverflowChecks::Checked).unwrap();
    }
}
