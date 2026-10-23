// EXPECT: NMM_READONLY_TO_WRITE
// Transferred-результат остаётся R.
class User { age: i32 }
class Box { value: User }
fn inspect(u: User): i32 { return u.age }
fn modify(var u: User): () { u.age += 1 }
fn swap(u: User, v: User, n: i32): User {
    if n < 1 {
        let b = new Box(u)
        return v
    }
    let c = new Box(v)
    return u
}
fn bad(): () {
    let a = new User(1)
    let b = new User(2)
    var r = swap(a, b, 1)
}
