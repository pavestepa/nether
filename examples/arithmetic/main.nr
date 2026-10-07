fn factorial(n: i32): i32 {
    if n == 0 { return 1 }
    return n * factorial(n - 1)
}

fn main(): i32 {
    let values = {1, 2, 3, 4}
    var index: usize = 0
    var total = 0
    while index < 4 {
        total += values[index]
        index += 1
    }
    return total + factorial(5)
}
