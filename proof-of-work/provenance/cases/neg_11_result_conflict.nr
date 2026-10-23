// EXPECT: NMM_RESULT_CONTRACT
// CONTRACT: route(u: R+O, n: R) -> CONFLICT
// Owned и borrowed результаты не смешиваются (NMM §11.4).
class User { age: i32 }
class Box { value: User }
fn inspect(u: User): i32 { return u.age }
fn modify(var u: User): () { u.age += 1 }
fn route(u: User, n: i32): User {
    if n < 1 {
        let b = new Box(u)
        return new User(0)
    }
    return u
}
