use nether_core::{check::check, llvm::emit};
use nether_frontend::{parser::parse, source::SourceId};
use nether_semantics::OverflowChecks;
use std::{
    fs,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

static SEQUENCE: AtomicUsize = AtomicUsize::new(0);

fn verify(source: &str, checks: OverflowChecks) {
    let parsed = parse(SourceId(0), source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let program = check(&parsed.module.unwrap()).unwrap();
    let ir = emit(&program, checks).unwrap();
    assert!(!ir.contains(" inbounds "));
    let compiler = std::env::var("NETHER_CLANG").unwrap_or_else(|_| "clang".into());
    // A missing compiler is a failure: these are actual LLVM verification tests.
    let dir = std::env::temp_dir().join(format!(
        "nether-llvm-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).unwrap();
    let input = dir.join("case.ll");
    let output = dir.join("case.o");
    fs::write(&input, &ir).unwrap();
    for optimization in ["-O0", "-O2"] {
        let result = Command::new(&compiler)
            .args([
                "--target=x86_64-unknown-linux-gnu",
                "-x",
                "ir",
                "-c",
                optimization,
            ])
            .arg(&input)
            .arg("-o")
            .arg(&output)
            .output()
            .expect("LLVM compiler required; set NETHER_CLANG");
        assert!(
            result.status.success(),
            "{}\n{ir}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(fs::metadata(&output).unwrap().len() > 0);
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn llvm_accepts_recursive_control_flow_and_aggregate_values() {
    for code in [
        include_str!("../../../examples/arithmetic/main.nr"),
        "#[Copy] struct Box<T:Copy> {value:T;fn create(value:T):Box<T> {return Box<T> {value:value}};fn extract(value:Box<T>):T {return value.value}} fn main():i32 {let boxed=Box<i32>.create(17);return Box.extract(boxed)}",
        "fn predicate(var x:i32):bool {x+=1;return x==4} fn main():i32 {let n=3;return match n {x if predicate(x)=>x,_=>0}}",

        "#[Copy] enum Option<T> {None,Some(T)} fn unwrap<T:Copy>(value:Option<T>,fallback:T):T {return match value {Option.Some(x)=>x,Option.None=>fallback}} fn main():i32 {let value=Option<i32>.Some(8);return unwrap(value,1)}",

        "fn identity<T>(x:T):T {return x} fn main():i32 {let a:u8=identity(7);return identity<i32>(a as i32)}",
        "fn last<T,const N:usize>(values:{T;N}):T {return values[N-1]} fn main():i32 {return last({1,2,3})}",
        "fn pick<T>(n:i32,x:T):T {if n==0 {return x};return pick<T>(n-1,x)} fn main():i32 {return pick(5,120)}",

        "#[Copy] enum E {None,Some(i32)} fn main():i32 {let a=E.None;return match a {E.Some(x)=>x,E.None=>7}}",
        "#[Copy] enum E {A {x:i32,y:i32},B(u128),C} fn main():i32 {let a=E.B(340282366920938463463374607431768211455);return match a {E.A {x,y}=>x+y,E.B(x)=> (x/3) as i32,E.C=>0}}",
        "#[Copy] enum Inner {A(i32),B} #[Copy] enum Outer {Some(Inner),None} fn main():i32 {let x=Outer.Some(Inner.B);return match x {Outer.Some(Inner.A(v))=>v,Outer.Some(_)=>2,Outer.None=>3}}",

        "fn main():i32 {return match (3,true) {(x,false)=>x,(x,true) if x>4=>9,(x,_)=>x+10}}",
        "fn main():i32 {match true {false=>{return 4},true=>{return 7}}}",
        "fn main():i32 {return match {1,2,3,4} {{x,..,y}=>x+y}}",
        "#[Copy] struct Pair {left:i32;right:i32} fn main():i32 {var n=0;var p=Pair {right:if true {n+=1;n} else {0},left:if true {n+=1;n} else {0}};let c=p;p.left+=10;return c.left+p.left}",

        "fn main():i32 {return if true {return 1} else {return 2}}",
        "fn main():i32 {var i=0;while i<4 {i+=1;if i==2 {continue};if i==3 {break}};return i}",
        "fn main():i32 {var pair=(1,true);pair.0+=2;return pair.0}",
        "fn main():i32 {var a={1,2};var i:usize=0;a[if true {i+=1;i} else {0}]+=3;return a[1]}",
        "fn main():i32 {return if false && 1/0==0 {1} else {2}}",
        "fn main():i32 {let text=\"héllo\";return 0}",
    ] {
        verify(code, OverflowChecks::Checked);
    }
}

#[test]
fn llvm_accepts_integer_checks_shifts_and_saturating_float_casts() {
    for mode in [OverflowChecks::Checked, OverflowChecks::Wrapping] {
        for code in [
            "fn main():i32 {let x:i8=127;return (x+1) as i32}",
            "fn main():i32 {let x:i128=-170141183460469231731687303715884105728;return (x % -1) as i32}",
            "fn main():i32 {let x:i128=1;let count:i8=-1;return (x << count) as i32}",
            "fn main():i32 {let x:u8=255;let count:u128=256;return (x >> count) as i32}",
            "fn main():i32 {let x:f32=1.5;return x as i32}",
            "fn main():i32 {let x=0.0/0.0;return x as i32}",
            "fn main():i32 {let x:f32=1.5;let wide=x as f64;return wide as i32}",
        ] {verify(code,mode);}
    }
}
