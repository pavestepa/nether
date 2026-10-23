fn collatz(start: u64): u64 {
    var n = start
    var steps: u64 = 0
    while n != 1 {
        if n % 2 == 0 { n = n / 2 } else { n = 3 * n + 1 }
        steps += 1
    }
    return steps
}

fn main(): i32 {
    var best: u64 = 0
    var i: u64 = 1
    while i < 2000000 {
        let s = collatz(i)
        if s > best { best = s }
        i += 1
    }
    return (best % 251) as i32
}
