// Trusted scalar owned storage. No initialized-length policy lives here.
struct RawStorage<T:Copy> {
    private data:*var T
    private cap:usize
    unsafe fn allocate(count:usize):RawStorage<T> {
        unsafe {return RawStorage<T> {data:heap_allocate<T>(count),cap:count}}
    }
    fn capacity(this):usize {return cap}
    unsafe fn initialize(var this,index:usize,value:T):() {
        unsafe {heap_initialize<T>(data,index,value)}
    }
    unsafe fn take(var this,index:usize):T {
        unsafe {return heap_take<T>(data,index)}
    }
    unsafe fn replace(var this,index:usize,value:T):T {
        unsafe {return heap_replace<T>(data,index,value)}
    }
    unsafe fn drop_slot(var this,index:usize):() {
        unsafe {heap_drop<T>(data,index)}
    }
    destructor {unsafe {heap_release<T>(data,cap)}}
}
private struct Released<T:Copy> {storage:RawStorage<T>}
unsafe fn allocate<T:Copy>(count:usize):RawStorage<T> {
    unsafe {return RawStorage<T>.allocate(count)}
}
fn capacity<T:Copy>(storage:RawStorage<T>):usize {return storage.capacity()}
fn size_of<T>():usize {return layout_size<T>()}
fn align_of<T>():usize {return layout_align<T>()}
fn slot_size<T:Copy>(storage:RawStorage<T>):usize {return layout_size<T>()}
fn slot_align<T:Copy>(storage:RawStorage<T>):usize {return layout_align<T>()}
unsafe fn initialize<T:Copy>(var storage:RawStorage<T>,index:usize,value:T):() {
    unsafe {storage.initialize(index,value)}
}
unsafe fn take<T:Copy>(var storage:RawStorage<T>,index:usize):T {
    unsafe {return storage.take(index)}
}
unsafe fn drop_slot<T:Copy>(var storage:RawStorage<T>,index:usize):() {
    unsafe {storage.drop_slot(index)}
}
unsafe fn release<T:Copy>(storage:RawStorage<T>):() {
    let guard=Released<T> {storage:storage}
}

unsafe fn replace<T:Copy>(var storage:RawStorage<T>,index:usize,value:T):T {
    unsafe {return storage.replace(index,value)}
}
