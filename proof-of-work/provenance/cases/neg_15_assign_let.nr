// EXPECT: NMM_READONLY_TO_WRITE
class User { age: i32 }
class Box { value: User }
fn inspect(u: User): i32 { return u.age }
fn modify(var u: User): () { u.age += 1 }
fn bad(): () {
    let a = new User(1)
    a = new User(2)
}
