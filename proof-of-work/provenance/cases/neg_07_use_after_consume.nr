// EXPECT: NMM_USE_AFTER_TRANSFER
// N08: использование после передачи O.
class User { age: i32 }
class Box { value: User }
fn inspect(u: User): i32 { return u.age }
fn modify(var u: User): () { u.age += 1 }
fn bad(): i32 {
    let o = new User(1)
    let b = new Box(o)
    return inspect(o)
}
