use nether_core::{check::check, hir, interpret, mir, rust};
use nether_frontend::{parser::parse, source::SourceId};
use nether_semantics::OverflowChecks;

fn checked(source: &str) -> Result<hir::Program, Vec<String>> {
    let parsed = parse(SourceId(0), source);
    assert!(
        parsed.diagnostics.is_empty(),
        "{source}: {:?}",
        parsed.diagnostics
    );
    check(&parsed.module.unwrap()).map_err(|e| e.into_iter().map(|e| e.code.to_owned()).collect())
}
#[test]
fn raw_scalar_operations_match_in_both_interpreters() {
    for (source, expected) in [
        ("fn main():i32 {let a={1,7};let n:isize=1;unsafe {return *(&a[0]+n)}}",7),
        ("fn main():i32 {let n=7;let p=&n;let ref v=p;let q:*i32=*v;unsafe {return *q}}",7),
        ("unsafe fn read(p:*i32):i32 {unsafe {return *p}} fn main():i32 {let n=7;let f:unsafe (*i32)=>i32=read;unsafe {return f(&n)}}",7),

        (include_str!("../../../examples/raw_pointers/main.nr"),8),
        ("fn main():i32 {var n=2;let p=&var n;let q=p;unsafe {*q=7;return *p}}",7),
        ("fn main():i32 {let n=7;let p:*i32=&n;unsafe {return *p}}",7),
        ("fn identity<T:Copy>(p:*T):*T {return p} fn main():i32 {let n=7;let p=identity(&n);unsafe {return *p}}",7),
        ("struct S {p:*i32} fn main():i32 {let n=7;let s=S {p:&n};unsafe {return *s.p}}",7),
        ("fn main():i32 {let a={7,3};let p=&a[0];unsafe {let end=p+2;let first=end+(-2);return *first}}",7),
        ("struct S {n:i32} fn main():i32 {var s=S {n:1};let p=&var s.n;unsafe {*p=7};return s.n}",7),
        ("class C {n:i32;constructor() {n=1}} fn main():i32 {var c=new C();let p=&var c.n;unsafe {*p=7};return c.n}",7),
        ("fn main():i32 {var n=1;let ref var v=n;let p=&var v;unsafe {*p=7};return n}",7),
        ("fn main():i32 {var b=false;let p=&var b;unsafe {*p=true;if *p {return 7}};return 0}",7),
    ] {
        let p=checked(source).unwrap_or_else(|e|panic!("{source}: {e:?}"));
        let m=mir::lower(&p);
        for mode in [OverflowChecks::Checked,OverflowChecks::Wrapping] {
            let a=interpret::execute(&p,p.main.unwrap(),vec![],mode,10000).unwrap_or_else(|e|panic!("HIR {source}: {e:?}"));
            let b=mir::interpret::execute(&m,m.main.unwrap(),vec![],mode,10000).unwrap_or_else(|e|panic!("MIR {source}: {e:?}"));
            assert_eq!(a,b,"{source}");
            let hir::Value::Integer(n)=a else {panic!()};
            assert_eq!(n.signed_value(),Some(expected),"{source}");
            rust::emit(&p,mode).unwrap();
        }
    }
}
#[test]
fn raw_permissions_and_capabilities_cannot_be_erased() {
    for source in [
        "fn main():i32 {var n=7;let p=&var n;unsafe {let q:i32=p as *i32};return 0}",
        "fn main():i32 {var n=7;let p=&var n;unsafe {let q:i32=(*p=3)};return 0}",
        "fn main():i32 {var n=7;let p=&var n;unsafe {let q:i32=p+0};return 0}",
    ] {
        assert!(
            checked(source).is_err(),
            "accepted wrong annotation: {source}"
        );
    }

    for (source, code) in [
        ("fn main():i32 {let n=7;return *(&n)}", "E0500"),
        (
            "unsafe fn f(p:*i32):i32 {return *p} fn main():i32 {return 0}",
            "E0500",
        ),
        ("fn main():i32 {let n=7;let p=&var n;return 0}", "E0317"),
        (
            "fn main():i32 {var n=7;let p=&n;unsafe {*p=3};return n}",
            "E0503",
        ),
        (
            "fn main():i32 {let n=7;let p=&n;unsafe {let q=p as *var i32};return 0}",
            "E0503",
        ),
        (
            "fn main():i32 {let n=7;let p=&n;unsafe {let q=p as *u8};return 0}",
            "E0503",
        ),
        (
            "fn main():i32 {var n=7;let p=&var n;let q=p as *i32;return 0}",
            "E0500",
        ),
        (
            "fn main():i32 {let n=7;let p=&n;let q=p+1;return 0}",
            "E0500",
        ),
        ("fn main():i32 {let p=&(1+2);return 0}", "E0502"),
        (
            "struct S {n:i32} fn main():i32 {let s=S {n:7};let p=&s;return 0}",
            "E0502",
        ),
        (
            "fn main():i32 {var n=7;let ref v=n;let p=&var n;return v}",
            "E0381",
        ),
        (
            "fn main():i32 {var n=7;let ref v=n;let p=&var v;return 0}",
            "E0317",
        ),
    ] {
        let errors = checked(source).expect_err(source);
        assert!(errors.iter().any(|e| e == code), "{source}: {errors:?}");
    }
}
#[test]
fn reference_interpreters_diagnose_dangling_and_out_of_bounds_raw_access() {
    // Invalid unsafe programs: never execute these as native Rust (undefined behavior).
    for source in [
        "fn dangling():*i32 {let n=7;return &n} fn main():i32 {let p=dangling();unsafe {return *p}}",
        "fn main():i32 {let a={1,2};let p=&a[0];unsafe {return *(p+2)}}",
        "fn main():i32 {let a={1,2};let p=&a[0];unsafe {let q=p+3};return 0}",
    ] {
        let p=checked(source).unwrap();let m=mir::lower(&p);
        let a=interpret::execute(&p,p.main.unwrap(),vec![],OverflowChecks::Checked,10000).unwrap_err();
        let b=mir::interpret::execute(&m,m.main.unwrap(),vec![],OverflowChecks::Checked,10000).unwrap_err();
        assert_eq!(a.kind,b.kind,"{source}");
    }
}
