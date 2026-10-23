// EXPECT: ok
// CONTRACT: c(u: R) -> Borrowed{u}@R
// CONTRACT: b(u: R) -> Borrowed{u}@R
// Порядок объявления против зависимостей: вывод от него не зависит.
class User { age: i32 }
class Box { value: User }
fn inspect(u: User): i32 { return u.age }
fn modify(var u: User): () { u.age += 1 }
fn c(u: User): User { return b(u) }
fn b(u: User): User { return a(u) }
fn a(u: User): User { return u }
