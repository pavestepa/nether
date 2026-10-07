use nether_core::{
    check::check,
    hir::Value,
    interpret::{execute, TrapKind},
};
use nether_frontend::{parser::parse, source::SourceId};
use nether_semantics::{ArithmeticError, OverflowChecks};

fn run(text: &str, checks: OverflowChecks) -> Result<Value, TrapKind> {
    let parsed = parse(SourceId(0), text);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let program = check(&parsed.module.unwrap()).unwrap();
    execute(&program, program.main.unwrap(), Vec::new(), checks, 100_000).map_err(|e| e.kind)
}
fn number(value: Value) -> i128 {
    let Value::Integer(value) = value else {
        panic!("integer expected")
    };
    value.signed_value().unwrap()
}

#[test]
fn executes_recursive_calls_and_fixed_arrays() {
    assert_eq!(
        number(
            run(
                include_str!("../../../examples/arithmetic/main.nr"),
                OverflowChecks::Checked
            )
            .unwrap()
        ),
        130
    );
}
#[test]
fn loops_propagate_return_break_and_continue_correctly() {
    let text="fn main(): i32 {var i=0;var sum=0;while i<10 {i+=1;if i==3 {continue};if i==6 {break};sum+=i};return sum}";
    assert_eq!(number(run(text, OverflowChecks::Checked).unwrap()), 12);
    assert_eq!(
        number(
            run(
                "fn main():i32 { while true { if true {return 7} }; return 0 }",
                OverflowChecks::Checked
            )
            .unwrap()
        ),
        7
    );
}
#[test]
fn checked_wrapping_and_mandatory_division_failures_match_contracts() {
    let text = "fn main():i32 {let x:i8=127;return (x+1) as i32}";
    assert_eq!(
        run(text, OverflowChecks::Checked),
        Err(TrapKind::Arithmetic(ArithmeticError::Overflow))
    );
    assert_eq!(number(run(text, OverflowChecks::Wrapping).unwrap()), -128);
    for checks in [OverflowChecks::Checked, OverflowChecks::Wrapping] {
        assert_eq!(
            run("fn main():i32 {return 1/0}", checks),
            Err(TrapKind::Arithmetic(ArithmeticError::DivisionByZero))
        );
        assert_eq!(
            run(
                "fn main():i32 {let x:i8=-128;return (x % -1) as i32}",
                checks
            ),
            Err(TrapKind::Arithmetic(ArithmeticError::Overflow))
        );
    }
}
#[test]
fn short_circuit_skips_trapping_rhs() {
    for expression in ["false && 1/0==0", "true || 1/0==0"] {
        let text = format!("fn main():i32 {{return if {expression} {{1}} else {{0}}}}");
        assert!(run(&text, OverflowChecks::Checked).is_ok());
    }
}
#[test]
fn assignment_places_are_evaluated_once_before_rhs() {
    let text="fn main():i32 {var a={1,2};var index:usize=0;a[if true {index+=1;index} else {0}] += 10;return a[1]+index as i32}";
    assert_eq!(number(run(text, OverflowChecks::Checked).unwrap()), 13);
    let bad = "fn main():i32 {var a={1};a[2] = 1/0;return a[0]}";
    assert_eq!(run(bad, OverflowChecks::Checked), Err(TrapKind::Bounds));
}
#[test]
fn float_casts_and_nan_comparisons_are_defined() {
    assert_eq!(
        number(
            run(
                "fn main():i32 {return (0.0/0.0) as i32}",
                OverflowChecks::Checked
            )
            .unwrap()
        ),
        0
    );
    assert_eq!(
        number(
            run(
                "fn main():i32 {return (1.0/0.0) as i32}",
                OverflowChecks::Checked
            )
            .unwrap()
        ),
        i32::MAX as i128
    );
    assert_eq!(
        number(
            run(
                "fn main():i32 {let nan=0.0/0.0;return if nan!=nan {1} else {0}}",
                OverflowChecks::Checked
            )
            .unwrap()
        ),
        1
    );
}
#[test]
fn runaway_execution_is_bounded_without_host_stack_overflow() {
    assert_eq!(
        run("fn main():i32 {return main()}", OverflowChecks::Checked),
        Err(TrapKind::CallDepth)
    );
    assert_eq!(
        run(
            "fn main():i32 {while true {};return 0}",
            OverflowChecks::Checked
        ),
        Err(TrapKind::StepLimit)
    );
}
