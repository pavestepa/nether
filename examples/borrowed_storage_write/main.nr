struct Resource {
    value: i32
    destructor {}
}

struct Holder<T> {
    value: T
}

fn set<T>(var holder: Holder<T>, source: T): () {
    let value = source
    holder.value = value
}

fn main(): i32 {
    let first = Resource {value: 7}
    let second = Resource {value: 3}
    let ref first_view = first
    let ref second_view = second
    var holder = Holder<Resource> {value: first_view}
    let old_view = holder.value
    set(holder, second_view)
    return old_view.value + holder.value.value
}
