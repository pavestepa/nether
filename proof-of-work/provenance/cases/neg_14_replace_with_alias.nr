// EXPECT: NMM_NO_OWNER_TOKEN
// Owned-слот можно заменить только owned-значением.
class User { age: i32 }
class Box { value: User }
fn inspect(u: User): i32 { return u.age }
fn modify(var u: User): () { u.age += 1 }
fn bad(): () {
    var o = new User(1)
    var p = new User(2)
    var q = p
    o = q
}
