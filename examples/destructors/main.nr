struct Resource<T:Copy> {
    value:T
    destructor {
        let final_value=value
    }
}

fn main():i32 {
    var first=Resource<i32> {value:1}
    let second=Resource<i32> {value:2}
    first=Resource<i32> {value:3}
    // Replacement destroys 1. Scope exit destroys 3, then 2.
    return 0
}
