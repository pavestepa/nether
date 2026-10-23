// EXPECT: NMM_SUSPENDED_ACCESS
// Alias живёт через back edge цикла.
class User { age: i32 }
class Box { value: User }
fn inspect(u: User): i32 { return u.age }
fn modify(var u: User): () { u.age += 1 }
fn bad(): () {
    var o = new User(1)
    var prev = o
    var i = 0
    while i < 3 {
        modify(o)
        i = i + 1
    }
    modify(prev)
}
