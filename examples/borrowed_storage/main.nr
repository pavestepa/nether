struct Resource {
    value: i32
    destructor {}
}

struct Holder {
    value: Resource
}

enum Choice {
    Some(Resource),
    None,
}

fn wrap(value: Resource): Holder {
    return Holder {value: value}
}

fn relay<T>(value: T): T {
    return value
}

fn main(): i32 {
    let owner = Resource {value: 7}
    let ref view = owner
    let holder = wrap(view)
    let array = {view, view}
    let tuple = (view, Resource {value: 3})
    let choice = relay(Choice.Some(view))
    let extracted = match choice {
        Choice.Some(value) => value.value,
        Choice.None => 0,
    }
    return holder.value.value + array[1].value + tuple.1.value + extracted
}
