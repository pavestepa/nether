//! Execute emitted Rust on the host and compare both independent interpreters.
use nether_core::{check::check, hir::Value, interpret::execute, rust::emit_with_sources};
use nether_frontend::{
    parser::parse,
    source::{Source, SourceId},
};
use nether_semantics::OverflowChecks;
use std::{fs, process::Command};
#[test]
fn rust_native_matches_reference_at_both_optimization_levels() {
    let cases=[
        "fn identity<T>(x:T):T {return x} fn main():i32 {let f:(i32)=>i32=identity;return f(9)}",
        "fn identity<T>(x:T):T {return x} fn main():i32 {let f:(i32)=>i32=identity<i32>;return f(9)}",
        "fn first<T:Copy,const N:usize>(xs:{T;N}):T {return xs[0]} fn main():i32 {let f:({i32;2})=>i32=first;return f({8,9})}",
        "fn identity<T>(x:T):T {return x} fn select():(i32)=>i32 {return identity} fn main():i32 {return select()(6)}",
        "struct Cell {value:i32} struct Hold {cell:Cell} fn take(x:Cell):i32 {let held=Hold {cell:x};return held.cell.value+1} fn main():i32 {var x=Cell {value:1};let n=take(x);x=Cell {value:4};return n+x.value}",
        "struct Cell {value:i32} struct Hold {cell:Cell} fn take(x:Cell):i32 {let held=Hold {cell:x};return held.cell.value+1} fn main():i32 {var x=Cell {value:1};let n=take(x);if true {x=Cell {value:4}} else {x=Cell {value:5}};return n+x.value}",
        "struct Cell {value:i32} struct Hold {cell:Cell} fn take(x:Cell):i32 {let held=Hold {cell:x};return held.cell.value+1} fn main():i32 {var x=Cell {value:1};var n=0;while n<6 {n+=take(x);x=Cell {value:1}};return n+x.value}",
        "struct Cell {value:i32} struct Hold {cell:Cell} fn take(x:Cell):i32 {let held=Hold {cell:x};return held.cell.value+1} fn main():i32 {var x=Cell {value:1};var n=0;while n<6 {n+=take(x);x=Cell {value:1}};return n+x.value}",
        "struct Math {fn add(n:i32):i32 {return n+2}} fn main():i32 {let f=Math.add;return f(3)}",
        "struct Handler {run:(i32)=>i32;fn invoke(this):i32 {return run(4)}} fn add(n:i32):i32 {return n+2} fn main():i32 {let h=Handler {run:add};return h.invoke()}",
        "fn add(n:i32):i32 {return n+2} fn main():i32 {let f:(i32)=>i32=add;return f(3)}",
        "fn one():i32 {return 1} fn two():i32 {return 2} fn choose(b:bool):()=>i32 {return if b {one} else {two}} fn main():i32 {return choose(false)()}",
        "fn bump(var n:i32):i32 {n+=1;return n} fn apply(f:(var i32)=>i32,n:i32):i32 {return f(n)} fn main():i32 {let n=3;return apply(bump,n)+n}",
        "fn one():i32 {return 1} fn two():i32 {return 2} fn main():i32 {var f=one;let copy=f;f=two;return f()*10+copy()}",
        "#[Copy] struct Handler {run:(i32)=>i32} fn add(n:i32):i32 {return n+2} fn main():i32 {let h=Handler {run:add};return h.run(3)}",
        "fn add(n:i32):i32 {return n+2} fn apply<T:Copy>(f:(T)=>T,n:T):T {return f(n)} fn main():i32 {return apply(add,3)}",
        "fn add(n:i32):i32 {return n+2} fn invoke<T:Copy>(f:()=>T):T {return f()} fn five():i32 {return 5} fn main():i32 {return invoke(five)}",
        "fn yes(n:i32):bool {return n>0} fn main():i32 {let predicate=yes;return match 7 {x if predicate(x)=>x,_=>0}}",
        "fn fail():i32 {return 1/0} fn main():i32 {let f=fail;return f()}",
        "fn fail():i32 {\nlet n=1\nlet text=\"привет\";return n/0\n}\nfn main():i32 {return fail()}",
        "fn main():i32 {\nlet a={1}\nreturn a[2]\n}",
        "struct Cell {value:i32} fn bump(var x:Cell):() {x.value+=1} fn read(x:Cell):i32 {return x.value} fn main():i32 {var x=Cell {value:1};bump(x);return read(x)+x.value}",
        "struct Cell {value:i32} fn bump(var x:Cell,n:i32):() {if n==0 {return};x.value+=1;bump(x,n-1)} fn main():i32 {var x=Cell {value:1};bump(x,5);return x.value}",
        "struct Cell {value:i32} fn read(x:Cell):i32 {return x.value} fn main():i32 {let x=Cell {value:3};return read(x)+read(x)+x.value}",
        "struct Cell {value:i32} fn select<T>(x:T,n:i32):i32 {return n} fn main():i32 {let x=Cell {value:1};return select(x,7)+x.value}",
        "struct Cell {value:i32} fn change(var x:Cell,var y:Cell):() {x=y;x.value=9} fn main():i32 {var a=Cell {value:1};var b=Cell {value:2};change(a,b);return a.value*10+b.value}",
        "struct Cell {value:i32} fn bump(var x:Cell):() {x.value+=1} fn main():i32 {var pair=(Cell {value:1},Cell {value:2});bump(pair.0);return pair.0.value+pair.1.value}",
        "struct Cell {value:i32;fn bump(var this):() {value+=1};fn read(this):i32 {return value};fn twice(var this):() {bump();this.bump()}} fn main():i32 {var c=Cell {value:1};c.twice();return c.read()}",
        "#[Copy] struct Cell {value:i32;fn bump(var this):() {this.value+=1};fn read(this):i32 {return this.value}} fn main():i32 {var c=Cell {value:1};c.bump();return c.read()+c.value}",
        "struct Box<T> {value:T;fn read(this):i32 {return 7}} fn main():i32 {let b=Box<i32> {value:3};return b.read()}",
        "#[Copy] struct Box<T:Copy> {value:T;fn get(this):T {return value}} fn main():i32 {return (Box<i32> {value:9}).get()}",
        "struct Cell {value:i32;fn read(this):i32 {let value=4;return value+this.value}} fn main():i32 {let c=Cell {value:3};return c.read()}",
        "struct Cell {value:i32} struct Hold {cell:Cell} fn main():i32 {var x=Cell {value:1};let y=Hold {cell:x};if true {x=Cell {value:2}} else {x=Cell {value:3}};return y.cell.value+x.value}",
        "struct Cell {value:i32} struct Hold {cell:Cell} fn main():i32 {var x=(Cell {value:1},Cell {value:2});let y=Hold {cell:x.0};let z=Hold {cell:x.1};x.0=Cell {value:3};return y.cell.value+z.cell.value+x.0.value}",
        "struct Cell {value:i32} fn identity(x:Cell):Cell {return x} fn main():i32 {let x=Cell {value:7};let y=identity(x);return y.value}",
        "struct Cell {value:i32} fn main():i32 {let x=Cell {value:7};let y=x;return y.value+x.value}",
        "struct Cell {value:i32} struct Hold {cell:Cell} struct Pair {a:Cell;b:Cell} fn main():i32 {var x=Pair {a:Cell {value:1},b:Cell {value:2}};let y=Hold {cell:x.a};x.a=Cell {value:4};let z=x;return y.cell.value+z.a.value+z.b.value}",
        "enum E {A(i32),B} fn main():i32 {let e=E.A(7);return match e {E.A(x) if x>2=>x,E.A(_)=>1,E.B=>0}}",
        "struct Cell {value:i32} fn identity<T>(x:T):T {return x} fn main():i32 {let x=Cell {value:5};let y=identity(x);return y.value}",
        "struct Cell {value:i32} fn take(x:Cell):i32 {return x.value} fn main():i32 {var x=Cell {value:1};var n=0;while n<3 {n+=take(x);x=Cell {value:1}};return n}",

        "fn main():i32 {var a=1;var b=2;let ref var x=a;*x=if true {x=b;7} else {0};return a*10+b}",
        "fn main():i32 {var a=1;let ref var x=a;*x=x+2;return a}",
        "fn main():i32 {var a={1};let ref x=a[2];return x}",
        "fn main():i32 {var a={1};match a[2] {ref x=>{return x}}}",
        "fn main():i32 {var a=(1,2);let (ref var x,ref y)=a;*x+=y;return a.0}",

        "#[Copy] enum E {A(i32),B} fn main():i32 {var a=E.A(3);match a {E.A(ref var x) if x>1=>{*x+=4},_=>{}};return match a {E.A(x)=>x,E.B=>0}}",
        "#[Copy] enum E {A(i32),B} fn main():i32 {var a=E.A(3);match a {E.A(ref var x) if false=>{*x=99},E.A(ref var x)=>{*x+=1},_=>{}};return match a {E.A(x)=>x,E.B=>0}}",
        "fn main():i32 {var a=(1,2);match a {(ref var x,_) if a.1==2=>{*x=8},_=>{}};return a.0}",
        "fn main():i32 {let a=(1,2);let x=match a {(x,_)=>x};return x+a.1}",
        "fn main():i32 {let pair=(1,2);var n=0;while n<2 {let (x,_)=pair;n+=x};return n}",
        "fn main():i32 {var pair=(1,2);let (x,_)=pair;pair.1=9;return x+pair.1}",

        "fn main():i32 {var a={1,2};let {ref var x,..}=a;*x=7;*x+=1;return a[0]}",
        "fn main():i32 {var a=1;var b=2;let ref var x=a;*x=3;x=b;*x=7;return a*10+b}",
        "fn main():i32 {var a=(1,2);let (ref var x,ref var y)=a;*x+=3;*y+=5;return a.0+a.1}",
        "fn main():i32 {var a={1,2};var i:usize=0;let ref var x=a[i];i=1;*x=8;return a[0]+a[1]}",
        "fn main():i32 {var a=1;let ref var parent=a;let ref child=parent;let copy=child;*parent=5;return a+copy}",
        "fn main():i32 {var a=1;let ref x=a;let copy=x;a=4;return copy+a}",
        "fn main():i32 {var a=1;let ref var x=a;var n=0;while n<3 {*x+=1;n+=1};return a}",

        include_str!("../../../examples/arithmetic/main.nr"),
        include_str!("../../../examples/unsafe/main.nr"),
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

        "fn main():i32 {return if true {return 9} else {return 4}}",
        "fn main():i32 {var i=0;var sum=0;while i<10 {i+=1;if i==3 {continue};if i==6 {break};sum+=i};return sum}",
        "fn main():i32 {var pair=(1,true);pair.0+=2;return pair.0}",
        "fn main():i32 {var a={1,2};var i:usize=0;a[if true {i+=1;i} else {0}]+=3;return a[1]}",
        "fn main():i32 {var a={1};a[2]=1/0;return a[0]}",
        "fn main():i32 {return if false && 1/0==0 {1} else {2}}",
        "fn main():i32 {return if true || 1/0==0 {3} else {2}}",
        "fn main():i32 {let x:i8=127;return (x+1) as i32}",
        "fn main():i32 {let x:i128=-170141183460469231731687303715884105728;return (x % -1) as i32}",
        "fn main():i32 {let x:i128=1;let count:i8=-1;return (x << count) as i32}",
        "fn main():i32 {let x:u8=255;let count:u128=256;return (x >> count) as i32}",
        "fn main():i32 {let x:f32=1.5;return x as i32}",
        "fn main():i32 {let x=0.0/0.0;return x as i32}",
        "fn main():i32 {let x=1.0/0.0;return x as i32}",
        "fn main():i32 {let x:f32=1.5;let wide=x as f64;return wide as i32}",
        "fn main():i32 {let x:u128=340282366920938463463374607431768211455;return (x / 3) as i32}",
        "fn main():i32 {let x=19.0;return (x % 4.0) as i32}",
    ];
    let dir = std::env::temp_dir().join(format!("nether-native-rust-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let compiler = std::env::var("NETHER_RUSTC").unwrap_or_else(|_| "rustc".into());
    let mut count = 0;
    for checks in [OverflowChecks::Checked, OverflowChecks::Wrapping] {
        let mut generated = String::new();
        let mut expected_cases = Vec::new();
        for (id, source) in cases.iter().enumerate() {
            let parsed = parse(SourceId(0), source);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            let program = check(&parsed.module.unwrap()).unwrap();
            let cfg = nether_core::mir::lower(&program);
            let expected = execute(&program, program.main.unwrap(), vec![], checks, 100_000);
            let mir_value = nether_core::mir::interpret::execute(
                &cfg,
                cfg.main.unwrap(),
                vec![],
                checks,
                100_000,
            );
            assert_eq!(
                expected.as_ref().map_err(|e| &e.kind),
                mir_value.as_ref().map_err(|e| &e.kind),
                "{source}"
            );
            let code = match &expected {
                Ok(Value::Integer(v)) => (v.bits() & 255) as i32,
                Ok(Value::Unit) => 0,
                Err(_) => 101,
                other => panic!("{other:?}"),
            };
            let original = Source::new("native.nr", *source);
            let location = expected
                .err()
                .map(|trap| original.location(trap.span.start).unwrap());
            let rust =
                emit_with_sources(&program, checks, std::slice::from_ref(&original)).unwrap();
            generated.push_str(&format!(
                "mod case{id} {{\n{}\n}}\n",
                rust.replace("fn main() {", "pub fn run() {")
            ));
            expected_cases.push((code, location));
        }
        generated.push_str(
            "fn main() { match std::env::args().nth(1).unwrap().parse::<usize>().unwrap() {\n",
        );
        for id in 0..cases.len() {
            generated.push_str(&format!("{id} => case{id}::run(),\n"));
        }
        generated.push_str("_ => panic!(\"invalid test case\"), } }\n");
        fs::write(dir.join("case.rs"), &generated).unwrap();
        for opt in ["opt-level=0", "opt-level=3"] {
            let output = Command::new(&compiler)
                .args(["--edition=2021", "-C", opt])
                .arg(dir.join("case.rs"))
                .arg("-o")
                .arg(dir.join("case"))
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}; source: {}",
                String::from_utf8_lossy(&output.stderr),
                dir.join("case.rs").display()
            );
            for (id, (code, location)) in expected_cases.iter().enumerate() {
                let output = Command::new(dir.join("case"))
                    .arg(id.to_string())
                    .output()
                    .unwrap();
                assert_eq!(
                    output.status.code(),
                    Some(*code),
                    "{}: {}",
                    cases[id],
                    String::from_utf8_lossy(&output.stderr)
                );
                if let Some((line, column)) = location {
                    assert!(
                        String::from_utf8_lossy(&output.stderr)
                            .contains(&format!("native.nr:{line}:{column}:")),
                        "{}: {}",
                        cases[id],
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
                count += 1;
            }
        }
    }
    eprintln!("{count} native Rust comparisons passed");
    fs::remove_dir_all(dir).unwrap();
}
