// EXPECT: ok
// N04: Copy-значение копируется независимо; оригинал не меняется.
class User { age: i32 }
class Box { value: User }
fn inspect(u: User): i32 { return u.age }
fn modify(var u: User): () { u.age += 1 }
fn bump(n: i32): i32 {
    var m = n
    m = m + 1
    return m + n
}
