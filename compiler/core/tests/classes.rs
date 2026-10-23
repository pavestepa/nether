use nether_core::{check::check, hir, interpret, mir};
use nether_frontend::{
    parser::parse,
    source::{Diagnostic, SourceId},
};
use nether_semantics::OverflowChecks;
fn checked(source: &str) -> Result<hir::Program, Vec<Diagnostic>> {
    let parsed = parse(SourceId(0), source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    check(&parsed.module.unwrap())
}
#[test]
fn constructor_initialization_follows_typed_paths_and_aliases() {
    for body in [
        "if flag {value=1} else {value=2}",
        "match flag {true=>{value=1},false=>{value=2}}",
        "let ref var alias=this;alias.value=3",
        "if flag {value=1;return};value=2",
        "value=1;while flag {value+=1;break}",
        "value=1;let h=(this,);let read=h.0.value",
    ] {
        let source=format!("class C {{value:i32;constructor(flag:bool) {{{body}}}}} fn main():i32 {{let c=new C(true);return c.value}}");
        let p = checked(&source).unwrap_or_else(|e| panic!("{source}: {e:?}"));
        let m = mir::lower(&p);
        assert_eq!(
            interpret::execute(&p, p.main.unwrap(), vec![], OverflowChecks::Checked, 10_000),
            mir::interpret::execute(&m, m.main.unwrap(), vec![], OverflowChecks::Checked, 10_000),
            "{source}"
        );
    }
    for body in [
        "let read=value;value=1",
        "value+=1",
        "if flag {value=1}",
        "while flag {value=1;break}",
        "if flag {return};value=1",
        "let value=1",
        "let ref var alias=this;let read=alias.value;value=1",
        "this.read();value=1",
        "let ref alias=this;consume(alias);value=1",
        "let h=(this,);let read=h.0.value;value=1",
        "let h=(this,);h.0.read();value=1",
        "let h=(this,);consume(h.0);value=1",
        "let h=(this,);consume_wrapper(h);value=1",
    ] {
        let source=format!("class C {{value:i32;constructor(flag:bool) {{{body}}};fn read(this):i32 {{return value}}}} fn consume(x:C):i32 {{return x.value}} fn consume_wrapper(x:(C,)):i32 {{return x.0.value}} fn main():i32 {{let c=new C(true);return c.value}}");
        let e = checked(&source).unwrap_err();
        assert!(e.iter().any(|d| d.code == "E0393"), "{source}: {e:?}");
    }
}
#[test]
fn class_copy_and_view_contracts_are_checked() {
    for source in [
        "#[Copy] class C {destructor {}}",
        "class C {constructor() {};constructor() {}}",
        "class C {value:i32} fn main():() {let c=new C()}",
        "class C {value:i32;constructor(n:i32) {value=n}} fn main():() {var a=new C(1);let ref var v=a;*v=new C(2)}",
        "class C {value:i32;constructor(n:i32) {value=n};fn bad(this):() {value=2}}",
        "class C {value:i32;constructor(n:i32) {value=n}} fn main():i32 {var a=new C(1);let ref v=a;a=new C(2);return v.value}",
        "struct R {value:i32} struct W {r:R} class C {r:R;constructor() {r=R {value:1}};destructor {}} fn main():i32 {let c=new C();let w=W {r:c.r};return w.r.value}",
        "class R {} #[Copy] class C {r:R}",
        "struct R {value:i32} fn main():() {let r=new R()}",
        "class C {} fn main():() {let a=new C();let x:i32=a==a}",
        "class C {} fn main():() {let a=new C();let x:i32=same_identity(a,a)}",
        "class C {} class D {} fn main():() {let a=new C();let b=new D();let x=a==b}",
        "class C {} fn main():() {let a=new C();let x=a<a}",
        "class C {value:i32;constructor() {let same=this==this;value=1}}",
        "class C {value:i32;constructor() {let same=same_identity(this,this);value=1}}",
        "class C {value:i32;constructor() {value=1}} fn main():() {let c=C {value:1}}",
    ] {
        assert!(checked(source).is_err(),"accepted {source}");
    }
}
#[test]
fn generic_class_arguments_infer_and_const_payloads_are_separate() {
    let source="class Cell<T> {value:T;constructor(v:T) {value=v}} fn read<T:Copy>(x:Cell<T>):T {return x.value} class Array<const N:usize> {values:{i32;N};constructor(v:{i32;N}) {values=v}} fn main():i32 {let c=new Cell<i32>(3);let a=new Array<2>({4,5});return read(c)+a.values[1]}";
    let p = checked(source).unwrap();
    let m = mir::lower(&p);
    let result = interpret::execute(&p, p.main.unwrap(), vec![], OverflowChecks::Checked, 10_000);
    assert_eq!(
        result,
        mir::interpret::execute(&m, m.main.unwrap(), vec![], OverflowChecks::Checked, 10_000)
    );
    assert!(matches!(result,Ok(hir::Value::Integer(n)) if n.bits()==8));
}
