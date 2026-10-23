// EXPECT: ok
// CONTRACT: readonly_identity(u: R) -> Borrowed{u}@R
// CONTRACT: mutable_identity(u: W) -> Borrowed{u}@W
// N06: результат — view входа; право доступа результата следует за параметром.
class User { age: i32 }
class Box { value: User }
fn inspect(u: User): i32 { return u.age }
fn modify(var u: User): () { u.age += 1 }
fn readonly_identity(u: User): User { return u }
fn mutable_identity(var u: User): User { return u }
fn example(): i32 {
    var owner = new User(20)
    var alias = mutable_identity(owner)
    modify(alias)
    let age = inspect(owner)
    let r = readonly_identity(owner)
    return age + inspect(r)
}
