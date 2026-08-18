use super::*;

#[test]
fn const_generic_sizes_a_fixed_array() {
    assert_ok(
        r#"
fn consume<const N usize>(values {i32, N}) i32 { return 7; }
fn main() { let result i32 = consume<3>([1, 2, 3]); }
"#,
    );
}

#[test]
fn const_parameter_is_a_compile_time_value() {
    assert_ok(
        r#"
fn length<const N usize>() usize { return N; }
fn main() { let result usize = length<9>(); }
"#,
    );
}

#[test]
fn fixed_array_length_mismatch_is_rejected() {
    assert_err(
        r#"
fn consume<const N usize>(values {i32, N}) i32 { return 7; }
fn main() { consume<3>([1, 2]); }
"#,
        "fixed array expects 3 element(s), found 2",
    );
}

#[test]
fn const_and_type_generic_arguments_are_distinct() {
    assert_err(
        r#"
fn consume<const N usize>() {}
fn main() { consume<i32>(); }
"#,
        "requires an integer constant",
    );
    assert_err(
        r#"
fn consume<T>() {}
fn main() { consume<3>(); }
"#,
        "requires a type, not a constant",
    );
}

#[test]
fn const_parameter_type_must_be_integer() {
    assert_err(
        r#"
fn consume<const N String>() {}
fn main() {}
"#,
        "must have an integer type",
    );
}
