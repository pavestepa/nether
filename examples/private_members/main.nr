class Vault<T:Copy> {
    private value:T
    constructor(initial:T) {value=initial}
    private fn identity<U:Copy>(input:U):U {return input}
    fn read(this):T {return value}
    fn transformer():(i32)=>i32 {return Vault<T>.identity<i32>}
    destructor {let final_value=value}
}
fn main():i32 {
    let vault=new Vault<i32>(3)
    let transform=Vault<i32>.transformer()
    return vault.read()+transform(4)
}
