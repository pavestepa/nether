interface Read {fn read(this):i32}
interface Bump {fn bump(var this):()}
struct Counter:Read,Bump {
    value:i32
    fn read(this):i32 {return value}
    fn bump(var this):() {value+=1}
}
fn advance<T:Read & Bump>(var value:T):i32 {
    value.bump()
    return value.read()
}
fn main():i32 {
    var counter=Counter {value:3}
    let current=advance(counter)
    return current+counter.read()
}
