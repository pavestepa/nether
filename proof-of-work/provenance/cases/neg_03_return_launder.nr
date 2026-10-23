// EXPECT: NMM_READONLY_TO_WRITE
// CONTRACT: relay2(u: R) -> Borrowed{u}@R
// N03: права не «отмываются» ни возвратом, ни цепочкой функций, ни var-владельцем.
class User { age: i32 }
class Box { value: User }
fn inspect(u: User): i32 { return u.age }
fn modify(var u: User): () { u.age += 1 }
fn relay(u: User): User { return u }
fn relay2(u: User): User { return relay(u) }
fn invalid(): () {
    var original = new User(20)
    var editable = relay2(original)
}
