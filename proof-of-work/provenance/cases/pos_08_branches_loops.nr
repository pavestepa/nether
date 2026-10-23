// EXPECT: ok
// Reborrow в цикле; перенос O на одной ветви без дальнейшего использования.
class User { age: i32 }
class Box { value: User }
fn inspect(u: User): i32 { return u.age }
fn modify(var u: User): () { u.age += 1 }
fn example(c: i32): i32 {
    var o = new User(1)
    var i = 0
    while i < 3 {
        var b = o
        b.age += 1
        i = i + 1
    }
    if c < 1 {
        let h = new Box(o)
    } else {
        modify(o)
    }
    return c
}
