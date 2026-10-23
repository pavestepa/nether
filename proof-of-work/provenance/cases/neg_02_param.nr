// EXPECT: NMM_READONLY_TO_WRITE
// N03: R-параметр не даёт W (rebinding, вызов, consuming не помогает).
class User { age: i32 }
class Box { value: User }
fn inspect(u: User): i32 { return u.age }
fn modify(var u: User): () { u.age += 1 }
fn f(user: User): () { var editable = user }
fn g(user: User): () { modify(user) }
