// EXPECT: ok
// Поле-объект: view наследует права владельца-оболочки.
class User { age: i32 }
class Box { value: User }
fn inspect(u: User): i32 { return u.age }
fn modify(var u: User): () { u.age += 1 }
fn example(): i32 {
    var h = new Box(new User(5))
    var v = h.value
    modify(v)
    let w = h.value
    return w.age
}
