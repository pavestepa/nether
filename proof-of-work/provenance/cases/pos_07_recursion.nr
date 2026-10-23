// EXPECT: ok
// CONTRACT: pass(u: R, n: R) -> Borrowed{u}@R
// Рекурсивный результат: происхождение выводится fixed point.
class User { age: i32 }
class Box { value: User }
fn inspect(u: User): i32 { return u.age }
fn modify(var u: User): () { u.age += 1 }
fn pass(u: User, n: i32): User {
    if n < 1 { return u }
    return pass(u, n - 1)
}
fn example(): i32 {
    var o = new User(1)
    let r = pass(o, 3)
    let age = inspect(r)
    modify(o)
    return age
}
