// EXPECT: NMM_BORROW_CONFLICT
// Один объект в двух аргументах, один из них изменяемый.
class User { age: i32 }
class Box { value: User }
fn inspect(u: User): i32 { return u.age }
fn modify(var u: User): () { u.age += 1 }
fn rw(var a: User, b: User): () { a.age = b.age }
fn bad(): () {
    var x = new User(1)
    rw(x, x)
}
