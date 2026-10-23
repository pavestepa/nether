fn main(): i32 {
    let values = {3, 1, 4, 1, 5, 9, 2, 6}
    var total: i64 = 0
    var i: usize = 0
    while i < 500000000 {
        total += values[i % 8] as i64
        i += 1
    }
    return (total % 251) as i32
}
