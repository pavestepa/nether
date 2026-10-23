// EXPECT: NMM_SUSPENDED_ACCESS
// Родитель приостановлен на время жизни W-ребёнка.
class User { age: i32 }
class Box { value: User }
fn inspect(u: User): i32 { return u.age }
fn modify(var u: User): () { u.age += 1 }
fn live_write(): () {
    var owner = new User(20)
    var writer = owner
    modify(owner)
    modify(writer)
}
