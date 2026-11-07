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
        ("#[Copy] enum E {A,B} fn main():i32 {var n=E.A;unsafe {let p=(&var n) as *var u32;*p=1};return match n {E.A=>0,E.B=>7}}",7),
        ("#[Copy] enum E {A(u32),B(u32)} fn main():i32 {var n=E.A(2);unsafe {let p=(&var n) as *var {u32;2};*p={1,7}};return match n {E.A(n)=>0,E.B(n)=>n as i32}}",7),

        ("fn main():i32 {let n:u8=1;unsafe {let p=(&n) as *bool;if *p {return 7};return 0}}",7),
        ("fn main():i32 {var n=true;unsafe {let p=(&var n) as *var u8;*p=0;if !n {return 7};return 0}}",7),
        ("fn main():i32 {let n:u32=65;unsafe {let p=(&n) as *char;if *p=='A' {return 7};return 0}}",7),
        ("fn main():i32 {var n='A';unsafe {let p=(&var n) as *var u32;*p=66;if n=='B' {return 7};return 0}}",7),
        ("fn main():i32 {var n:(bool,u8)=(false,7);unsafe {let p=(&var n) as *var u8;*p=1;if n.0 {return n.1 as i32};return 0}}",7),

        ("fn main():i32 {let a={2,7};unsafe {let all=(&a) as usize;let first=(&a[0]) as usize;let second=(&a[1]) as usize;if all==first && second-first==4 {return 7};return 0}}",7),
        ("#[Copy] struct Pair {a:u8;b:u32} fn main():i32 {let a=Pair {a:1,b:7};unsafe {let all=(&a) as usize;let field=(&a.b) as usize;let p=field as *u32;if field-all==4 {return *p as i32};return 0}}",7),

        ("fn main():i32 {var n=2;unsafe {*(&var n)+=5};return n}",7),
        ("fn main():i32 {var n:f32=14.0;unsafe {*(&var n)/=2.0};return n as i32}",7),
        ("fn main():i32 {var n:u8=1;unsafe {*(&var n)<<=3;*(&var n)^=15};return n as i32}",7),
        ("fn address(p:*var i32,count:*var i32):*var i32 {unsafe {*count+=1};return p} fn rhs(p:*var i32):i32 {unsafe {*p=99};return 5} fn main():i32 {var n=2;var count=0;unsafe {*address(&var n,&var count)+=rhs(&var n)};if count==1 {return n};return 0}",7),

        ("fn main():i32 {var n=2;unsafe {let a=(&var n) as isize;let p=a as *var i32;*p=7;return n}}",7),
        ("fn main():i32 {unsafe {let a:isize=-1;let p=a as *i32;let b=p as isize;if b==a {return 7};return 0}}",7),
        ("fn main():i32 {let a={2,7};unsafe {let first=(&a[0]) as usize;let end=(&a[0]+2) as usize;if end-first==8 {return 7};return 0}}",7),
        ("fn main():i32 {let n:f32=7.0;unsafe {let a=(&n) as usize;let p=a as *u32;if *p==1088421888 {return 7};return 0}}",7),

        ("fn main():i32 {var n=2;unsafe {let a=(&var n) as usize;let p=a as *var i32;*p=7;return n}}",7),
        ("fn main():i32 {let a={2,7};unsafe {let address=(&a[0]) as usize;let p=(address+4) as *i32;return *p}}",7),
        ("fn main():i32 {unsafe {let zero:usize=0;let p=zero as *i32;let a=p as usize;if a==0 {return 7};return 0}}",7),
        ("fn main():i32 {var n=1;let p=&var n;unsafe {*p=7};if n==7 {return 7};return 0}",7),
        ("fn main():i32 {var n:u32=16843009;unsafe {let p=(&var n) as *var u8;*(p+1)=7;return (*(p+1)) as i32}}",7),
        ("fn main():i32 {let a:{u32;2}={16843009,117901063};unsafe {let p=(&a[0]) as *u8;let end=p+8;let q=(end-4) as *u32;if *q==117901063 {return 7};return 0}}",7),
        ("fn main():i32 {let a:u64=0;unsafe {let p=(&a) as *u8;let end=p+8;return (end-p) as i32}}",8),
        ("fn main():i32 {var a:u64=0;unsafe {let p=(&var a) as *var u8;let q=p as *var u64;*q=7;return a as i32}}",7),
        ("fn pass<T:Copy>(p:*T):*T {return p} fn main():i32 {let n:f32=7.0;unsafe {let p=pass<u32>((&n) as *u32);if *p==1088421888 {return 7};return 0}}",7),
        ("fn main():i32 {var bits:u32=1065353216;let p=&var bits;unsafe {let q=p as *var f32;*q=7.0;return (*q as i32)}}",7),
        ("fn main():i32 {let bits:u64=4619567317775286272;unsafe {let q=(&bits) as *f64;return *q as i32}}",7),
        ("fn main():i32 {var bits:i32=-1;unsafe {let p=(&var bits) as *var u32;*p=7;return bits}}",7),
        ("fn main():i32 {let a:{u32;2}={1065353216,1088421888};unsafe {let p=(&a[0]) as *f32;return *(p+1) as i32}}",7),
        ("fn main():i32 {let a={2,5,7};let p=&a[0];unsafe {let end=p+3;let last=end-1;let distance:isize=last-p;return *last+(distance as i32)}}",9),
        ("fn main():i32 {var a=(2,5);let p=&var a;unsafe {*p=(3,4);let b=*p;return b.0+b.1}}",7),
        ("fn main():i32 {var a={2,5};let p=&var a;unsafe {*p={3,4};let b=*p;return b[0]+b[1]}}",7),
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
        ("fn main():i32 {let n=2;unsafe {*(&n)+=5};return n}","E0503"),
        ("fn main():i32 {var n=2;*(&var n)+=5;return n}","E0500"),
        ("fn main():i32 {var n=false;unsafe {*(&var n)&=true};return 0}","E0311"),
        ("fn main():i32 {var n:f32=1.0;unsafe {*(&var n)<<=2};return 0}","E0311"),

        ("fn main():i32 {let n=7;let a=(&n) as usize;return 0}","E0500"),
        ("fn main():i32 {let a:usize=0;let p=a as *i32;return 0}","E0500"),
        ("fn main():i32 {let n=7;unsafe {let a=(&n) as u32};return 0}","E0312"),
        ("fn main():i32 {let a:u32=0;unsafe {let p=a as *i32};return 0}","E0312"),
        ("fn main():i32 {let n=();unsafe {let a=(&n) as usize};return 0}","E0503"),

        ("#[Copy] struct R {v:u32;w:u8} fn main():i32 {let n=R {v:0,w:0};unsafe {let p=(&n) as *u8};return 0}","E0503"),
        ("fn main():i32 {let n:(u8,u32)=(0,0);unsafe {let p=(&n) as *u8};return 0}","E0503"),
        ("fn main():i32 {let n=7;let p=&n;unsafe {let q=(&p) as *u64};return 0}","E0503"),
        ("#[Copy] enum E {None,Some(u32)} fn main():i32 {let n=E.None;unsafe {let p=(&n) as *u32};return 0}","E0503"),
        ("fn main():i32 {let n:(u32,u32)=(0,0);unsafe {let p=(&n) as *var {u32;2}};return 0}","E0503"),
        ("unsafe fn bits<T>(p:*T):*u32 {unsafe {return p as *u32}} fn main():i32 {let n:f32=7.0;unsafe {let p=bits<f32>(&n)};return 0}","E0373"),
        ("fn main():i32 {let n:u32=0;let p=(&n) as *f32;return 0}","E0500"),
        ("fn main():i32 {let n:u32=0;unsafe {let p=(&n) as *var f32};return 0}","E0503"),
        ("#[Copy] struct R {v:u8;w:u32} fn main():i32 {let n=R {v:0,w:0};unsafe {let p=(&n) as *f32};return 0}","E0503"),
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
        "#[Copy] enum E {A,B} fn main():i32 {let n:u32=2;unsafe {let p=(&n) as *E;let e=*p;return match e {E.A=>0,E.B=>7}}}",

        "fn main():i32 {let n:u8=2;unsafe {let p=(&n) as *bool;if *p {return 7};return 0}}",
        "fn main():i32 {var n=true;unsafe {let p=(&var n) as *var u8;*p=2;if n {return 7};return 0}}",
        "fn main():i32 {let n:u32=55296;unsafe {let p=(&n) as *char;if *p=='A' {return 7};return 0}}",
        "fn main():i32 {let n:u32=1114112;unsafe {let p=(&n) as *char;if *p=='A' {return 7};return 0}}",

        "fn main():i32 {let a=();unsafe {let p=&a+1};return 0}",
        "fn dangling():usize {let n=7;unsafe {return (&n) as usize}} fn main():i32 {unsafe {let p=dangling() as *i32;return *p}}",
        "fn main():i32 {unsafe {let a:usize=0;let p=a as *i32;return *p}}",
        "fn main():i32 {let n=7;unsafe {let a=(&n) as usize;let p=a as *var i32;*p=2;return n}}",
        "fn main():i32 {let n=7;unsafe {let a=(&n) as usize;let p=(a+1) as *i32;return *p}}",
        "fn main():i32 {let n=7;unsafe {let a=(&n) as usize;let p=(a+4) as *i32;return *p}}",
        "fn main():i32 {let n=7;unsafe {let a=(&n) as usize;let p=(a+64) as *i32;return *p}}",

        "fn main():i32 {let n:(u32,u32)=(1,2);unsafe {let p=(&n) as *{u32;3};let a=*p;return a[0] as i32}}",
        "fn main():i32 {let n:(u32,u32)=(1,2);unsafe {let p=(&n) as *u8;let q=(p+1) as *(u32,u32);let a=*q;return a.0 as i32}}",
        "fn main():i32 {let a:u64=0;unsafe {let p=(&a) as *u8;return *(p+8) as i32}}",
        "fn main():i32 {let a:u64=0;unsafe {let p=(&a) as *u8;let q=(p+1) as *u32;return *q as i32}}",
        "fn main():i32 {let a:u32=0;unsafe {let p=(&a) as *u64;return *p as i32}}",
        "fn dangling():*f32 {let n:u32=1088421888;unsafe {return (&n) as *f32}} fn main():i32 {unsafe {return *dangling() as i32}}",
        "fn main():i32 {let a:{u32;2}={0,0};unsafe {let p=(&a[0]) as *f32;return *(p+2) as i32}}",
        "fn main():i32 {let a=1;let b=2;unsafe {return (&a-&b) as i32}}",
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

#[test]
fn verifier_rejects_reinterpret_capability_and_pointee_corruption() {
    let p = checked(
        "fn main():i32 {let n:u32=1088421888;unsafe {let p=(&n) as *f32;return *p as i32}}",
    )
    .unwrap();
    let original = mir::lower(&p);
    for target in [
        hir::Type::Pointer {
            mutable: false,
            pointee: Box::new(hir::Type::Tuple(vec![
                hir::Type::Integer(nether_semantics::IntegerType::U8),
                hir::Type::Integer(nether_semantics::IntegerType::U32),
            ])),
        },
        hir::Type::Pointer {
            mutable: true,
            pointee: Box::new(hir::Type::F32),
        },
        hir::Type::Pointer {
            mutable: false,
            pointee: Box::new(hir::Type::Str),
        },
    ] {
        let mut bad = original.clone();
        let mut changed = false;
        for f in &mut bad.functions {
            for block in &mut f.blocks {
                if let mir::Terminator::Evaluate {
                    operation: mir::Operation::Cast(_, ty),
                    destination,
                    ..
                } = &mut block.terminator
                {
                    if matches!(ty, hir::Type::Pointer { .. }) {
                        *ty = target.clone();
                        f.locals[destination.local].ty = target.clone();
                        changed = true;
                    }
                }
            }
        }
        assert!(changed);
        assert!(mir::verify::verify(&bad).is_err());
        assert!(rust::emit_mir(&bad, OverflowChecks::Checked).is_err());
    }
}

#[test]
fn verifier_checks_address_cast_types_and_exposure_permissions() {
    let p =
        checked("fn main():i32 {let n=7;unsafe {let a=(&n) as usize;let p=a as *i32;return *p}}")
            .unwrap();
    let original = mir::lower(&p);
    for (opcode, replacement) in [
        ("raw_expose", "raw_expose_mut"),
        ("raw_recover", "raw_expose"),
    ] {
        let mut bad = original.clone();
        let mut changed = false;
        for f in &mut bad.functions {
            for block in &mut f.blocks {
                if let mir::Terminator::Evaluate {
                    operation: mir::Operation::Unary(op, _),
                    ..
                } = &mut block.terminator
                {
                    if *op == opcode {
                        *op = replacement;
                        changed = true;
                    }
                }
            }
        }
        assert!(changed);
        assert!(mir::verify::verify(&bad).is_err());
        assert!(rust::emit_mir(&bad, OverflowChecks::Checked).is_err());
    }
}
