// Trusted owned storage, including nominal payloads with generated cleanup. No initialized-length policy lives here.
struct RawStorage<T> {
    private data:*var T
    private cap:usize
    unsafe fn allocate(count:usize):RawStorage<T> {
        unsafe {return RawStorage<T> {data:heap_allocate<T>(count),cap:count}}
    }
    unsafe fn from_raw_parts(pointer:*var T,count:usize):RawStorage<T> {
        unsafe {heap_adopt<T>(pointer,count);return RawStorage<T> {data:pointer,cap:count}}
    }
    fn capacity(this):usize {return cap}
    unsafe fn check_range(this,start:usize,count:usize):() {
        unsafe {heap_check_range<T>(data,cap,start,count)}
    }
    unsafe fn move_within(var this,destination:usize,source:usize,count:usize):() {
        unsafe {heap_move_slots<T>(data,cap,destination,data,cap,source,count)}
    }
    unsafe fn move_from(var this,destination:usize,var source:RawStorage<T>,start:usize,count:usize):() {
        unsafe {heap_move_slots<T>(data,cap,destination,source.data,source.cap,start,count)}
    }
    unsafe fn initialize(var this,index:usize,value:T):() {
        unsafe {heap_initialize<T>(data,index,value)}
    }
    unsafe fn view(this,index:usize):T {
        unsafe {return heap_view<T>(this,index)}
    }
    unsafe fn view_mut(var this,index:usize):T {
        unsafe {return heap_view_mut<T>(this,index)}
    }
    unsafe fn slot_address(this,index:usize):*T {
        unsafe {return heap_address<T>(data,cap,index)}
    }
    unsafe fn slot_address_mut(var this,index:usize):*var T {
        unsafe {return heap_address_mut<T>(data,cap,index)}
    }
    unsafe fn take(var this,index:usize):T {
        unsafe {return heap_take<T>(data,index)}
    }
    unsafe fn replace(var this,index:usize,value:T):T {
        unsafe {return heap_replace<T>(data,index,value)}
    }
    unsafe fn drop_range(var this,start:usize,count:usize):() {
        unsafe {heap_drop_range<T>(data,cap,start,count)}
    }
    unsafe fn drop_slot(var this,index:usize):() {
        unsafe {heap_drop<T>(data,index)}
    }
    destructor {unsafe {heap_release<T>(data,cap)}}
}
private struct Released<T> {storage:RawStorage<T>}
unsafe fn allocate<T>(count:usize):RawStorage<T> {
    unsafe {return RawStorage<T>.allocate(count)}
}
fn capacity<T>(storage:RawStorage<T>):usize {return storage.capacity()}
fn size_of<T>():usize {return layout_size<T>()}
fn align_of<T>():usize {return layout_align<T>()}
fn slot_size<T>(storage:RawStorage<T>):usize {return layout_size<T>()}
fn slot_align<T>(storage:RawStorage<T>):usize {return layout_align<T>()}
unsafe fn initialize<T>(var storage:RawStorage<T>,index:usize,value:T):() {
    unsafe {storage.initialize(index,value)}
}
unsafe fn take<T>(var storage:RawStorage<T>,index:usize):T {
    unsafe {return storage.take(index)}
}
unsafe fn drop_slot<T>(var storage:RawStorage<T>,index:usize):() {
    unsafe {storage.drop_slot(index)}
}
unsafe fn release<T>(storage:RawStorage<T>):() {
    let guard=Released<T> {storage:storage}
}

unsafe fn replace<T>(var storage:RawStorage<T>,index:usize,value:T):T {
    unsafe {return storage.replace(index,value)}
}

unsafe fn move_within<T>(var storage:RawStorage<T>,destination_start:usize,source_start:usize,count:usize):() {
    unsafe {storage.move_within(destination_start,source_start,count)}
}
unsafe fn move_slots<T>(var destination:RawStorage<T>,destination_start:usize,var source:RawStorage<T>,source_start:usize,count:usize):() {
    unsafe {destination.move_from(destination_start,source,source_start,count)}
}

// Safety: every slot in the range is initialized and owned by this storage.
// Cleanup continues after the first panic; a second panic aborts.
unsafe fn drop_range<T>(var storage:RawStorage<T>,start:usize,count:usize):() {
    unsafe {storage.drop_range(start,count)}
}

unsafe fn view<T>(storage:RawStorage<T>,index:usize):T {unsafe {return storage.view(index)}}
unsafe fn view_mut<T>(var storage:RawStorage<T>,index:usize):T {unsafe {return storage.view_mut(index)}}

fn checked_index(index:usize,length:usize):usize {return bounds_index<()>(index,length)}

// Safety: raw addresses do not retain storage or establish a safe loan.
// index may equal capacity (one-past); that address cannot be dereferenced.
unsafe fn slot_address<T>(storage:RawStorage<T>,index:usize):*T {
    unsafe {return storage.slot_address(index)}
}
unsafe fn slot_address_mut<T>(var storage:RawStorage<T>,index:usize):*var T {
    unsafe {return storage.slot_address_mut(index)}
}

// Safety: pointer addresses one initialized owned value, with exclusive access.
// The value becomes uninitialized before its destructor runs, including on panic.
unsafe fn drop_in_place<T>(pointer:*var T):() {
    unsafe {pointer_drop<T>(pointer)}
}

// Safety: caller assumes responsibility for dropping initialized slots and release.
unsafe fn into_raw_parts<T>(storage:RawStorage<T>):(*var T,usize) {
    unsafe {return heap_detach<T>(storage)}
}
// Safety: pointer/count are the exact allocation returned by into_raw_parts<T>,
// and no other owner exists. This transfers allocation ownership, not slot state.
unsafe fn from_raw_parts<T>(pointer:*var T,count:usize):RawStorage<T> {
    unsafe {return RawStorage<T>.from_raw_parts(pointer,count)}
}

// Safety: pointer identifies one fully initialized owned T, with exclusive
// access. Ownership transfers to the result; the source is now uninitialized.
// The caller must prevent subsequent source access and automatic double cleanup.
unsafe fn read_owned<T>(pointer:*var T):T {
    unsafe {return pointer_take<T>(pointer)}
}

// Safety: pointer identifies one uninitialized whole slot for T, with exclusive
// access. Consumes value without dropping any previous contents.
unsafe fn write_owned<T>(pointer:*var T,value:T):() {
    unsafe {pointer_initialize<T>(pointer,0,value)}
}
