// EXPECT: ok
// N02: последовательные var-reborrow; каждый ребёнок завершается до использования родителя.
class User { age: i32 }
fn modify(var u: User): () { u.age += 1 }
fn twice(var u: User): () { modify(u) modify(u) }
fn main(): i32 {
    var a = new User(20)
    var b = a
    b.age += 1
    var c = b
    modify(c)
    b.age += 1
    a.age += 1
    twice(a)
    return a.age
}
