// EXPECT: ok
// CONTRACT: store(u: R+O) -> Fresh
// Параметр, попавший в owning-позицию, получает consuming-контракт.
class User { age: i32 }
class Box { value: User }
fn inspect(u: User): i32 { return u.age }
fn modify(var u: User): () { u.age += 1 }
fn store(u: User): Box { return new Box(u) }
fn example(): i32 {
    let o = new User(1)
    let b = store(o)
    let c = new Box(new User(2))
    let v = b.value
    let w = c.value
    return v.age + w.age
}
