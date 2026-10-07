use nether_core::{check::check, hir::Value, interpret::execute, llvm::emit};
use nether_frontend::{parser::parse, source::SourceId};
use nether_semantics::OverflowChecks;

#[test]
fn forward_constants_and_array_lengths_are_evaluated_before_bodies() {
    let parsed=parse(SourceId(0),"const SIZE:usize=BASE+1\nconst BASE:usize=2\nconst DATA:{i32;SIZE}={4,5,6}\nfn main():i32 { let values:{i32;SIZE}=DATA; return values[SIZE-1] }");
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let program = check(&parsed.module.unwrap()).unwrap();
    let Value::Integer(value) = execute(
        &program,
        program.main.unwrap(),
        Vec::new(),
        OverflowChecks::Wrapping,
        1000,
    )
    .unwrap() else {
        panic!()
    };
    assert_eq!(value.signed_value(), Some(6));
    assert!(emit(&program, OverflowChecks::Wrapping)
        .unwrap()
        .contains("[3 x i32]"));
}

#[test]
fn const_cycles_overflow_unknown_names_and_layout_overflow_are_rejected() {
    for source in [
        "const A:i32=B\nconst B:i32=A",
        "const A:i8=127+1",
        "const A:i8=-128 % -1",
        "const A:i32=1<<32",
        "const A:i32=1/0",
        "const A:i32=unknown",
        "const A:i32=foo()\nfn foo():i32{return 1}",
        "fn f(value:{i128;18446744073709551615}):(){ }",
        "const f:i32=1\nfn f():i32{return 1}",
    ] {
        let parsed = parse(SourceId(0), source);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        assert!(check(&parsed.module.unwrap()).is_err(), "accepted {source}");
    }
}

#[test]
fn constant_short_circuit_does_not_evaluate_unselected_failure() {
    let parsed=parse(SourceId(0),"const OK:bool=false && 1/0==0\nconst VALUE:i32=if OK {1/0} else {7}\nfn main():i32{return VALUE}");
    let program = check(&parsed.module.unwrap()).unwrap();
    let Value::Integer(value) = execute(
        &program,
        program.main.unwrap(),
        Vec::new(),
        OverflowChecks::Checked,
        1000,
    )
    .unwrap() else {
        panic!()
    };
    assert_eq!(value.signed_value(), Some(7));
}
