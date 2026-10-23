// EXPECT: NMM_ESCAPE
// N07: view локального объекта не может покинуть функцию.
class User { age: i32 }
class Box { value: User }
fn inspect(u: User): i32 { return u.age }
fn modify(var u: User): () { u.age += 1 }
fn bad(): User {
    let o = new User(1)
    let a = o
    return a
}
fn bad2(): User {
    let h = new Box(new User(1))
    let v = h.value
    return v
}
