struct Item {value:i32;destructor {}}
fn identity<T>(value:T):T {return value}
fn apply<T>(function:(T)=>T,value:T):T {return function(value)}
fn main():i32 {
    let item=Item {value:7}
    let function:(Item)=>Item=identity
    let result=apply(function,item)
    return item.value+result.value
}
