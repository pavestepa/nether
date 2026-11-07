import {OwnedBuffer,BufferResult} from "core/storage"
class Item {
    value:i32
    constructor(n:i32) {value=n}
    destructor {}
}
fn fetch():BufferResult<Item> {
    var buffer=OwnedBuffer<Item>.make(2)
    buffer.push(new Item(3))
    buffer.push(new Item(7))
    return buffer.pop()
}
fn main():i32 {
    return match fetch() {
        BufferResult.Some(item)=>item.value,
        BufferResult.None=>0
    }
}
