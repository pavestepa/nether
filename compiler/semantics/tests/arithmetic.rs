use nether_semantics::{
    ArithmeticError as Error, BinaryOp as Op, Integer as Int, IntegerType as Ty,
    OverflowChecks as Checks,
};

fn signed(ty: Ty, value: i128) -> Int {
    Int::literal(ty, value.unsigned_abs(), value < 0).unwrap()
}

#[test]
fn exhaustive_eight_bit_arithmetic_against_wider_mathematical_values() {
    for ty in [Ty::I8, Ty::U8] {
        let (min, max) = if ty.is_signed() {
            (-128_i128, 127_i128)
        } else {
            (0, 255)
        };
        for a in min..=max {
            for b in min..=max {
                let (left, right) = (signed(ty, a), signed(ty, b));
                for (op, exact) in [
                    (Op::Add, a + b),
                    (Op::Subtract, a - b),
                    (Op::Multiply, a * b),
                ] {
                    let expected = if (min..=max).contains(&exact) {
                        Ok(Int::from_bits(ty, exact as u128))
                    } else {
                        Err(Error::Overflow)
                    };
                    assert_eq!(
                        left.binary(op, right, Checks::Checked),
                        expected,
                        "{ty:?}: {a} {op:?} {b}"
                    );
                    assert_eq!(
                        left.binary(op, right, Checks::Wrapping).unwrap().bits(),
                        (exact as u128) & 255
                    );
                }
                for op in [Op::Divide, Op::Remainder] {
                    let expected = if b == 0 {
                        Err(Error::DivisionByZero)
                    } else if ty.is_signed() && a == min && b == -1 {
                        Err(Error::Overflow)
                    } else {
                        Ok(Int::from_bits(
                            ty,
                            if op == Op::Divide { a / b } else { a % b } as u128,
                        ))
                    };
                    for checks in [Checks::Checked, Checks::Wrapping] {
                        assert_eq!(left.binary(op, right, checks), expected);
                    }
                }
                assert_eq!(left.compare(right).unwrap(), a.cmp(&b));
                for (op, expected) in [(Op::BitAnd, a & b), (Op::BitOr, a | b), (Op::BitXor, a ^ b)]
                {
                    assert_eq!(
                        left.const_binary(op, right).unwrap().bits(),
                        (expected as u128) & 255
                    );
                }
            }
        }
    }
}

#[test]
fn every_integer_width_checks_boundaries_without_host_overflow() {
    for ty in Ty::ALL {
        let one = Int::literal(ty, 1, false).unwrap();
        let zero = Int::literal(ty, 0, false).unwrap();
        let max = Int::from_bits(
            ty,
            if ty.is_signed() {
                ty.mask() >> 1
            } else {
                ty.mask()
            },
        );
        assert_eq!(max.const_binary(Op::Add, one), Err(Error::Overflow));
        assert_eq!(
            max.const_binary(Op::Multiply, signed(ty, 2)),
            Err(Error::Overflow)
        );
        assert_eq!(
            max.binary(Op::Add, one, Checks::Wrapping).unwrap().bits(),
            if ty.is_signed() {
                1 << (ty.bits() - 1)
            } else {
                0
            }
        );
        assert_eq!(
            max.binary(Op::Divide, zero, Checks::Wrapping),
            Err(Error::DivisionByZero)
        );
        assert_eq!(
            max.binary(Op::Remainder, zero, Checks::Wrapping),
            Err(Error::DivisionByZero)
        );
        if ty.is_signed() {
            let min = Int::from_bits(ty, 1_u128 << (ty.bits() - 1));
            let minus_one = signed(ty, -1);
            for checks in [Checks::Checked, Checks::Wrapping] {
                assert_eq!(
                    min.binary(Op::Divide, minus_one, checks),
                    Err(Error::Overflow)
                );
                assert_eq!(
                    min.binary(Op::Remainder, minus_one, checks),
                    Err(Error::Overflow)
                );
            }
            assert_eq!(min.const_binary(Op::Subtract, one), Err(Error::Overflow));
            assert_eq!(min.negate(Checks::Checked), Err(Error::Overflow));
            assert_eq!(min.negate(Checks::Wrapping), Ok(min));
            assert_eq!(Int::literal(ty, min.bits(), true), Ok(min));
            assert_eq!(
                Int::literal(ty, min.bits(), false),
                Err(Error::LiteralOutOfRange)
            );
        } else {
            assert_eq!(zero.const_binary(Op::Subtract, one), Err(Error::Overflow));
            assert_eq!(one.negate(Checks::Wrapping), Err(Error::UnsignedNegation));
            assert_eq!(Int::literal(ty, 0, true), Err(Error::LiteralOutOfRange));
        }
        if ty.bits() < 128 {
            assert_eq!(
                Int::literal(ty, ty.mask() + 1, false),
                Err(Error::LiteralOutOfRange)
            );
        }
    }
}

#[test]
fn shifts_check_counts_and_mask_only_in_wrapping_mode() {
    for ty in Ty::ALL {
        let one = signed(ty, 1);
        for count_ty in Ty::ALL {
            let width = signed(count_ty, i128::from(ty.bits().min(127)));
            // I8 cannot represent positive 128. Test it through all wider count types.
            let width = if ty.bits() == 128 && count_ty == Ty::I8 {
                None
            } else {
                Some(if ty.bits() == 128 {
                    signed(count_ty, 128)
                } else {
                    width
                })
            };
            if let Some(width) = width {
                for op in [Op::ShiftLeft, Op::ShiftRight] {
                    assert_eq!(
                        one.binary(op, width, Checks::Checked),
                        Err(Error::InvalidShift)
                    );
                    assert_eq!(one.binary(op, width, Checks::Wrapping), Ok(one));
                }
            }
        }
        let negative = signed(Ty::I128, -1);
        assert_eq!(
            one.const_binary(Op::ShiftLeft, negative),
            Err(Error::InvalidShift)
        );
        assert_eq!(
            one.binary(Op::ShiftLeft, negative, Checks::Wrapping)
                .unwrap()
                .bits(),
            1 << (ty.bits() - 1)
        );
        let last = signed(Ty::Usize, i128::from(ty.bits() - 1));
        assert_eq!(
            one.const_binary(Op::ShiftLeft, last).unwrap().bits(),
            1 << (ty.bits() - 1)
        );
        let all = Int::from_bits(ty, ty.mask());
        assert_eq!(
            all.const_binary(Op::ShiftRight, signed(Ty::U8, 1))
                .unwrap()
                .bits(),
            if ty.is_signed() {
                ty.mask()
            } else {
                ty.mask() >> 1
            }
        );
        assert_eq!(all.bit_not().bits(), 0);
        assert_eq!(
            one.const_binary(Op::ShiftLeft, Int::from_bits(Ty::U128, u128::MAX)),
            Err(Error::InvalidShift)
        );
    }
}

#[test]
fn casts_preserve_sign_before_destination_truncation() {
    assert_eq!(signed(Ty::I8, -1).cast(Ty::U128).bits(), u128::MAX);
    assert_eq!(signed(Ty::U8, 255).cast(Ty::I16).signed_value(), Some(255));
    assert_eq!(signed(Ty::I16, -129).cast(Ty::I8).signed_value(), Some(127));
    assert_eq!(
        Int::from_bits(Ty::U128, u128::MAX)
            .cast(Ty::I128)
            .signed_value(),
        Some(-1)
    );
    for ty in Ty::ALL {
        let max = Int::from_bits(ty, ty.mask());
        assert_eq!(max.cast(ty), max);
    }
}

#[test]
fn float_casts_saturate_instead_of_producing_poison() {
    for ty in Ty::ALL {
        assert_eq!(Int::from_float(ty, f64::NAN).bits(), 0);
        assert_eq!(Int::from_float(ty, -0.0).bits(), 0);
        assert_eq!(Int::from_float(ty, 1.99).bits(), 1);
        let max = if ty.is_signed() {
            ty.mask() >> 1
        } else {
            ty.mask()
        };
        assert_eq!(Int::from_float(ty, f64::INFINITY).bits(), max);
        assert_eq!(
            Int::from_float(ty, f64::NEG_INFINITY).bits(),
            if ty.is_signed() {
                1 << (ty.bits() - 1)
            } else {
                0
            }
        );
    }
    assert_eq!(Int::from_float(Ty::I8, -128.9).signed_value(), Some(-128));
    assert_eq!(Int::from_float(Ty::U8, -1.0).bits(), 0);
    assert_eq!(Int::from_float(Ty::U8, 256.0).bits(), 255);
    assert_eq!(
        Int::from_float(Ty::I128, 2_f64.powi(127)).signed_value(),
        Some(i128::MAX)
    );
    assert_eq!(Int::from_float(Ty::U128, 2_f64.powi(128)).bits(), u128::MAX);
}

#[test]
fn integer_types_remain_distinct_even_with_equal_width() {
    let a = signed(Ty::Usize, 1);
    let b = signed(Ty::U64, 1);
    assert_eq!(a.const_binary(Op::Add, b), Err(Error::TypeMismatch));
    assert_eq!(a.compare(b), Err(Error::TypeMismatch));
    assert_eq!(Ty::Usize.bits(), 64);
    assert_eq!(Ty::Isize.bits(), 64);
}

#[test]
fn exhaustive_eight_bit_shifts_and_negation() {
    for raw in 0_u128..=255 {
        for ty in [Ty::I8, Ty::U8] {
            let left = Int::from_bits(ty, raw);
            for count in -128_i128..=255 {
                let right = signed(Ty::I16, count);
                let reduced = count.rem_euclid(8) as u32;
                let expected_left = ((raw as u16) * 2_u16.pow(reduced)) as u8;
                // Wider signed host arithmetic is independent of the oracle's
                // normalized u128 representation and always fits here.
                let expected_right = if ty.is_signed() {
                    ((raw as u8 as i8 as i16) >> reduced) as u8
                } else {
                    ((raw as u16) / 2_u16.pow(reduced)) as u8
                };
                for (op, expected) in [
                    (Op::ShiftLeft, expected_left),
                    (Op::ShiftRight, expected_right),
                ] {
                    assert_eq!(
                        left.binary(op, right, Checks::Wrapping).unwrap().bits(),
                        u128::from(expected)
                    );
                    let checked = left.const_binary(op, right);
                    if !(0..8).contains(&count) {
                        assert_eq!(checked, Err(Error::InvalidShift));
                    } else {
                        assert_eq!(checked.unwrap().bits(), u128::from(expected));
                    }
                }
            }
        }
        let left = Int::from_bits(Ty::I8, raw);
        let wide = -(raw as u8 as i8 as i16);
        assert_eq!(
            left.negate(Checks::Wrapping).unwrap().bits(),
            u128::from(wide as u8)
        );
        if raw == 128 {
            assert_eq!(left.negate(Checks::Checked), Err(Error::Overflow));
        } else {
            assert_eq!(
                left.negate(Checks::Checked).unwrap().signed_value(),
                Some(i128::from(wide))
            );
        }
    }
}

#[test]
fn float_casts_at_large_integer_boundaries() {
    // The representable f64 directly below each upper boundary must not be
    // mistaken for the boundary itself (integer maxima round upwards as f64).
    for (ty, exponent) in [
        (Ty::I64, 63),
        (Ty::U64, 64),
        (Ty::I128, 127),
        (Ty::U128, 128),
    ] {
        let boundary = 2_f64.powi(exponent);
        let below = f64::from_bits(boundary.to_bits() - 1);
        let expected = if exponent == 128 {
            u128::MAX - ((1_u128 << 75) - 1)
        } else {
            (1_u128 << exponent) - (1_u128 << (exponent - 53))
        };
        assert_eq!(Int::from_float(ty, below).bits(), expected);
        assert_eq!(
            Int::from_float(ty, boundary).bits(),
            if ty.is_signed() {
                ty.mask() >> 1
            } else {
                ty.mask()
            }
        );
    }
}
