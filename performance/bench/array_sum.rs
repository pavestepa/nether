fn main() {
    let values: [i32; 8] = [3, 1, 4, 1, 5, 9, 2, 6];
    let mut total: i64 = 0;
    let mut i: usize = 0;
    while i < 500000000 {
        total += values[i % 8] as i64;
        i += 1;
    }
    std::process::exit((total % 251) as i32);
}
