class Node {
    value: i32
    constructor(initial: i32) { value = initial }
    fn read(this): i32 { return value }
}

fn main(): i32 {
    var total: i64 = 0
    var i: i32 = 0
    while i < 20000000 {
        let node = new Node(i)
        total += node.read() as i64
        i += 1
    }
    return (total % 251) as i32
}
