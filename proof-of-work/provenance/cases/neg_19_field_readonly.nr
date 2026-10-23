// EXPECT: NMM_READONLY_TO_WRITE
// W к полю через readonly оболочку не выдаётся.
class User { age: i32 }
class Box { value: User }
fn inspect(u: User): i32 { return u.age }
fn modify(var u: User): () { u.age += 1 }
fn bad(): () {
    let h = new Box(new User(5))
    var v = h.value
}
