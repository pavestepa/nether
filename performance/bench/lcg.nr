fn main(): i32 {
    var x: u32 = 12345
    var i: u32 = 0
    while i < 1000000000 {
        x = x * 1664525 + 1013904223
        i += 1
    }
    return (x >> 24) as i32
}
