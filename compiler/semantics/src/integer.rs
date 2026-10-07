use crate::TARGET_POINTER_BITS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntegerType {
    I8,
    I16,
    I32,
    I64,
    I128,
    Isize,
    U8,
    U16,
    U32,
    U64,
    U128,
    Usize,
}

impl IntegerType {
    pub const ALL: [Self; 12] = [
        Self::I8,
        Self::I16,
        Self::I32,
        Self::I64,
        Self::I128,
        Self::Isize,
        Self::U8,
        Self::U16,
        Self::U32,
        Self::U64,
        Self::U128,
        Self::Usize,
    ];

    pub const fn bits(self) -> u32 {
        match self {
            Self::I8 | Self::U8 => 8,
            Self::I16 | Self::U16 => 16,
            Self::I32 | Self::U32 => 32,
            Self::I64 | Self::U64 => 64,
            Self::I128 | Self::U128 => 128,
            Self::Isize | Self::Usize => TARGET_POINTER_BITS,
        }
    }

    pub const fn is_signed(self) -> bool {
        matches!(
            self,
            Self::I8 | Self::I16 | Self::I32 | Self::I64 | Self::I128 | Self::Isize
        )
    }

    pub const fn mask(self) -> u128 {
        u128::MAX >> (128 - self.bits())
    }

    fn signed_min(self) -> i128 {
        i128::MIN >> (128 - self.bits())
    }

    fn signed_max(self) -> i128 {
        i128::MAX >> (128 - self.bits())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverflowChecks {
    /// Debug default and mandatory for constant evaluation.
    Checked,
    /// Release default; division faults remain errors.
    Wrapping,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    BitAnd,
    BitOr,
    BitXor,
    ShiftLeft,
    ShiftRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArithmeticError {
    TypeMismatch,
    LiteralOutOfRange,
    Overflow,
    DivisionByZero,
    InvalidShift,
    UnsignedNegation,
}

/// A typed, normalized two's-complement bit pattern, including all u128 values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Integer {
    ty: IntegerType,
    raw: u128,
}

impl Integer {
    /// Constructs a bit pattern, not a source literal. High bits are truncated.
    pub fn from_bits(ty: IntegerType, raw: u128) -> Self {
        Self {
            ty,
            raw: raw & ty.mask(),
        }
    }

    /// Checks the magnitude before narrowing. `negative` handles signed MIN
    /// without requiring its positive magnitude to fit the signed type.
    pub fn literal(
        ty: IntegerType,
        magnitude: u128,
        negative: bool,
    ) -> Result<Self, ArithmeticError> {
        let limit = if ty.is_signed() {
            ty.signed_max() as u128 + u128::from(negative)
        } else {
            ty.mask()
        };
        if magnitude > limit || (negative && !ty.is_signed()) {
            return Err(ArithmeticError::LiteralOutOfRange);
        }
        Ok(Self::from_bits(
            ty,
            if negative {
                magnitude.wrapping_neg()
            } else {
                magnitude
            },
        ))
    }

    pub fn ty(self) -> IntegerType {
        self.ty
    }
    pub fn bits(self) -> u128 {
        self.raw
    }

    /// Returns the mathematical signed value only for signed types.
    pub fn signed_value(self) -> Option<i128> {
        self.ty.is_signed().then(|| self.sign_extended())
    }

    fn sign_extended(self) -> i128 {
        let shift = 128 - self.ty.bits();
        ((self.raw << shift) as i128) >> shift
    }

    /// Numeric `as`: sign/zero extension followed by truncation to destination.
    pub fn cast(self, destination: IntegerType) -> Self {
        let raw = if self.ty.is_signed() {
            self.sign_extended() as u128
        } else {
            self.raw
        };
        Self::from_bits(destination, raw)
    }

    /// Saturating float-to-int cast: truncate toward zero, NaN -> 0.
    /// An f32 source is widened exactly before entering this function.
    pub fn from_float(destination: IntegerType, value: f64) -> Self {
        if destination.is_signed() {
            let clamped = (value as i128).clamp(destination.signed_min(), destination.signed_max());
            Self::from_bits(destination, clamped as u128)
        } else {
            Self::from_bits(destination, (value as u128).min(destination.mask()))
        }
    }

    pub fn negate(self, checks: OverflowChecks) -> Result<Self, ArithmeticError> {
        if !self.ty.is_signed() {
            return Err(ArithmeticError::UnsignedNegation);
        }
        if checks == OverflowChecks::Checked && self.sign_extended() == self.ty.signed_min() {
            return Err(ArithmeticError::Overflow);
        }
        Ok(Self::from_bits(self.ty, self.raw.wrapping_neg()))
    }

    pub fn bit_not(self) -> Self {
        Self::from_bits(self.ty, !self.raw)
    }

    pub fn compare(self, rhs: Self) -> Result<std::cmp::Ordering, ArithmeticError> {
        self.require_same_type(rhs)?;
        Ok(if self.ty.is_signed() {
            self.sign_extended().cmp(&rhs.sign_extended())
        } else {
            self.raw.cmp(&rhs.raw)
        })
    }

    pub fn binary(
        self,
        op: BinaryOp,
        rhs: Self,
        checks: OverflowChecks,
    ) -> Result<Self, ArithmeticError> {
        use BinaryOp::*;
        if matches!(op, ShiftLeft | ShiftRight) {
            return self.shift(op, rhs, checks);
        }
        self.require_same_type(rhs)?;
        let raw = match op {
            BitAnd => self.raw & rhs.raw,
            BitOr => self.raw | rhs.raw,
            BitXor => self.raw ^ rhs.raw,
            Divide | Remainder => return self.divide(op, rhs),
            Add | Subtract | Multiply => {
                if checks == OverflowChecks::Checked {
                    self.check_arithmetic(op, rhs)?;
                }
                match op {
                    Add => self.raw.wrapping_add(rhs.raw),
                    Subtract => self.raw.wrapping_sub(rhs.raw),
                    Multiply => self.raw.wrapping_mul(rhs.raw),
                    _ => unreachable!(),
                }
            }
            ShiftLeft | ShiftRight => unreachable!(),
        };
        Ok(Self::from_bits(self.ty, raw))
    }

    /// Const arithmetic always checks overflow, regardless of build profile.
    pub fn const_binary(self, op: BinaryOp, rhs: Self) -> Result<Self, ArithmeticError> {
        self.binary(op, rhs, OverflowChecks::Checked)
    }

    fn require_same_type(self, rhs: Self) -> Result<(), ArithmeticError> {
        if self.ty == rhs.ty {
            Ok(())
        } else {
            Err(ArithmeticError::TypeMismatch)
        }
    }

    fn check_arithmetic(self, op: BinaryOp, rhs: Self) -> Result<(), ArithmeticError> {
        use BinaryOp::*;
        let fits = if self.ty.is_signed() {
            let (a, b) = (self.sign_extended(), rhs.sign_extended());
            let value = match op {
                Add => a.checked_add(b),
                Subtract => a.checked_sub(b),
                Multiply => a.checked_mul(b),
                _ => unreachable!(),
            };
            value.is_some_and(|v| v >= self.ty.signed_min() && v <= self.ty.signed_max())
        } else {
            let value = match op {
                Add => self.raw.checked_add(rhs.raw),
                Subtract => self.raw.checked_sub(rhs.raw),
                Multiply => self.raw.checked_mul(rhs.raw),
                _ => unreachable!(),
            };
            value.is_some_and(|v| v <= self.ty.mask())
        };
        if fits {
            Ok(())
        } else {
            Err(ArithmeticError::Overflow)
        }
    }

    fn divide(self, op: BinaryOp, rhs: Self) -> Result<Self, ArithmeticError> {
        if rhs.raw == 0 {
            return Err(ArithmeticError::DivisionByZero);
        }
        let raw = if self.ty.is_signed() {
            let (a, b) = (self.sign_extended(), rhs.sign_extended());
            // Check every width, including MIN % -1, before the host operation.
            if a == self.ty.signed_min() && b == -1 {
                return Err(ArithmeticError::Overflow);
            }
            if op == BinaryOp::Divide {
                (a / b) as u128
            } else {
                (a % b) as u128
            }
        } else if op == BinaryOp::Divide {
            self.raw / rhs.raw
        } else {
            self.raw % rhs.raw
        };
        Ok(Self::from_bits(self.ty, raw))
    }

    fn shift(
        self,
        op: BinaryOp,
        rhs: Self,
        checks: OverflowChecks,
    ) -> Result<Self, ArithmeticError> {
        let negative = rhs.ty.is_signed() && rhs.sign_extended() < 0;
        if checks == OverflowChecks::Checked && (negative || rhs.raw >= u128::from(self.ty.bits()))
        {
            return Err(ArithmeticError::InvalidShift);
        }
        let count = (rhs.raw & u128::from(self.ty.bits() - 1)) as u32;
        let raw = if op == BinaryOp::ShiftLeft {
            self.raw << count
        } else if self.ty.is_signed() {
            (self.sign_extended() >> count) as u128
        } else {
            self.raw >> count
        };
        Ok(Self::from_bits(self.ty, raw))
    }
}
