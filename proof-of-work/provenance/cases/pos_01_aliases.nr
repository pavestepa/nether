// EXPECT: ok
// CONTRACT: inspect(u: R) -> -
// CONTRACT: modify(u: W) -> -
// N01: несколько let-алиасов одного non-Copy; N02 после завершения алиасов родитель восстановлен.
class User { age: i32 }
fn inspect(u: User): i32 { return u.age }
fn modify(var u: User): () { u.age += 1 }
fn main(): i32 {
    var a = new User(20)
    let b = a
    let c = b
    let total = inspect(b) + inspect(c)
    modify(a)
    return total + inspect(a)
}
