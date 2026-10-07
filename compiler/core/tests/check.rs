use nether_core::{check::check, hir};
use nether_frontend::{parser::parse, source::SourceId};

fn checked(text: &str) -> Result<hir::Program, Vec<String>> {
    let parsed = parse(SourceId(0), text);
    if !parsed.diagnostics.is_empty() {
        return Err(parsed
            .diagnostics
            .into_iter()
            .map(|d| format!("{}: {}", d.code, d.message))
            .collect());
    }
    check(&parsed.module.unwrap()).map_err(|errors| {
        errors
            .into_iter()
            .map(|d| format!("{}: {}", d.code, d.message))
            .collect()
    })
}

#[test]
fn checks_forward_recursive_signatures_and_control_flow() {
    let program=checked("fn main(): i32 { return factorial(5) }\nfn factorial(n:i32):i32 { if n == 0 { return 1 } else { return n * factorial(n-1) } }").unwrap();
    assert_eq!(program.functions.len(), 2);
    assert_eq!(program.main, Some(0));
    assert!(program.functions[1].body.diverges);
}

#[test]
fn checks_arrays_tuples_scopes_and_mutable_places() {
    checked("fn main(): i32 { var values:{i32;3}={1,2,3}; var i:usize=0; var sum=0; while i<3 { sum += values[i]; i+=1 }; var pair=(sum,true); pair.0 += 1; return pair.0 }").unwrap();
    checked("fn f(): {i32;0} { return {} }").unwrap();
    checked("fn f():i32 { let x=1; if true {let x=2; x}; return x }").unwrap();
}

#[test]
fn rejects_type_and_scope_violations_before_backend() {
    for text in [
        "fn f():i32 { 1 }",
        "fn f():i32 { return\n1 }",
        "fn f():i32 { if true {return 1} }",
        "fn f():() { break }",
        "fn f():() { let x=1; x=2 }",
        "fn f():() { let a={1,2}; a[0]=3 }",
        "fn f():() { if 1 {} }",
        "fn f():i32 { return missing }",
        "fn f():i32 { if true {let x=1}; return x }",
        "fn f():() { let x:u8=256 }",
        "fn f():() { let x:i8=-129 }",
        "fn f():() { let x=1 as bool }",
        "fn f():() { let a={}; }",
        "fn f():() { let x:(i32,bool)=(1,2) }",
        "fn main(x:i32):i32 { return x }",
        "fn f(x:i32,x:i32):i32 { return x }",
        "fn g():i32 {return 1} fn main():i32 {let g=2;return g()}",
    ] {
        assert!(checked(text).is_err(), "accepted {text}");
    }
}

#[test]
fn unsupported_ownership_forms_are_not_silently_accepted() {
    let errors =
        checked("class Resource { destructor {} }\nfn main():() { let value=new Resource() }")
            .unwrap_err();
    assert!(errors.iter().any(|e| e.starts_with("E0900")));
}

#[test]
fn signed_minimum_is_representable_without_positive_intermediate() {
    checked("fn f():i8 { return -128 }").unwrap();
    checked("fn f():i128 { return -170141183460469231731687303715884105728 }").unwrap();
    assert!(checked("fn f():i128 { return 170141183460469231731687303715884105728 }").is_err());
}

#[test]
fn literal_context_can_come_from_the_other_operand() {
    checked("fn f(value:u8):bool {return 1 < value}").unwrap();
    checked("fn f(value:u8):bool {return 1 as u8 < value}").unwrap();
}
