struct Resource {
    value: i32
    destructor {}
}

enum Choice {
    Item(Resource),
    Empty,
}

fn main(): i32 {
    var owner = Resource {value: 7}
    let ref view = owner
    var choice = Choice.Item(view)
    choice = Choice.Empty
    owner = Resource {value: 3}
    return match choice {
        Choice.Item(value) => value.value,
        Choice.Empty => owner.value,
    }
}
