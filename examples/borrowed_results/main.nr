struct Cell { value: i32 }

struct Pair {
    left: Cell
    right: Cell

    fn first(var this): Cell {
        return left
    }
}

fn identity<T>(value: T): T {
    return value
}

fn relay<T>(value: T): T {
    return identity(value)
}

fn main(): i32 {
    var pair = Pair { left: Cell { value: 5 }, right: Cell { value: 9 } }
    let ref var first = pair.first()
    first.value += 2

    let ref source = pair.left
    let result = relay(source)
    return result.value + pair.right.value
}
