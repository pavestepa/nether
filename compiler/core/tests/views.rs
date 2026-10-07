use nether_core::{check::check, hir::Value, interpret::execute, mir};
use nether_frontend::{parser::parse, source::SourceId};
use nether_semantics::OverflowChecks;
fn checked(
    source: &str,
) -> Result<nether_core::hir::Program, Vec<nether_frontend::source::Diagnostic>> {
    let parsed = parse(SourceId(0), source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    check(&parsed.module.unwrap())
}
#[test]
fn safe_views_write_original_places_and_rebinding_preserves_old_target() {
    for (source, expected) in [
        ("fn main():i32 {var a=1;var b=2;let ref var x=a;*x=if true {x=b;7} else {0};return a*10+b}",72),
        ("fn main():i32 {var a=1;let ref var x=a;*x=x+2;return a}",3),

        ("#[Copy] enum E {A(i32),B} fn main():i32 {var a=E.A(3);match a {E.A(ref var x) if x>1=>{*x+=4},_=>{}};return match a {E.A(x)=>x,E.B=>0}}",7),
        ("#[Copy] enum E {A(i32),B} fn main():i32 {var a=E.A(3);match a {E.A(ref var x) if false=>{*x=99},E.A(ref var x)=>{*x+=1},_=>{}};return match a {E.A(x)=>x,E.B=>0}}",4),
        ("fn main():i32 {var a=(1,2);match a {(ref var x,_) if a.1==2=>{*x=8},_=>{}};return a.0}",8),
        ("fn main():i32 {let a=(1,2);let x=match a {(x,_)=>x};return x+a.1}",3),

        ("fn main():i32 {let pair=(1,2);var n=0;while n<2 {let (x,_)=pair;n+=x};return n}",2),
        ("fn main():i32 {var pair=(1,2);let (x,_)=pair;pair.1=9;return x+pair.1}",10),

        ("fn main():i32 {var a={1,2};let {ref var x,..}=a;*x=7;*x+=1;return a[0]}",8),
        ("fn main():i32 {var a=1;var b=2;let ref var x=a;*x=3;x=b;*x=7;return a*10+b}",37),
        ("fn main():i32 {var a=(1,2);let (ref var x,ref var y)=a;*x+=3;*y+=5;return a.0+a.1}",11),
        ("fn main():i32 {var a={1,2};var i:usize=0;let ref var x=a[i];i=1;*x=8;return a[0]+a[1]}",10),
        ("fn main():i32 {var a=1;let ref var parent=a;let ref child=parent;let copy=child;*parent=5;return a+copy}",6),
        ("fn main():i32 {var a=1;let ref x=a;let copy=x;a=4;return copy+a}",5),
        ("fn main():i32 {var a=1;let ref var x=a;var n=0;while n<3 {*x+=1;n+=1};return a}",4),
    ] {
        let hir = checked(source).unwrap_or_else(|e| panic!("{source}: {e:?}"));
        let cfg = mir::lower(&hir);
        for f in &cfg.functions { assert!(mir::dataflow::initialization_errors(f).is_empty(), "{source}"); }
        let a = execute(&hir, hir.main.unwrap(), vec![], OverflowChecks::Checked, 10000).unwrap();
        let b = mir::interpret::execute(&cfg, cfg.main.unwrap(), vec![], OverflowChecks::Checked, 10000).unwrap();
        assert_eq!(a,b,"{source}");
        let Value::Integer(a) = a else { panic!() };
        assert_eq!(a.signed_value(), Some(expected),"{source}");
        nether_core::llvm::emit(&hir, OverflowChecks::Checked).unwrap();
    }
}
#[test]
fn views_reject_aliasing_readonly_writes_and_loop_carried_conflicts() {
    for source in [
        "fn main():i32 {var a=0;let ref var out=a;match 1 {var z=>{out=z}};return out}",

        "fn main():i32 {var a=1;let ref var x=a;**x=4;return a}",

        "#[Copy] enum E {A(i32),B} fn main():i32 {var a=E.A(3);return match a {E.A(ref var x)=>{a=E.B;x},E.B=>0}}",
        "fn main():i32 {var a=(1,2);return match a {(ref var x,_) if if true {*x=4;true} else {false}=>x,_=>0}}",

        "fn main():i32 {var a=1;let ref var x=a;let ref var y=x;var n=0;while n<2 {x=y;y=x;n+=1};*x=4;return y}",

        "fn main():i32 {var a=1;let ref var x=a;if true {var b=2;x=b};return x}",
        "fn main():i32 {var a=1;let ref var x=a;while true {var b=2;x=b;break};return x}",
        "fn main():i32 {var a=1;let ref var x=a;var n=0;while n<2 {var b=2;x=b;n+=1;continue};return x}",
        "fn main():i32 {var a=1;let ref x=a;*x=2;return a}",
        "fn main():i32 {let a=1;let ref var x=a;return x}",
        "fn main():i32 {var a=1;let ref x=a;a=2;return x}",
        "fn main():i32 {var a=1;let ref var x=a;let b=a;return x+b}",
        "fn main():i32 {var a=1;let ref var x=a;let ref var y=a;return x+y}",
        "fn main():i32 {var a=1;let ref var x=a;let ref child=x;*x=2;return child}",
        "fn main():i32 {var a=1;let ref x=a;var n=0;while n<3 {n+=x;a+=1};return n}",
        "fn main():i32 {var a={1,2};var i:usize=0;let ref var x=a[i];let ref var y=a[1];*x=3;return y}",
        "fn main():i32 {var a=1;let ref var x=a;x+=2;return a}",
    ] { assert!(checked(source).is_err(), "accepted: {source}"); }
}
