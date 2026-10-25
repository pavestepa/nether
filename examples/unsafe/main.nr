// Safety: caller must establish value >= 0.
unsafe fn trusted_nonnegative(value: i32): i32 {
    return value
}

fn checked_nonnegative(value: i32): i32 {
    if value < 0 {return 0}
    unsafe {
        // Safety: the preceding check established the precondition.
        return trusted_nonnegative(value)
    }
}

fn main(): i32 {
    return checked_nonnegative(7)
}
