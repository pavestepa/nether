// EXPECT: NMM_BORROW_CONFLICT
// CONTRACT: pass(u: R, n: R) -> Borrowed{u}@R
// NAIVE: ok
// Происхождение результата рекурсивной функции: однопроход его теряет.
class User { age: i32 }
class Box { value: User }
fn inspect(u: User): i32 { return u.age }
fn modify(var u: User): () { u.age += 1 }
fn caller(): i32 {
    var o = new User(1)
    let r = pass(o, 3)
    modify(o)
    return inspect(r)
}
fn pass(u: User, n: i32): User {
    if n < 1 { return u }
    return pass(u, n - 1)
}
