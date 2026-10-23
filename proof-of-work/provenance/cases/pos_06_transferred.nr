// EXPECT: ok
// CONTRACT: swap(u: R+O, v: R+O, n: R) -> Transferred{u,v}@R
// O входа возвращается в результат; право результата остаётся R.
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
fn example(): i32 {
    let a = new User(1)
    let b = new User(2)
    let r = swap(a, b, 1)
    return r.age
}
