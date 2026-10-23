// EXPECT: NMM_VAR_CONSUME
// N09: var-параметр (W borrow) не может быть потреблён, в том числе транзитивно.
class User { age: i32 }
class Box { value: User }
fn inspect(u: User): i32 { return u.age }
fn modify(var u: User): () { u.age += 1 }
fn f(var u: User): () { let b = new Box(u) }
fn eat(u: User): () { let b = new Box(u) }
fn g(var u: User): () { eat(u) }
