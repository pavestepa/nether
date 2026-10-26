import {RawStorage,allocate,capacity,initialize,take,drop_slot} from "core/intrinsic"
// A fixed-capacity safe wrapper. The initialized prefix belongs to this buffer.
struct Buffer {
    private storage:RawStorage<i32>
    private length:usize
    fn make(count:usize):Buffer {
        unsafe {return Buffer {storage:allocate<i32>(count),length:0}}
    }
    fn push(var this,value:i32):bool {
        if length==capacity(storage) {return false}
        unsafe {initialize(storage,length,value)}
        length+=1
        return true
    }
    fn pop(var this):i32 {
        if length==0 {return 0}
        length-=1
        unsafe {return take(storage,length)}
    }
    destructor {
        while length>0 {
            length-=1
            unsafe {drop_slot(storage,length)}
        }
        // The compiler cleans storage after this destructor returns.
    }
}
fn main():i32 {
    var buffer=Buffer.make(3)
    buffer.push(3)
    buffer.push(7)
    return buffer.pop()
}
