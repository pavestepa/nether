struct Node {
    value: i32,
}

impl Node {
    fn new(initial: i32) -> Box<Node> { Box::new(Node { value: initial }) }
    fn read(&self) -> i32 { self.value }
}

fn main() {
    let mut total: i64 = 0;
    let mut i: i32 = 0;
    while i < 20000000 {
        let node = std::hint::black_box(Node::new(i));
        total += node.read() as i64;
        i += 1;
    }
    std::process::exit((total % 251) as i32);
}
