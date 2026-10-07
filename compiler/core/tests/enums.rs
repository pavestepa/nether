use nether_core::{check::check, hir::Value, interpret::execute};
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
fn enum_payloads_are_read_only_on_matching_paths() {
    for (source,expected) in [
  ("#[Copy] enum E {None,Some(i32)} fn main():i32 {let a=E.None;return match a {E.Some(x)=>x,E.None=>7}}",7),
  ("#[Copy] enum E {None,Some(i32)} fn main():i32 {let a=E.Some(13);return match a {E.Some(x) if x>20=>1,E.Some(x)=>x,E.None=>7}}",13),
  ("#[Copy] enum E {A {x:i32,y:i32},B(u128),C} fn main():i32 {var n=0;let a=E.A {y:if true {n+=1;n} else {0},x:if true {n+=1;n} else {0}};return match a {E.A {x,y}=>x*10+y,E.B(x)=>x as i32,E.C=>0}}",21),
  ("#[Copy] enum Inner {A(i32),B} #[Copy] enum Outer {Some(Inner),None} fn main():i32 {let x=Outer.Some(Inner.B);return match x {Outer.Some(Inner.A(v))=>v,Outer.Some(_)=>2,Outer.None=>3}}",2),
  ("#[Copy] enum E {A,B(i32)} fn f(x:E):E {return x} fn main():i32 {var x=E.A;x=f(E.B(9));return match x {E.A=>0,E.B(v)=>v}}",9),
 ] {let p=checked(source).unwrap();let Value::Integer(result)=execute(&p,p.main.unwrap(),vec![],OverflowChecks::Checked,10000).unwrap() else {panic!()};assert_eq!(result.signed_value(),Some(expected));}
}
#[test]
fn enum_shape_and_exhaustiveness_are_checked() {
    for (source, code) in [
        ("#[Copy] enum E {A,A}", "E0301"),
        ("#[Copy] enum E {A(E)}", "E0350"),
        (
            "#[Copy] enum E {A(i32),B} fn main():i32 {return match E.B {E.A(x)=>x}}",
            "E0340",
        ),
        (
            "#[Copy] enum E {A(i32),B} fn main():i32 {return match E.B {E.A(1)=>1,E.B=>0}}",
            "E0340",
        ),
        ("#[Copy] enum E {A(i32)} fn main():() {let x=E.A}", "E0360"),
        (
            "#[Copy] enum E {A(i32)} fn main():() {let x=E.A(true)}",
            "E0320",
        ),
        (
            "#[Copy] enum E {A(i32)} fn main():() {let x=E.A()}",
            "E0360",
        ),
        (
            "#[Copy] enum E {A(i32)} fn main():i32 {return match E.A(1) {E.A=>1}}",
            "E0342",
        ),
    ] {
        let e = checked(source).unwrap_err();
        assert_eq!(e[0].code, code, "{source}: {e:?}");
    }
}
