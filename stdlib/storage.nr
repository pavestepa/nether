import {RawStorage,allocate,capacity,initialize,take,drop_range,checked_index,view,view_mut} from "core/intrinsic"

// Safe fixed-capacity owner. Some returns ownership to the caller when full.
enum BufferResult<T> {None,Some(T)}
struct OwnedBuffer<T> {
    private storage:RawStorage<T>
    private length:usize
    fn make(count:usize):OwnedBuffer<T> {
        unsafe {return OwnedBuffer<T> {storage:allocate<T>(count),length:0}}
    }
    fn len(this):usize {return length}
    fn capacity(this):usize {return storage.capacity()}
    fn push(var this,value:T):BufferResult<T> {
        if length==storage.capacity() {return BufferResult<T>.Some(value)}
        unsafe {initialize(storage,length,value)}
        length+=1
        return BufferResult<T>.None
    }
    fn pop(var this):BufferResult<T> {
        if length==0 {return BufferResult<T>.None}
        length-=1
        unsafe {return BufferResult<T>.Some(take(storage,length))}
    }
    // Copy values materialize unless the caller uses an explicit ref binding.
    // Resource class results borrow their identity from this buffer.
    fn at(this,index:usize):T {
        let valid=checked_index(index,length)
        unsafe {return view(storage,valid)}
    }
    fn at_mut(var this,index:usize):T {
        let valid=checked_index(index,length)
        unsafe {return view_mut(storage,valid)}
    }
    fn clear(var this):() {
        let count=length
        length=0
        unsafe {drop_range(storage,0,count)}
    }
    destructor {
        let count=length
        length=0
        unsafe {drop_range(storage,0,count)}
    }
}
