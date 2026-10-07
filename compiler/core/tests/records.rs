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
fn copy_structs_have_nominal_types_nested_places_and_ordered_initialization() {
    let program=checked("#[Copy] struct Pair {left:i32;right:i32} #[Copy] struct Boxed {pair:Pair} fn swap(p:Pair):Pair {return Pair {right:p.left,left:p.right}} fn main():i32 {var n=0;var box=Boxed {pair:Pair {right:if true {n+=1;n} else {0},left:if true {n+=1;n} else {0}}};let copy=box;box.pair.left+=10;return swap(copy.pair).left+box.pair.left}").unwrap();
    let Value::Integer(result) = execute(
        &program,
        program.main.unwrap(),
        vec![],
        OverflowChecks::Checked,
        10000,
    )
    .unwrap() else {
        panic!()
    };
    assert_eq!(result.signed_value(), Some(13));
}
#[test]
fn rejects_recursive_incomplete_duplicate_and_structurally_similar_nominals() {
    for (source,code) in [
        ("#[Copy] struct A {a:A}","E0350"),
        ("#[Copy] struct A {b:B} #[Copy] struct B {a:{A;1}}","E0350"),
        ("#[Copy] struct A {x:i32;x:i32}","E0301"),
        ("#[Copy] struct A {x:i32} fn main():() {let a=A {}}","E0351"),
        ("#[Copy] struct A {x:i32} fn main():() {let a=A {x:1,x:2}}","E0351"),
        ("#[Copy] struct A {x:i32} fn main():() {let a=A {y:1}}","E0351"),
        ("#[Copy] struct A {x:i32} #[Copy] struct B {x:i32} fn f(a:A):() {} fn main():() {f(B {x:1})}","E0320"),
        ("#[Copy] struct A {x:i32} fn main():() {let a=A {x:1};a.x=2}","E0317"),
    ] { let errors=checked(source).unwrap_err(); assert_eq!(errors[0].code,code,"{source}: {errors:?}"); }
}
