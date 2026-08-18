use super::*;

#[test]
fn associated_constants_work_for_inherent_and_trait_impls() {
    assert_ok(
        r#"
trait SizedValue {
    pub const SIZE u64;
    const FALLBACK i32 = 7;
}
struct Packet;
impl Packet SizedValue {
    pub const SIZE u64 = 12;
}
impl Packet {
    pub const TAG i32 = 3;
}
fn main() {
    let size u64 = Packet.SIZE;
    let fallback i32 = Packet.FALLBACK;
    let tag i32 = Packet.TAG;
}
"#,
    );
}

#[test]
fn missing_required_associated_constant_is_rejected() {
    assert_err(
        r#"
trait SizedValue { const SIZE u64; }
struct Packet;
impl Packet SizedValue {}
fn main() {}
"#,
        "missing associated constant `SIZE`",
    );
}

#[test]
fn associated_constant_type_mismatch_is_rejected() {
    assert_err(
        r#"
trait SizedValue { const SIZE u64; }
struct Packet;
impl Packet SizedValue { const SIZE i32 = 1; }
fn main() {}
"#,
        "has type `i32`, expected `u64`",
    );
}

#[test]
fn ambiguous_generic_associated_constant_is_rejected() {
    assert_err(
        r#"
trait Left { const VALUE i32 = 1; }
trait Right { const VALUE i32 = 2; }
fn value<T Left + Right>() i32 { return T.VALUE; }
fn main() {}
"#,
        "is ambiguous across bounds",
    );
}
