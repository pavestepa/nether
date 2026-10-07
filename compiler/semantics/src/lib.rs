//! Executable arithmetic contracts for Nether 0.1, independent of the host target.
//!
//! This is an oracle for constant evaluation and later interpreter/backend tests,
//! not a parser, type checker or runtime implementation.

mod integer;
mod layout;
mod storage;

pub use integer::{ArithmeticError, BinaryOp, Integer, IntegerType, OverflowChecks};
pub use layout::{AggregateLayout, EnumLayout, Layout, LayoutError, MAX_OBJECT_SIZE};
pub use storage::{ElementMode, ModeError, ModeRequirement, ModeVar, StorageModes};

/// The initial target's pointer width; never derived from the compiler host.
pub const TARGET_POINTER_BITS: u32 = 64;
pub const TARGET_TRIPLE: &str = "x86_64-unknown-linux-gnu";
