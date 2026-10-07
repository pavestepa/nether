use nether_core::{check::check, hir::Value, interpret::execute};
use nether_frontend::{parser::parse, source::SourceId};
use nether_semantics::OverflowChecks;

fn program(source: &str) -> nether_frontend::ast::Module {
    let parsed = parse(SourceId(0), source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    parsed.module.unwrap()
}
#[test]
fn matches_evaluate_once_and_keep_arm_bindings_scoped() {
    for (source, expected) in [
        ("fn main():i32 {var n=0;let v=match if true {n+=1;n} else {0} {0=>10,x if x>2=>20,x=>x+30};return v+n}",32),
        ("fn main():i32 {return match (3,true) {(x,false)=>x, (x,true) if x>4=>9, (x,_)=>x+10}}",13),
        ("fn main():i32 {return match {1,2,3,4} {{x,..,y}=>x+y}}",5),
        ("fn main():i32 {match true {false=>{return 4},true=>{return 7}}}",7),
        ("fn main():i32 {return match -3 {-3=>8,_=>9}}",8),
        ("fn main():i32 {return match 'é' {'é'=>1,_=>2}}",1),
    ] {
        let checked=check(&program(source)).unwrap();
        let result=execute(&checked,checked.main.unwrap(),vec![],OverflowChecks::Checked,10000).unwrap();
        let Value::Integer(result)=result else {panic!("integer expected")};
        assert_eq!(result.signed_value(),Some(expected));
    }
}
#[test]
fn rejects_nonexhaustive_illtyped_and_mutating_patterns() {
    for (source, code) in [
        ("fn main():i32 {return match true {true=>1}}", "E0340"),
        ("fn main():i32 {return match 1 {1=>1}}", "E0340"),
        ("fn main():i32 {return match 1 {_ if true=>1}}", "E0340"),
        ("fn main():i32 {return match 1 {_=>1,_=>2}}", "E0341"),
        ("fn main():i32 {return match (1,2) {(x,x)=>1}}", "E0301"),
        ("fn main():i32 {return match {1,2} {{x}=>x}}", "E0342"),
        ("fn main():i32 {var n=0;return match 1 {x if if true {n+=1;true} else {false}=>1,_=>0}}", "E0343"),
        ("fn main():i32 {let a=match 1 {x=>x};return x}", "E0310"),
        ("fn main():i32 {return match true {true=>1,false=>false}}", "E0320"),
    ] {
        let errors=check(&program(source)).unwrap_err();
        assert_eq!(errors[0].code,code,"{source}: {errors:?}");
    }
}

#[test]
fn irrefutable_bindings_and_parameters_destructure_copy_values() {
    let source="#[Copy] struct Pair {x:i32;y:i32} fn sum((a,b):(i32,i32)):i32 {return a+b} fn main():i32 {let pair=Pair {x:2,y:3};var Pair {x,y}=pair;x+=4;let {a,..,b}={1,2,3};return sum((x,y))+a+b}";
    let checked = check(&program(source)).unwrap();
    let Value::Integer(value) = execute(
        &checked,
        checked.main.unwrap(),
        vec![],
        OverflowChecks::Checked,
        10000,
    )
    .unwrap() else {
        panic!()
    };
    assert_eq!(value.signed_value(), Some(13));
    for source in [
        "fn f((a,a):(i32,i32)):() {}",
        "fn f(a:i32,a:i32):() {}",
        "fn main():() {let (1,x)=(1,2)}",
        "private fn main():() {}",
    ] {
        assert!(check(&program(source)).is_err(), "accepted {source}");
    }
}
#[test]
fn guards_can_call_checked_value_functions_without_mutating_inputs() {
    let p=check(&program("fn predicate(var x:i32):bool {x+=1;return x==4} fn main():i32 {let n=3;return match n {x if predicate(x)=>x,_=>0}} ")).unwrap();
    let Value::Integer(value) =
        execute(&p, p.main.unwrap(), vec![], OverflowChecks::Checked, 10000).unwrap()
    else {
        panic!()
    };
    assert_eq!(value.signed_value(), Some(3));
}
#[test]
fn extraction_does_not_silently_copy_when_future_use_requires_a_view() {
    for source in [
        "fn main():i32 {var pair=(1,2);let (x,_)=pair;pair.0=3;return x}",
        "fn main():i32 {var pair=(1,2);return match pair {(x,_)=>{pair.0=3;x}}}",
    ] {
        let errors = check(&program(source)).unwrap_err();
        assert!(["E0380", "E0381"].contains(&errors[0].code));
    }
}
