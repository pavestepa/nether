struct Point {
    x: i64,
    y: i64,
}

fn step(p: &mut Point, i: i64) {
    p.x = p.x.wrapping_add(i);
    p.y ^= p.x;
}

fn main() {
    let mut p = Point { x: 1, y: 2 };
    let mut i: i64 = 0;
    while i < 400000000 {
        step(&mut p, i);
        i += 1;
    }
    std::process::exit(((p.x ^ p.y) % 251) as i32);
}
