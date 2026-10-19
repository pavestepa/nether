fn identity<T>(value: T): T {
    return value
}

fn add_two(value: i32): i32 {
    return value + 2
}

fn apply<T: Copy>(value: T, operation: (T) => T): T {
    return operation(value)
}

fn main(): i32 {
    let identity_i32: (i32) => i32 = identity
    return apply(identity_i32(5), add_two)
}
