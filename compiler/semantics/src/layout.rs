//! Baseline Nether object layout for the first 64-bit target.
//!
//! This is not a C ABI argument classifier. Fields stay in declaration order,
//! enums use an explicit u32 tag, and no niches or packing are introduced.

use crate::IntegerType;

/// Objects must fit in the target's signed pointer-offset range.
pub const MAX_OBJECT_SIZE: u64 = i64::MAX as u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutError {
    InvalidAlignment,
    UnpaddedSize,
    ObjectTooLarge,
    TooManyVariants,
}

/// Type size includes tail padding, so it is also the array element stride.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    size: u64,
    align: u64,
}

impl Layout {
    pub const UNIT: Self = Self { size: 0, align: 1 };
    pub const BOOL: Self = Self { size: 1, align: 1 };
    pub const CHAR: Self = Self { size: 4, align: 4 };
    pub const F32: Self = Self { size: 4, align: 4 };
    pub const F64: Self = Self { size: 8, align: 8 };
    pub const POINTER: Self = Self { size: 8, align: 8 };
    /// Data address followed by UTF-8 byte length; no owned allocation.
    pub const STR: Self = Self { size: 16, align: 8 };

    pub fn new(size: u64, align: u64) -> Result<Self, LayoutError> {
        if !align.is_power_of_two() {
            return Err(LayoutError::InvalidAlignment);
        }
        if size > MAX_OBJECT_SIZE || align > MAX_OBJECT_SIZE {
            return Err(LayoutError::ObjectTooLarge);
        }
        if size % align != 0 {
            return Err(LayoutError::UnpaddedSize);
        }
        Ok(Self { size, align })
    }

    pub fn integer(ty: IntegerType) -> Self {
        let bytes = u64::from(ty.bits() / 8);
        Self {
            size: bytes,
            align: bytes,
        }
    }

    pub fn size(self) -> u64 {
        self.size
    }
    pub fn align(self) -> u64 {
        self.align
    }

    /// Empty and zero-sized arrays still retain element alignment.
    pub fn array(self, length: u64) -> Result<Self, LayoutError> {
        let size = self
            .size
            .checked_mul(length)
            .ok_or(LayoutError::ObjectTooLarge)?;
        Self::new(size, self.align)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AggregateLayout {
    pub layout: Layout,
    pub field_offsets: Vec<u64>,
}

impl AggregateLayout {
    /// Struct, tuple, class payload, or one enum variant's fields.
    pub fn new(fields: &[Layout]) -> Result<Self, LayoutError> {
        let mut end = 0;
        let mut align = 1;
        let mut offsets = Vec::with_capacity(fields.len());
        for field in fields {
            align = align.max(field.align);
            let offset = align_up(end, field.align)?;
            end = offset
                .checked_add(field.size)
                .ok_or(LayoutError::ObjectTooLarge)?;
            if end > MAX_OBJECT_SIZE {
                return Err(LayoutError::ObjectTooLarge);
            }
            offsets.push(offset);
        }
        Ok(Self {
            layout: Layout::new(align_up(end, align)?, align)?,
            field_offsets: offsets,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumLayout {
    pub layout: Layout,
    /// Absent for an uninhabited enum with no variants.
    pub tag: Option<Layout>,
    pub payload_offset: u64,
}

impl EnumLayout {
    /// Tags are declaration indices. Payload fields use AggregateLayout first.
    /// This represents storage, not a claim that every bit pattern is valid.
    pub fn new(variants: &[Layout]) -> Result<Self, LayoutError> {
        if variants.is_empty() {
            return Ok(Self {
                layout: Layout::UNIT,
                tag: None,
                payload_offset: 0,
            });
        }
        if variants.len() as u128 > u128::from(u32::MAX) + 1 {
            return Err(LayoutError::TooManyVariants);
        }
        let tag = Layout::integer(IntegerType::U32);
        let payload_align = variants.iter().map(|v| v.align).max().unwrap();
        let payload_size = variants.iter().map(|v| v.size).max().unwrap();
        let align = tag.align.max(payload_align);
        let payload_offset = align_up(tag.size, payload_align)?;
        let end = payload_offset
            .checked_add(payload_size)
            .ok_or(LayoutError::ObjectTooLarge)?;
        Ok(Self {
            layout: Layout::new(align_up(end, align)?, align)?,
            tag: Some(tag),
            payload_offset,
        })
    }
}

fn align_up(value: u64, align: u64) -> Result<u64, LayoutError> {
    let rounded = value
        .checked_add(align - 1)
        .ok_or(LayoutError::ObjectTooLarge)?
        & !(align - 1);
    if rounded > MAX_OBJECT_SIZE {
        return Err(LayoutError::ObjectTooLarge);
    }
    Ok(rounded)
}
