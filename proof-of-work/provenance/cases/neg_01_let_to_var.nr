// EXPECT: NMM_READONLY_TO_WRITE
// N03: R -> W через локальный let.
class User { age: i32 }
fn bad(): () {
    let a = new User(20)
    var b = a
}
