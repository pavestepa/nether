unsafe fn trusted_identity<T>(value:T):T {return value}

unsafe fn invoke(callback:unsafe (i32)=>i32, value:i32):i32 {
    // Caller must satisfy the callback preconditions.
    unsafe {return callback(value)}
}

fn main():i32 {
    let callback:unsafe (i32)=>i32=trusted_identity
    var count=0
    loop {
        count+=1
        if count<7 {continue}
        unsafe {return invoke(callback,count)}
    }
}
