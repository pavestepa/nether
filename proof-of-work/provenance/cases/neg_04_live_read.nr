// EXPECT: NMM_BORROW_CONFLICT
// Запись владельца при живом R-view.
class User { age: i32 }
class Box { value: User }
fn inspect(u: User): i32 { return u.age }
fn modify(var u: User): () { u.age += 1 }
fn live_read(): i32 {
    var owner = new User(20)
    let reader = owner
    modify(owner)
    return inspect(reader)
}
