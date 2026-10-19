class Counter {
    private value:i32
    constructor(initial:i32) {value=initial}
    virtual fn read(this):i32 {return value}
    virtual fn bump(var this):() {value+=1}
}
fn main():i32 {
    var counter=new Counter(3)
    counter.bump()
    let read:(Counter)=>i32=Counter.read
    return read(counter)+counter.read()
}
