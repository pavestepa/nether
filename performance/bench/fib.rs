fn fib(n: i32) -> i32 {
    if n < 2 { return n }
    fib(n - 1).wrapping_add(fib(n - 2))
}

fn main() {
    std::process::exit(fib(40) % 251);
}
