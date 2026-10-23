fn collatz(start: u64) -> u64 {
    let mut n = start;
    let mut steps: u64 = 0;
    while n != 1 {
        if n % 2 == 0 { n /= 2 } else { n = n.wrapping_mul(3).wrapping_add(1) }
        steps += 1;
    }
    steps
}

fn main() {
    let mut best: u64 = 0;
    let mut i: u64 = 1;
    while i < 2000000 {
        let s = collatz(i);
        if s > best { best = s }
        i += 1;
    }
    std::process::exit((best % 251) as i32);
}
