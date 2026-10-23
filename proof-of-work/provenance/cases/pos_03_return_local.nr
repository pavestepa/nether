// EXPECT: ok
// CONTRACT: make_user() -> Fresh
// CONTRACT: build_user() -> Fresh
// N05: возврат локального owner передаёт O вызывающему; свежий результат допускает var.
class User { age: i32 }
class Box { value: User }
fn inspect(u: User): i32 { return u.age }
fn modify(var u: User): () { u.age += 1 }
fn make_user(): User { return new User(20) }
fn build_user(): User {
    var local = new User(20)
    modify(local)
    return local
}
fn valid(): i32 {
    var user = make_user()
    modify(user)
    var w = build_user()
    modify(w)
    return inspect(user) + inspect(w)
}
