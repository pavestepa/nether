// EXPECT: NMM_INVALIDATION
// CONTRACT: first(u: R+O, n: R) -> -
// NAIVE: ok
// Consuming выводится только через взаимную рекурсию: однопроход его теряет.
class User { age: i32 }
class Box { value: User }
fn inspect(u: User): i32 { return u.age }
fn modify(var u: User): () { u.age += 1 }
fn first(u: User, n: i32): () {
    if n < 1 { return }
    second(u, n)
}
fn second(u: User, n: i32): () {
    if n < 1 {
        let b = new Box(u)
    } else {
        first(u, n - 1)
    }
}
fn caller(): i32 {
    var o = new User(1)
    let v = o
    first(o, 1)
    return inspect(v)
}
