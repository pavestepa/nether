// EXPECT: NMM_INVALIDATION
// Передача O при живом view на тот же объект.
class User { age: i32 }
class Box { value: User }
fn inspect(u: User): i32 { return u.age }
fn modify(var u: User): () { u.age += 1 }
fn bad(): i32 {
    var o = new User(1)
    let v = o
    let b = new Box(o)
    return inspect(v)
}
