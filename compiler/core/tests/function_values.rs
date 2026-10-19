use nether_core::{check::check, hir::Value, interpret, mir};
use nether_frontend::{parser::parse, source::SourceId};
use nether_semantics::OverflowChecks;

#[test]
fn function_values_use_fixed_entries_and_propagate_panics() {
    for (source, expected) in [
        ("struct R {value:i32;destructor {}} fn take(r:R):i32 {return r.value} fn main():i32 {let f=take;return f(R {value:8})}",Some(8)),
        ("struct R {value:i32;destructor {}} fn id<T>(x:T):T {return x} fn apply<T>(f:(T)=>T,x:T):T {return f(x)} fn main():i32 {let f:(R)=>R=id;let r=apply(f,R {value:9});return r.value}",Some(9)),
        ("class C {value:i32;constructor(n:i32) {value=n}} fn id(x:C):C {return x} fn main():i32 {let f:(C)=>C=id;let a=new C(7);let b=f(a);return b.value}",Some(7)),

        ("fn identity<T>(x:T):T {return x} fn main():i32 {let f:(i32)=>i32=identity;return f(9)}",Some(9)),
        ("fn identity<T>(x:T):T {return x} fn main():i32 {let f:(i32)=>i32=identity<i32>;return f(9)}",Some(9)),
        ("fn first<T:Copy,const N:usize>(xs:{T;N}):T {return xs[0]} fn main():i32 {let f:({i32;2})=>i32=first;return f({8,9})}",Some(8)),
        ("fn identity<T>(x:T):T {return x} fn select():(i32)=>i32 {return identity} fn main():i32 {return select()(6)}",Some(6)),
        ("struct Math {fn add(n:i32):i32 {return n+2}} fn main():i32 {let f=Math.add;return f(3)}",Some(5)),
        ("struct Handler {run:(i32)=>i32;fn invoke(this):i32 {return run(4)}} fn add(n:i32):i32 {return n+2} fn main():i32 {let h=Handler {run:add};return h.invoke()}",Some(6)),
        ("fn add(n:i32):i32 {return n+2} fn main():i32 {let f:(i32)=>i32=add;return f(3)}",Some(5)),
        ("fn one():i32 {return 1} fn two():i32 {return 2} fn choose(b:bool):()=>i32 {return if b {one} else {two}} fn main():i32 {return choose(false)()}",Some(2)),
        ("fn bump(var n:i32):i32 {n+=1;return n} fn apply(f:(var i32)=>i32,n:i32):i32 {return f(n)} fn main():i32 {let n=3;return apply(bump,n)+n}",Some(7)),
        ("fn one():i32 {return 1} fn two():i32 {return 2} fn main():i32 {var f=one;let copy=f;f=two;return f()*10+copy()}",Some(21)),
        ("#[Copy] struct Handler {run:(i32)=>i32} fn add(n:i32):i32 {return n+2} fn main():i32 {let h=Handler {run:add};return h.run(3)}",Some(5)),
        ("fn add(n:i32):i32 {return n+2} fn apply<T:Copy>(f:(T)=>T,n:T):T {return f(n)} fn main():i32 {return apply(add,3)}",Some(5)),
        ("fn add(n:i32):i32 {return n+2} fn invoke<T:Copy>(f:()=>T):T {return f()} fn five():i32 {return 5} fn main():i32 {return invoke(five)}",Some(5)),
        ("fn yes(n:i32):bool {return n>0} fn main():i32 {let predicate=yes;return match 7 {x if predicate(x)=>x,_=>0}}",Some(7)),
        ("fn fail():i32 {return 1/0} fn main():i32 {let f=fail;return f()}",None),
    ] {
        let parsed=parse(SourceId(0),source);
        assert!(parsed.diagnostics.is_empty(),"{source}: {:?}",parsed.diagnostics);
        let p=check(&parsed.module.unwrap()).unwrap_or_else(|e|panic!("{source}: {e:?}"));
        let cfg=mir::lower(&p);
        let a=interpret::execute(&p,p.main.unwrap(),vec![],OverflowChecks::Checked,10000).map_err(|e|e.kind);
        let b=mir::interpret::execute(&cfg,cfg.main.unwrap(),vec![],OverflowChecks::Checked,10000).map_err(|e|e.kind);
        assert_eq!(a,b,"{source}");
        match expected {
            Some(expected) => {let Value::Integer(value)=a.unwrap() else {panic!()};assert_eq!(value.signed_value(),Some(expected));}
            None=>assert!(a.is_err()),
        }
        nether_core::rust::emit(&p,OverflowChecks::Checked).unwrap();
    }
}

#[test]
fn function_values_reject_signature_and_entry_mode_mismatches() {
    for source in [
        "fn bad<T>(x:T):T {return x+1} fn main():() {let f:(i32)=>i32=bad}",
        "fn identity<T>(x:T):T {return x} fn main():() {let f:(i32)=>i32=identity<bool>}",
        "fn f(n:i32):i32 {return n} fn main():() {let pointer:(bool)=>bool=f}",
        "fn f(var n:i32):i32 {return n} fn main():() {let pointer:(i32)=>i32=f}",
        "fn f(n:i32):i32 {return n} fn main():() {let pointer=f;pointer(true)}",
        "fn f(n:i32):i32 {return n} fn main():() {let pointer=f;pointer()}",
        "fn main():() {let f=1;f()}",
        "fn identity<T>(x:T):T {return x} fn main():() {let f=identity}",
    ] {
        let parsed = parse(SourceId(0), source);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        assert!(check(&parsed.module.unwrap()).is_err(), "accepted {source}");
    }
}
