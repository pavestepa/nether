import {allocate,initialize,take,drop_slot,release,capacity,size_of,align_of} from "core/intrinsic"
fn main():i32 {
    unsafe {
        var storage=allocate<i32>(3)
        initialize(storage,0,7)
        initialize(storage,1,9)
        drop_slot(storage,1)
        let result=take(storage,0)
        release(storage)
        return result
    }
}
