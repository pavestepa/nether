struct Item {value:i32;destructor {}}
class Holder<T> {
    value:T
    constructor(source:T) {let value=source;this.value=value}
    fn get(this):T {return value}
    destructor {}
}
fn main():i32 {
    let item=Item {value:7}
    let ref view=item
    let holder=new Holder<Item>(view)
    let result=holder.get()
    return item.value+result.value
}
