class Counter {
    value: i32
    constructor(initial: i32) { value = initial }
    fn increment(var this): () { value += 1 }
    fn read(this): i32 { return value }
    destructor { value = 0 }
}

#[Copy]
class Number {
    value: i32
    constructor(initial: i32) { value = initial }
}

fn relay(counter: Counter): Counter { return counter }

fn main(): i32 {
    var counter = new Counter(3)
    counter.increment()
    let moved = relay(counter)
    let original = new Number(5)
    var independent = original
    independent.value = 9
    if same_identity(original, independent) { return 1 }
    return moved.read() + original.value + independent.value
}
