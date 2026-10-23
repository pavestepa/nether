// EXPECT: NMM_NO_OWNER_TOKEN
// Alias не может быть передан в owning-позицию.
class User { age: i32 }
class Box { value: User }
fn inspect(u: User): i32 { return u.age }
fn modify(var u: User): () { u.age += 1 }
fn bad(): () {
    let o = new User(1)
    let a = o
    let b = new Box(a)
}
