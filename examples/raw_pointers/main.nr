// Safety: p must point to a live, initialized, writable i32 with no conflicting loan.
unsafe fn increment(p:*var i32):() {
    unsafe {*p=*p+1}
}
class Counter {
    value:i32
    constructor(n:i32) {value=n}
    fn increment(var this):() {
        let p=&var value
        // Safety: this holds W for the initialized field during the whole call.
        unsafe {*p=*p+1}
    }
}
fn main():i32 {
    var counter=new Counter(1)
    counter.increment()
    var values={2,5,9}
    let first=&var values[0]
    unsafe {
        let second=first+1
        increment(second)
        let readonly=second as *i32
        return *readonly+counter.value
    }
}
