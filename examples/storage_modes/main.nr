struct Resource {
    value: i32
    destructor {}
}

struct Holder {
    value: Resource
}

fn main(): i32 {
    let first = Resource {value: 7}
    let next = Resource {value: 3}
    let ref view = next
    var holder = Holder {value: first}
    holder.value = view
    let third = Resource {value: 5}
    let array = {third, holder.value}
    return holder.value.value + array[0].value + array[1].value
}
