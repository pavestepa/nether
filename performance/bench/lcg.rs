fn main() {
    let mut x: u32 = 12345;
    let mut i: u32 = 0;
    while i < 1000000000 {
        x = x.wrapping_mul(1664525).wrapping_add(1013904223);
        i += 1;
    }
    std::process::exit((x >> 24) as i32);
}
