// EXPECT: NMM_BORROW_CONFLICT
// Результат несёт origin входа: запись владельца при живом результате — конфликт.
class User { age: i32 }
fn inspect(u: User): i32 { return u.age }
fn modify(var u: User): () { u.age += 1 }
fn readonly_identity(u: User): User { return u }
fn bad(): i32 {
    var o = new User(1)
    let r = readonly_identity(o)
    modify(o)
    return inspect(r)
}

