use nether_core::{check::check, hir, interpret, mir, rust};
use nether_frontend::{modules::load, source::Diagnostic};
use nether_semantics::OverflowChecks;
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new(source: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "nether-storage-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("main.nr"), source).unwrap();
        Self(dir)
    }
    fn checked(&self) -> Result<hir::Program, Vec<Diagnostic>> {
        let loaded = load(&self.0.join("main.nr"), "app");
        if !loaded.diagnostics.is_empty() {
            return Err(loaded.diagnostics);
        }
        check(&loaded.module.unwrap())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
const IMPORT:&str="import {RawStorage,allocate,capacity,initialize,take,drop_slot,release,replace,size_of,align_of,slot_size,slot_align,move_within,move_slots,drop_range,view,view_mut,slot_address,slot_address_mut,drop_in_place,into_raw_parts,from_raw_parts,read_owned,write_owned} from \"core/intrinsic\"\n";
fn source(body: &str) -> String {
    format!("{IMPORT}{body}")
}
fn cases() -> Vec<(String, Option<i32>)> {
    let buffer = include_str!("../../../examples/scalar_buffer/main.nr");
    let mut result = vec![
        (source("fn main():i32 {unsafe {var s=allocate<i32>(4);initialize(s,0,2);initialize(s,1,5);move_within(s,1,0,2);move_within(s,0,1,2);let a=take(s,0);let b=take(s,1);release(s);return a+b}}"),Some(7)),
        (source("class R {v:i32;constructor(n:i32) {v=n};destructor {}} fn main():i32 {unsafe {var a=allocate<R>(1);var b=allocate<R>(1);initialize(a,0,new R(7));move_slots(b,0,a,0,1);let r=take(b,0);release(a);release(b);return r.v}}"),Some(7)),
        (source("struct R {v:i32;destructor {}} fn main():i32 {unsafe {var s=allocate<R>(1);initialize(s,0,R {v:7});let r=take(s,0);release(s);return r.v}}"),Some(7)),
        (source("class R {v:i32;constructor(n:i32) {v=n};destructor {}} fn main():i32 {unsafe {var s=allocate<R>(1);initialize(s,0,new R(7));drop_slot(s,0);release(s);return 7}}"),Some(7)),
        (source("fn main():i32 {unsafe {var s=allocate<(i32,i32)>(2);initialize(s,0,(2,5));let old=replace(s,0,(3,4));initialize(s,1,old);drop_slot(s,0);let pair=take(s,1);release(s);return pair.0+pair.1}}"),Some(7)),
        (source("fn main():i32 {unsafe {var s=allocate<{i32;3}>(1);initialize(s,0,{2,5,9});let a=take(s,0);release(s);return a[0]+a[1]}}"),Some(7)),
        (include_str!("../../../examples/raw_storage/main.nr").into(),Some(7)),
        (buffer.into(),Some(7)),
        (source("fn main():i32 {unsafe {var s=allocate<({i32;2},())>(1);initialize(s,0,({2,5},()));let a=take(s,0);release(s);return a.0[0]+a.0[1]}}"),Some(7)),
        (source("fn main():i32 {unsafe {var s=allocate<{();2}>(1);initialize(s,0,{(),()});drop_slot(s,0);release(s);return 7}}"),Some(7)),
        (buffer.replace("return buffer.pop()","buffer.push(1/0);return 0"),None),
        (source("fn main():i32 {unsafe {let s=allocate<i32>(0);let n=capacity(s);release(s);return n as i32}}"),Some(0)),
        (source("fn main():i32 {unsafe {var s=allocate<()>(2);initialize(s,0,());initialize(s,1,());let a=take(s,0);drop_slot(s,1);release(s);return 7}}"),Some(7)),
        (source("fn main():i32 {unsafe {var s=allocate<i32>(1);initialize(s,0,1);let n=take(s,0);initialize(s,0,6);return take(s,0)+n}}"),Some(7)),
        (source("fn main():i32 {unsafe {var s=allocate<i32>(1);var alias=s;initialize(alias,0,7);return take(alias,0)}}"),Some(7)),
        (source("fn main():i32 {unsafe {let s=allocate<i32>(2);return (size_of<i32>()+align_of<i32>()+slot_size(s)+slot_align(s)+capacity(s)) as i32}}"),Some(18)),
        (source("fn main():i32 {unsafe {let s=allocate<i32>(18446744073709551615);return 0}}"),None),
        (source("fn main():i32 {unsafe {let s=allocate<i32>(1);return 1/0}}"),None),
        (source("fn main():i32 {unsafe {var s=allocate<i32>(1);initialize(s,0,2);let old=replace(s,0,5);return old+take(s,0)}}"),Some(7)),
        (source("fn main():i32 {unsafe {var s=allocate<u128>(1);initialize(s,0,340282366920938463463374607431768211455);let n=take(s,0);return (n%10) as i32}}"),Some(5)),
    ];
    result.extend([
        (source("struct Item {v:i32;destructor {}} fn main():i32 {unsafe {var s=allocate<Item>(1);let p=slot_address_mut(s,0);write_owned(p,Item {v:7});let item=read_owned(p);return item.v/(item.v-7)}}"),None),

        (source("struct Item {v:i32;destructor {}} fn main():i32 {unsafe {var s=allocate<Item>(2);let p=slot_address_mut(s,1);write_owned(p,Item {v:7});let item=read_owned(p);release(s);return item.v}}"),Some(7)),
        (source("fn main():i32 {unsafe {var s=allocate<u32>(1);let p=slot_address_mut(s,0) as *var f32;write_owned(p,7.0);let n=read_owned(p);release(s);return n as i32}}"),Some(7)),
        (source("fn main():i32 {var n=2;let p=&var n;unsafe {let old=read_owned(p);write_owned(p,7)};return n}"),Some(7)),
        (source("fn main():i32 {var n=(2,3);let p=&var n.1;unsafe {let old=read_owned(p);write_owned(p,5)};return n.0+n.1}"),Some(7)),

        (source("fn main():i32 {unsafe {var s=allocate<u32>(1);initialize(s,0,1088421888);let p=slot_address_mut(s,0) as *var f32;let n=read_owned(p);release(s);return n as i32}}"),Some(7)),

        (source("class Item {v:i32;constructor(n:i32) {v=n};destructor {}} fn main():i32 {unsafe {var s=allocate<Item>(1);initialize(s,0,new Item(7));let item=read_owned(slot_address_mut(s,0));release(s);return item.v}}"),Some(7)),
        (source("struct Item {v:i32;destructor {}} fn main():i32 {unsafe {var s=allocate<Item>(2);initialize(s,1,Item {v:7});let p=slot_address_mut(s,0)+1;let item=read_owned(p);release(s);return item.v}}"),Some(7)),
        (source("enum Item {None,Some(i32)} fn main():i32 {unsafe {var s=allocate<Item>(1);initialize(s,0,Item.Some(7));let item=read_owned(slot_address_mut(s,0));release(s);return match item {Item.None=>0,Item.Some(n)=>n}}}"),Some(7)),
        (source("fn main():i32 {unsafe {var s=allocate<()>(1);initialize(s,0,());let n=read_owned(slot_address_mut(s,0));release(s);return 7}}"),Some(7)),

        (source("#[Copy] enum E {A,B} fn main():i32 {var n=E.A;unsafe {let p=(&var n) as *var u32;*p=1};return match n {E.A=>0,E.B=>7}}"),Some(7)),
        (source("#[Copy] enum E {A(u32),B(u32)} fn main():i32 {var n=E.A(2);unsafe {let p=(&var n) as *var {u32;2};*p={1,7}};return match n {E.A(n)=>0,E.B(n)=>n as i32}}"),Some(7)),

        (source("fn main():i32 {let n:u8=1;unsafe {let p=(&n) as *bool;if *p {return 7};return 0}}"),Some(7)),
        (source("fn main():i32 {var n='A';unsafe {let p=(&var n) as *var u32;*p=66;if n=='B' {return 7};return 0}}"),Some(7)),
        (source("fn main():i32 {var n:(bool,u8)=(false,7);unsafe {let p=(&var n) as *var u8;*p=1;if n.0 {return n.1 as i32};return 0}}"),Some(7)),

        (source("fn main():i32 {let a={2,7};unsafe {let all=(&a) as usize;let first=(&a[0]) as usize;let second=(&a[1]) as usize;if all==first && second-first==4 {return 7};return 0}}"),Some(7)),
        (source("#[Copy] struct Pair {a:u8;b:u32} fn main():i32 {let a=Pair {a:1,b:7};unsafe {let all=(&a) as usize;let field=(&a.b) as usize;let p=field as *u32;if field-all==4 {return *p as i32};return 0}}"),Some(7)),

        (source("fn main():i32 {unsafe {var s=allocate<i32>(2);initialize(s,0,2);initialize(s,1,7);let ref v=view(s,0);let p=&*v;let address=p as usize;let direct=slot_address(s,0) as usize;let n=*(p+1);drop_range(s,0,2);if address==direct {return n};return 0}}"),Some(7)),

        (source("unsafe fn item<T:Copy>(var s:RawStorage<T>):T {unsafe {return view_mut(s,0)}} fn main():i32 {unsafe {var s=allocate<i32>(1);initialize(s,0,2);let ref var v=item<i32>(s);*v=7;return take(s,0)}}"),Some(7)),
        (source("fn main():i32 {unsafe {var s=allocate<i32>(1);initialize(s,0,2);let n=if true {view(s,0)} else {view(s,0)};drop_slot(s,0);return n+5}}"),Some(7)),
        (source("fn main():i32 {unsafe {var s=allocate<i32>(1);initialize(s,0,7);var copy=view_mut(s,0);copy=0;let n=take(s,0);return n+copy}}"),Some(7)),
        (source("unsafe fn item(var s:RawStorage<i32>):i32 {unsafe {return view_mut(s,0)}} fn main():i32 {unsafe {var s=allocate<i32>(1);initialize(s,0,2);let f=item;let ref var v=f(s);*v=7;return take(s,0)}}"),Some(7)),

        (source("fn main():i32 {unsafe {var s=allocate<i32>(1);initialize(s,0,2);let ref var v=view_mut(s,0);*v=7;let n=view(s,0);drop_slot(s,0);return n}}"),Some(7)),
        (source("#[Copy] struct Pair {a:i32;b:i32} fn main():i32 {unsafe {var s=allocate<Pair>(1);initialize(s,0,Pair {a:2,b:3});let ref var v=view_mut(s,0);v.b=5;let n=view(s,0);drop_slot(s,0);return n.a+n.b}}"),Some(7)),
        (source("fn main():i32 {unsafe {var s=allocate<{i32;2}>(1);initialize(s,0,{2,3});let ref var v=view_mut(s,0);v[1]=5;let n=view(s,0);drop_slot(s,0);return n[0]+n[1]}}"),Some(7)),
        (source("#[Copy] enum E {None,Some(i32)} fn main():i32 {unsafe {var s=allocate<E>(1);initialize(s,0,E.None);let ref var v=view_mut(s,0);*v=E.Some(7);let n=view(s,0);drop_slot(s,0);return match n {E.None=>0,E.Some(n)=>n}}}"),Some(7)),
        (source("fn main():i32 {unsafe {var s=allocate<()>(2);initialize(s,0,());initialize(s,1,());let ref v=view(s,1);let n=*v;drop_range(s,0,2);return 7}}"),Some(7)),
        (source("fn main():i32 {unsafe {var s=allocate<i32>(1);initialize(s,0,2);let ref var v=view_mut(s,0);let p=&var *v;let old=take(s,0);initialize(s,0,7);let n=*p;drop_slot(s,0);return n}}"),Some(7)),
        ("import {OwnedBuffer} from \"core/storage\"\nfn main():i32 {var b=OwnedBuffer<i32>.make(1);b.push(2);let ref var v=b.at_mut(0);*v=7;let n=b.at(0);b.clear();return n}".into(),Some(7)),

        (source("fn address(p:*var i32,count:*var i32):*var i32 {unsafe {*count+=1};return p} fn rhs(p:*var i32):i32 {unsafe {*p=99};return 5} fn main():i32 {var n=2;var count=0;unsafe {*address(&var n,&var count)+=rhs(&var n)};if count==1 {return n};return 0}"),Some(7)),
        (source("fn main():i32 {unsafe {var s=allocate<i32>(1);initialize(s,0,2);*slot_address_mut(s,0)+=5;return take(s,0)}}"),Some(7)),
        (source("fn main():i32 {var n=2;unsafe {let a=(&var n) as isize;let p=a as *var i32;*p=7;return n}}"),Some(7)),
        (source("fn main():i32 {unsafe {let a:isize=-1;let p=a as *i32;let b=p as isize;if b==a {return 7};return 0}}"),Some(7)),
        (source("fn main():i32 {unsafe {let s=allocate<i32>(1);let parts=into_raw_parts(s);let address=parts.0 as usize;let p=address as *var i32;var back=from_raw_parts(p,parts.1);initialize(back,0,7);return take(back,0)}}"),Some(7)),

        (source("fn main():i32 {unsafe {var s=allocate<u32>(2);initialize(s,0,2);initialize(s,1,7);let addr=slot_address(s,0) as usize;let p=(addr+4) as *u32;let n=*p;drop_range(s,0,2);return n as i32}}"),Some(7)),
        (source("fn main():i32 {var n=2;unsafe {let a=(&var n) as usize;let p=a as *var i32;*p=7;return n}}"),Some(7)),
        (source("#[Copy] struct Pair {a:u32;b:u32} fn main():i32 {var value=Pair {a:1,b:2};unsafe {let p=(&var value) as *var u32;*(p+1)=6;return (value.a+value.b) as i32}}"),Some(7)),
        (source("fn main():i32 {var value:(u32,u32)=(1,2);unsafe {let p=(&var value) as *var {u32;2};*p={3,4};return (value.0+value.1) as i32}}"),Some(7)),
        (source("fn main():i32 {var value:{u32;8}={1,2,3,4,5,6,7,8};unsafe {let p=(&var value) as *var u32;*(p+7)=0;let q=p as *{u32;8};let copied=*q;return (copied[6]+copied[7]) as i32}}"),Some(7)),
        (source("#[Copy] struct Pair {a:u32;b:u32} fn main():i32 {unsafe {var s=allocate<Pair>(1);initialize(s,0,Pair {a:2,b:5});let p=slot_address(s,0) as *(u32,u32);let pair=*p;drop_slot(s,0);return (pair.0+pair.1) as i32}}"),Some(7)),
        (source("struct Set {p:*var i32;destructor {unsafe {*p=7}}} fn main():i32 {var n=1;if true {let guard=Set {p:&var n}};if n==7 {return 7};return 0}"),Some(7)),
        (source("fn main():i32 {var n=1;let p=&var n;unsafe {*p=7};if n==7 {return 7};return 0}"),Some(7)),
        (source("fn main():i32 {var a:u64=506381209866536711;unsafe {let p=(&var a) as *var u8;let q=(p+4) as *var u32;*q=0;if a!=0 && a!=506381209866536711 {return 7};return 0}}"),Some(7)),
        (source("fn main():i32 {unsafe {var s=allocate<u32>(2);initialize(s,0,0);initialize(s,1,0);let p=slot_address_mut(s,0) as *var u8;*(p+4)=7;*(p+5)=7;*(p+6)=7;*(p+7)=7;let a=take(s,0);let b=take(s,1);if a==0 && b==117901063 {return 7};return 0}}"),Some(7)),
        (source("fn main():i32 {var a:u64=0;unsafe {let p=(&var a) as *var u8;let q=(p+4) as *var u32;*q=117901063;return *(p+4) as i32}}"),Some(7)),
        (source("fn main():i32 {unsafe {let s=allocate<u64>(1);let parts=into_raw_parts(s);let bytes=parts.0 as *var u8;let p=bytes as *var u64;var back=from_raw_parts(p,parts.1);initialize(back,0,7);return take(back,0) as i32}}"),Some(7)),
        (source("fn main():i32 {var f:f32=0.0;unsafe {let p=(&var f) as *var u32;*p=2147483648;let q=(&f) as *i32;if *q==(-2147483647-1) {return 7};return 0}}"),Some(7)),
        (source("fn main():i32 {let a:{u32;2}={0,1088421888};unsafe {let p=(&a[0]) as *f32;let end=p+2;return *(end-1) as i32}}"),Some(7)),
        (source("fn main():i32 {unsafe {var s=allocate<u32>(1);initialize(s,0,1065353216);let p=slot_address_mut(s,0) as *var f32;*p=7.0;let bits=take(s,0);release(s);if bits==1088421888 {return 7};return 0}}"),Some(7)),
        (source("fn main():i32 {var bits:u64=4619567317775286272;unsafe {let p=(&var bits) as *var f64;let q=p as *var u64;*q=4619567317775286272;return *p as i32}}"),Some(7)),
        (source("fn main():i32 {unsafe {var s=allocate<i32>(2);initialize(s,1,7);let parts=into_raw_parts(s);var adopted=from_raw_parts(parts.0,parts.1);let n=take(adopted,1);release(adopted);return n}}"),Some(7)),
        (source("class Item {value:i32;constructor(n:i32) {value=n}} fn main():i32 {unsafe {var s=allocate<Item>(1);initialize(s,0,new Item(7));let parts=into_raw_parts(s);var adopted=from_raw_parts(parts.0,parts.1);let n=view(adopted,0).value;drop_slot(adopted,0);return n}}"),Some(7)),
        (source("fn main():i32 {unsafe {let s=allocate<()>(0);let parts=into_raw_parts(s);let adopted=from_raw_parts(parts.0,parts.1);return 7}}"),Some(7)),
        (source("#[Copy] struct Pair {a:i32;b:i32} fn main():i32 {var value=Pair {a:2,b:3};let p=&var value;unsafe {*p=Pair {a:3,b:4};let v=*p;return v.a+v.b}}"),Some(7)),
        (source("#[Copy] enum E {None,Some(i32)} fn main():i32 {var value=E.None;let p=&var value;unsafe {*p=E.Some(7);let v=*p;return match v {E.None=>0,E.Some(n)=>n}}}"),Some(7)),
        (source("fn main():i32 {var value=();let p=&var value;unsafe {*p=();let v=*p};return 7}"),Some(7)),
        (source("class R {v:i32;constructor(n:i32) {v=n};destructor {}} fn main():i32 {unsafe {var s=allocate<R>(2);initialize(s,1,new R(7));let first=slot_address_mut(s,0);let p=first+1;let n=p-first;let readonly=p as *R;drop_in_place(p);release(s);return 6+(n as i32)}}"),Some(7)),
        (source("struct R {v:i32;destructor {let bad=1/0}} fn main():i32 {unsafe {var s=allocate<R>(2);initialize(s,1,R {v:7});drop_in_place(slot_address_mut(s,1));return 7}}"),None),
        (source("fn main():i32 {var n=2;let p=&var n;unsafe {drop_in_place(p)};return 7}"),Some(7)),
        (source("fn main():i32 {unsafe {var s=allocate<i32>(2);initialize(s,0,2);initialize(s,1,5);let p=slot_address_mut(s,1);*p=7;let first=slot_address(s,0);let end=slot_address(s,2);let n=*(end-1);drop_range(s,0,2);return n+(end-first) as i32}}"),Some(9)),
        (source("fn main():i32 {unsafe {var s=allocate<()>(0);let p=slot_address(s,0);return 7}}"),Some(7)),
        (source("fn main():i32 {unsafe {var s=allocate<i32>(0);let p=slot_address(s,1);return 7}}"),None),
        ("import {OwnedBuffer} from \"core/storage\"\nclass Item {value:i32;constructor(n:i32) {value=n}} fn main():i32 {var b=OwnedBuffer<Item>.make(2);b.push(new Item(2));var item=b.at_mut(0);item.value=7;return b.at(0).value}".into(),Some(7)),
        ("import {OwnedBuffer} from \"core/storage\"\nclass Item {value:i32;constructor(n:i32) {value=n}} fn main():i32 {var b=OwnedBuffer<Item>.make(2);b.push(new Item(2));return b.at(1).value}".into(),None),
        (source("class Item {value:i32;constructor(n:i32) {value=n}} fn main():i32 {unsafe {var s=allocate<Item>(1);initialize(s,0,new Item(2));var item=view_mut(s,0);item.value=7;let n=view(s,0).value;drop_slot(s,0);return n}}"),Some(7)),
        (include_str!("../../../examples/owned_buffer/main.nr").into(),Some(7)),
        ("import {OwnedBuffer,BufferResult} from \"core/storage\"\nfn main():i32 {var buffer=OwnedBuffer<i32>.make(1);buffer.push(3);let result=buffer.push(7);buffer.clear();return match result {BufferResult.Some(n)=>n,BufferResult.None=>0}}".into(),Some(7)),
        ("import {OwnedBuffer} from \"core/storage\"\nclass Item {v:i32;constructor(n:i32) {v=n};destructor {if v==1 {let bad=1/0}}} fn main():i32 {var buffer=OwnedBuffer<Item>.make(3);buffer.push(new Item(1));buffer.push(new Item(2));buffer.push(new Item(3));return 7}".into(),None),
        (source("class R {v:i32;constructor(n:i32) {v=n};destructor {}} fn main():i32 {unsafe {var s=allocate<R>(1);initialize(s,0,new R(2));let old=replace(s,0,new R(5));let current=take(s,0);release(s);return old.v+current.v}}"),Some(7)),
        (source("struct R {v:i32;destructor {let bad=1/0}} fn main():i32 {unsafe {var s=allocate<R>(1);initialize(s,0,R {v:7});drop_slot(s,0);return 7}}"),None),
        (source("struct Z {destructor {}} fn main():i32 {unsafe {var s=allocate<Z>(2);initialize(s,0,Z {});move_within(s,1,0,1);drop_slot(s,1);release(s);return 7}}"),Some(7)),
        (source("fn main():i32 {unsafe {var s=allocate<i32>(0);move_within(s,0,0,0);release(s);return 7}}"),Some(7)),
        (source("fn main():i32 {unsafe {var s=allocate<i32>(0);move_within(s,0,0,18446744073709551615);return 7}}"),None),
    ]);
    result
}
#[test]
fn scalar_storage_reference_lifecycle_layout_and_unwind() {
    for (source, expected) in cases() {
        let files = Fixture::new(&source);
        let p = files
            .checked()
            .unwrap_or_else(|e| panic!("{source}: {e:?}"));
        let m = mir::lower(&p);
        for mode in [OverflowChecks::Checked, OverflowChecks::Wrapping] {
            let a = interpret::execute(&p, p.main.unwrap(), vec![], mode, 100000);
            let b = mir::interpret::execute(&m, m.main.unwrap(), vec![], mode, 100000);
            assert_eq!(
                a.as_ref().map_err(|e| &e.kind),
                b.as_ref().map_err(|e| &e.kind),
                "{source}"
            );
            match expected {
                Some(n) => assert!(
                    matches!(a,Ok(hir::Value::Integer(i)) if i.signed_value()==Some(n as i128)),
                    "{source}: {a:?}"
                ),
                None => assert!(a.is_err(), "{source}"),
            }
            rust::emit(&p, mode).unwrap();
        }
    }
}
#[test]
fn storage_symbols_ownership_privacy_and_safety_are_enforced() {
    for body in [
        "fn f(s:RawStorage<i32>):(*var i32,usize) {return into_raw_parts(s)} fn main():i32 {return 0}",
        "fn main():i32 {unsafe {let s=allocate<i32>(0);let alias=s;let parts=into_raw_parts(alias)};return 0}",
        "fn main():i32 {unsafe {let s=allocate<i32>(0);let parts=into_raw_parts(s);return capacity(s) as i32}}",
        "class R {v:i32;constructor() {v=7}} fn main():i32 {unsafe {var s=allocate<R>(1);initialize(s,0,new R());let v=view(s,0);let parts=into_raw_parts(s);return v.v}}",
        "fn main():i32 {var n=7;drop_in_place(&var n);return 0}",
        "fn main():i32 {var n=7;unsafe {drop_in_place(&n)};return 0}",
        "fn main():i32 {unsafe {let s=allocate<i32>(1);let p=slot_address_mut(s,0)};return 0}",
        "fn f(s:RawStorage<i32>):*i32 {return slot_address(s,0)} fn main():i32 {return 0}",
        "fn main():i32 {unsafe {var s=allocate<i32>(1);let p=slot_address(s,0);*p=7};return 0}",
        "struct R {n:i32} struct S {r:R} fn main():i32 {let target=R {n:7};let alias=target;let item=S {r:alias};unsafe {var storage=allocate<S>(1);initialize(storage,0,item)};return 0}",
        "struct R {v:i32} fn main():i32 {unsafe {var s=allocate<R>(1);let r=R {v:7};let alias=r;initialize(s,0,alias)};return 0}",
        "struct R {v:i32} fn main():i32 {unsafe {var s=allocate<R>(1);let r=R {v:7};initialize(s,0,r);return r.v}}",
        "fn main():i32 {let s=allocate<i32>(1);return 0}",
        "fn main():i32 {unsafe {let s=allocate<i32>(1);let alias=s;release(alias)};return 0}",
        "fn main():i32 {unsafe {let s=allocate<i32>(1);release(s);return capacity(s) as i32}}",
        "fn main():i32 {unsafe {var s=allocate<i32>(1);let alias=s;initialize(alias,0,7)};return 0}",
        "fn main():i32 {unsafe {let s=allocate<i32>(1);let p=s.data};return 0}",
        "fn main():i32 {let n=7;let fake=RawStorage<i32> {data:&var n,cap:1};return 0}",




    ] {let files=Fixture::new(&source(body));assert!(files.checked().is_err(),"accepted: {body}");}
    let files =
        Fixture::new("import {heap_allocate} from \"core/intrinsic\"\nfn main():i32 {return 0}");
    assert!(files
        .checked()
        .unwrap_err()
        .iter()
        .any(|e| e.code == "E0504"));
    let files = Fixture::new(
        "fn heap_allocate<T>(n:usize):i32 {return 7} fn main():i32 {return heap_allocate<i32>(1)}",
    );
    assert!(files.checked().is_ok());
}
#[test]
fn interpreter_rejects_invalid_unsafe_slot_protocols() {
    // Deliberately invalid unsafe programs are never executed natively.
    for body in [
        "fn main():i32 {unsafe {var s=allocate<i32>(1);let p=slot_address_mut(s,0);write_owned(p,7);write_owned(p,2);return 0}}",

        "fn main():i32 {unsafe {var s=allocate<i32>(1);initialize(s,0,7);let p=slot_address_mut(s,0);let a=read_owned(p);return read_owned(p)}}",
        "fn main():i32 {unsafe {var s=allocate<i32>(1);return read_owned(slot_address_mut(s,1))}}",

        "fn main():i32 {unsafe {var s=allocate<i32>(1);initialize(s,0,7);let p=slot_address_mut(s,0);let n=take(s,0);*p=2;return n}}",
        "fn main():i32 {unsafe {var s=allocate<i32>(1);initialize(s,0,7);let address=slot_address(s,0) as usize;drop_slot(s,0);release(s);let p=address as *i32;return *p}}",
        "fn main():i32 {unsafe {var s=allocate<i32>(1);initialize(s,0,7);let ref v=view(s,0);let p=&*v;drop_slot(s,0);release(s);return *p}}",

        "fn main():i32 {unsafe {let s=allocate<u32>(1);let parts=into_raw_parts(s);let p=parts.0 as *var f32;let bad=from_raw_parts(p,parts.1);return 0}}",
        "fn main():i32 {unsafe {let s=allocate<i32>(0);let parts=into_raw_parts(s);let a=from_raw_parts(parts.0,parts.1);let b=from_raw_parts(parts.0,parts.1);return 0}}",
        "fn main():i32 {unsafe {let s=allocate<i32>(1);let parts=into_raw_parts(s);let a=from_raw_parts(parts.0,0);return 0}}",
        "fn main():i32 {unsafe {var s=allocate<i32>(1);drop_in_place(slot_address_mut(s,0))};return 0}",
        "fn main():i32 {unsafe {var s=allocate<i32>(1);initialize(s,0,7);let p=slot_address_mut(s,0);drop_in_place(p);drop_in_place(p)};return 0}",
        "fn main():i32 {unsafe {var s=allocate<i32>(1);let p=slot_address_mut(s,1);drop_in_place(p)};return 0}",
        "fn main():i32 {unsafe {var s=allocate<i32>(1);let p=slot_address(s,0);release(s);return *p}}",
        "fn main():i32 {unsafe {var s=allocate<i32>(1);var t=allocate<i32>(1);return (slot_address(s,0)-slot_address(t,0)) as i32}}",
        "fn main():i32 {unsafe {var s=allocate<i32>(1);return take(s,0)}}",
        "fn main():i32 {unsafe {var s=allocate<i32>(1);initialize(s,0,7);initialize(s,0,9)};return 0}",
        "fn main():i32 {unsafe {var s=allocate<i32>(1);initialize(s,1,7)};return 0}",
        "fn main():i32 {unsafe {var s=allocate<i32>(1);initialize(s,0,7);drop_slot(s,0);drop_slot(s,0)};return 0}",
        "fn main():i32 {unsafe {var s=allocate<i32>(1);initialize(s,0,7);release(s)};return 0}",
    ] {
        let files=Fixture::new(&source(body));let p=files.checked().unwrap();let m=mir::lower(&p);
        let a=interpret::execute(&p,p.main.unwrap(),vec![],OverflowChecks::Checked,100000).unwrap_err();
        let b=mir::interpret::execute(&m,m.main.unwrap(),vec![],OverflowChecks::Checked,100000).unwrap_err();
        assert_eq!(a.kind,b.kind,"{body}");
    }
}

#[test]
fn native_storage_releases_allocations_on_return_panic_and_oom() {
    let mut cases = cases();
    cases.push((source("fn main():i32 {unsafe {let first=allocate<i32>(2);let second=allocate<i32>(3);return 7}}"),Some(7)));
    let files = Fixture::new("");
    let counter = r#"
use std::sync::atomic::{AtomicIsize,AtomicUsize,Ordering};
struct Counting;
static LIVE:AtomicIsize=AtomicIsize::new(0);
static ALLOCATED:AtomicUsize=AtomicUsize::new(0);
static FAIL_AFTER:AtomicIsize=AtomicIsize::new(-1);
unsafe impl std::alloc::GlobalAlloc for Counting {
 unsafe fn alloc(&self,l:std::alloc::Layout)->*mut u8 {
  if FAIL_AFTER.load(Ordering::SeqCst)>=0 && FAIL_AFTER.fetch_sub(1,Ordering::SeqCst)==0 {return std::ptr::null_mut();}
  let p=std::alloc::GlobalAlloc::alloc(&std::alloc::System,l);
  if !p.is_null() {LIVE.fetch_add(1,Ordering::SeqCst);ALLOCATED.fetch_add(1,Ordering::SeqCst);}
  p
 }
 unsafe fn dealloc(&self,p:*mut u8,l:std::alloc::Layout) {
  LIVE.fetch_sub(1,Ordering::SeqCst);std::alloc::GlobalAlloc::dealloc(&std::alloc::System,p,l)
 }
}
#[global_allocator] static ALLOCATOR:Counting=Counting;
"#;
    for mode in [OverflowChecks::Checked, OverflowChecks::Wrapping] {
        let mut generated = counter.to_string();
        for (id, (source, expected)) in cases.iter().enumerate() {
            let fixture = Fixture::new(source);
            let p = fixture.checked().unwrap();
            // Keep allocation sites observable for fault injection: Rust may
            // otherwise elide unused allocations even with a counting allocator.
            let code = rust::emit(&p, mode)
                .unwrap()
                .replace(
                    "std::alloc::alloc(layout)",
                    "std::hint::black_box(std::alloc::alloc(layout))",
                )
                .replace("fn main() {", "fn unused_main() {");
            generated.push_str(&format!("mod case{id} {{\n{code}\n"));
            generated.push_str(&format!(
                r#"
pub fn check(fail:isize) {{
 let before=super::LIVE.load(super::Ordering::SeqCst);
 let allocated=super::ALLOCATED.load(super::Ordering::SeqCst);
 super::FAIL_AFTER.store(fail,super::Ordering::SeqCst);
 let result=unsafe{{f{}({})}};
 super::FAIL_AFTER.store(-1,super::Ordering::SeqCst);
 let after=super::LIVE.load(super::Ordering::SeqCst);
 let allocated_after=super::ALLOCATED.load(super::Ordering::SeqCst);
 assert_eq!(before,after,"allocation leak in case {id}");
 if fail>=0 {{assert!(result.is_err(),"expected OOM case {id}, fail={{fail}}");}} else {{ {} }}
 {}
}}
}}
"#,
                p.main.unwrap(),
                if code.contains("struct Context") {
                    "&mut Context{clock:0,panicking:false}"
                } else {
                    ""
                },
                expected.map_or(
                    format!("assert!(result.is_err(),\"expected panic case {id}\");"),
                    |n| format!("assert_eq!(result.unwrap(),{n});")
                ),
                if id == 11 || id == 12 {
                    "assert_eq!(allocated,allocated_after);"
                } else {
                    ""
                }
            ));
        }
        generated.push_str("fn main(){\n");
        for id in 0..cases.len() {
            generated.push_str(&format!("case{id}::check(-1);\n"));
        }
        generated.push_str(&format!(
            "case{}::check(0);case{}::check(1);}}",
            cases.len() - 1,
            cases.len() - 1
        ));
        fs::write(files.0.join("native.rs"), generated).unwrap();
        for opt in ["opt-level=0", "opt-level=3"] {
            let output =
                Command::new(std::env::var("NETHER_RUSTC").unwrap_or_else(|_| "rustc".into()))
                    .args(["--edition=2021", "-C", opt])
                    .arg(files.0.join("native.rs"))
                    .arg("-o")
                    .arg(files.0.join("native"))
                    .output()
                    .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let output = Command::new(files.0.join("native")).output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}

#[test]
fn storage_metadata_survives_reexports_and_verifier_rejects_corruption() {
    let files=Fixture::new("import {allocate,initialize,take,release} from \"./api\"\nfn main():i32 {unsafe {var s=allocate<i32>(1);initialize(s,0,7);let n=take(s,0);release(s);return n}}");
    fs::write(
        files.0.join("api.nr"),
        "export {allocate,initialize,take,release} from \"core/intrinsic\"",
    )
    .unwrap();
    let p = files.checked().unwrap();
    let original = mir::lower(&p);
    mir::verify::verify(&original).unwrap();
    let hir::Value::Integer(n) =
        interpret::execute(&p, p.main.unwrap(), vec![], OverflowChecks::Checked, 100000).unwrap()
    else {
        panic!()
    };
    assert_eq!(n.signed_value(), Some(7));
    let mut bad = original.clone();
    for f in &mut bad.functions {
        for b in &f.blocks {
            if let mir::Terminator::Evaluate {
                operation: mir::Operation::Unary("storage_allocate", _),
                destination,
                ..
            } = &b.terminator
            {
                let hir::Type::Pointer { mutable, .. } = &mut f.locals[destination.local].ty else {
                    panic!()
                };
                *mutable = false;
            }
        }
    }
    assert!(mir::verify::verify(&bad).is_err());
    assert!(rust::emit_mir(&bad, OverflowChecks::Checked).is_err());
    let mut bad = original;
    for f in &mut bad.functions {
        for b in &f.blocks {
            if let mir::Terminator::Evaluate {
                operation: mir::Operation::Unary("storage_take", _),
                destination,
                ..
            } = &b.terminator
            {
                f.locals[destination.local].ty = hir::Type::Bool;
            }
        }
    }
    assert!(mir::verify::verify(&bad).is_err());
    assert!(rust::emit_mir(&bad, OverflowChecks::Checked).is_err());
}

#[test]
fn owned_slots_run_destructors_once_in_transfer_order() {
    let files = Fixture::new(&source("struct R {v:i32;destructor {}} fn main():i32 {unsafe {var s=allocate<R>(3);initialize(s,0,R {v:1});initialize(s,1,R {v:2});move_within(s,1,0,2);drop_slot(s,1);let old=replace(s,2,R {v:3});drop_slot(s,2);release(s);return old.v}}"));
    let p = files.checked().unwrap();
    let m = mir::lower(&p);
    for mode in [OverflowChecks::Checked, OverflowChecks::Wrapping] {
        let a = interpret::execute_traced(&p, p.main.unwrap(), vec![], mode, 100000);
        let b = mir::interpret::execute_traced(&m, m.main.unwrap(), vec![], mode, 100000);
        assert!(a.0.is_ok());
        assert!(b.0.is_ok());
        for events in [a.1, b.1] {
            let values: Vec<_> = events
                .iter()
                .filter_map(|event| {
                    let hir::Value::Aggregate(fields) = &event.value else {
                        return None;
                    };
                    let [hir::Value::Integer(n)] = fields.as_slice() else {
                        return None;
                    };
                    n.signed_value()
                })
                .collect();
            assert_eq!(values, vec![1, 3, 2]);
        }
    }
}

#[test]
fn buffer_cleanup_continues_after_first_panic_and_stops_after_second() {
    for (condition, expected_kind, expected_values) in [
        (
            "v==1",
            interpret::TrapKind::Arithmetic(nether_semantics::ArithmeticError::DivisionByZero),
            vec![1, 2, 3],
        ),
        ("v<3", interpret::TrapKind::DoublePanic, vec![1, 2]),
    ] {
        let code=format!("import {{OwnedBuffer}} from \"core/storage\"\nstruct Item {{v:i32;destructor {{if {condition} {{let bad=1/0}}}}}} fn main():i32 {{var buffer=OwnedBuffer<Item>.make(3);buffer.push(Item {{v:1}});buffer.push(Item {{v:2}});buffer.push(Item {{v:3}});return 0}}");
        let files = Fixture::new(&code);
        let p = files.checked().unwrap();
        let m = mir::lower(&p);
        for mode in [OverflowChecks::Checked, OverflowChecks::Wrapping] {
            for (result, events) in [
                interpret::execute_traced(&p, p.main.unwrap(), vec![], mode, 100000),
                mir::interpret::execute_traced(&m, m.main.unwrap(), vec![], mode, 100000),
            ] {
                assert_eq!(result.unwrap_err().kind, expected_kind);
                let values: Vec<_> = events
                    .iter()
                    .filter_map(|event| {
                        let hir::Value::Aggregate(fields) = &event.value else {
                            return None;
                        };
                        let [hir::Value::Integer(n)] = fields.as_slice() else {
                            return None;
                        };
                        n.signed_value()
                    })
                    .collect();
                assert_eq!(values, expected_values);
            }
        }
    }
}

#[test]
fn safe_buffer_preserves_privacy_and_requires_owners() {
    for body in [
        "struct Item {v:i32} fn main():i32 {var b=OwnedBuffer<Item>.make(1);let owner=Item {v:7};let alias=owner;b.push(alias);return 0}",
        "fn main():i32 {var b=OwnedBuffer<i32>.make(1);let hidden=b.storage;return 0}",
        "fn main():i32 {let b=OwnedBuffer<i32>.make(1);b.push(7);return 0}",
    ] {
        let files=Fixture::new(&format!("import {{OwnedBuffer}} from \"core/storage\"\n{body}"));
        assert!(files.checked().is_err(),"accepted {body}");
    }
}

#[test]
fn storage_views_keep_capability_and_owner_lifetime() {
    let prefix = "class Item {value:i32;constructor(n:i32) {value=n}} ";
    for body in [
        "fn main():i32 {unsafe {var s=allocate<Item>(1);initialize(s,0,new Item(2));let v=view(s,0);drop_slot(s,0);return v.value}}",
        "fn main():i32 {unsafe {var s=allocate<Item>(1);initialize(s,0,new Item(2));let v=view(s,0);release(s);return v.value}}",
        "fn main():i32 {unsafe {var s=allocate<Item>(1);initialize(s,0,new Item(2));let v=view(s,0);v.value=7;return 0}}",
        "fn escape():Item {unsafe {var s=allocate<Item>(1);initialize(s,0,new Item(2));return view(s,0)}} fn main():i32 {return escape().value}",
        "fn main():i32 {unsafe {var s=allocate<Item>(1);initialize(s,0,new Item(2));var a=view_mut(s,0);var b=view_mut(s,0);b.value=3;return a.value}}",
    ] {
        let files=Fixture::new(&source(&format!("{prefix}{body}")));
        assert!(files.checked().is_err(),"accepted {body}");
    }
    let files=Fixture::new(&source(&format!("{prefix} fn main():i32 {{unsafe {{var s=allocate<Item>(1);initialize(s,0,new Item(7));return view(s,0).value}}}}")));
    let p = files.checked().unwrap();
    let mut m = mir::lower(&p);
    mir::verify::verify(&m).unwrap();
    for f in &mut m.functions {
        for b in &mut f.blocks {
            if let mir::Terminator::Evaluate {
                operation: mir::Operation::StorageView { contract, .. },
                ..
            } = &mut b.terminator
            {
                contract.origins.clear();
            }
        }
    }
    assert!(mir::verify::verify(&m).is_err());
    assert!(rust::emit_mir(&m, OverflowChecks::Checked).is_err());
}

#[test]
fn verifier_checks_raw_address_drop_and_difference_contracts() {
    let files=Fixture::new(&source("fn main():i32 {unsafe {var s=allocate<i32>(1);initialize(s,0,7);let p=slot_address(s,0);let q=slot_address_mut(s,0);let d=q-p;drop_in_place(q);let parts=into_raw_parts(s);let adopted=from_raw_parts(parts.0,parts.1);return 7+(d as i32)}}"));
    let p = files.checked().unwrap();
    let original = mir::lower(&p);
    mir::verify::verify(&original).unwrap();
    for target in [
        "storage_address",
        "raw_drop",
        "raw_difference",
        "storage_detach",
        "storage_adopt",
    ] {
        let mut bad = original.clone();
        let mut changed = false;
        for f in &mut bad.functions {
            for block in &f.blocks {
                let mir::Terminator::Evaluate {
                    operation,
                    destination,
                    ..
                } = &block.terminator
                else {
                    continue;
                };
                match operation {
                    mir::Operation::Unary("storage_address", _) if target == "storage_address" => {
                        let hir::Type::Pointer { mutable, .. } =
                            &mut f.locals[destination.local].ty
                        else {
                            panic!()
                        };
                        *mutable = true;
                        changed = true;
                    }
                    mir::Operation::Unary(
                        op @ ("raw_drop" | "storage_adopt"),
                        mir::Operand::Local(args),
                    ) if target == *op => {
                        let hir::Type::Tuple(fields) = &mut f.locals[*args].ty else {
                            panic!()
                        };
                        let hir::Type::Pointer { mutable, .. } = &mut fields[0] else {
                            panic!()
                        };
                        *mutable = false;
                        changed = true;
                    }
                    mir::Operation::Unary("storage_detach", _) if target == "storage_detach" => {
                        f.locals[destination.local].ty = hir::Type::Bool;
                        changed = true;
                    }
                    mir::Operation::Binary("raw_difference", _, _)
                        if target == "raw_difference" =>
                    {
                        f.locals[destination.local].ty = hir::Type::Bool;
                        changed = true;
                    }
                    _ => {}
                }
            }
        }
        assert!(changed, "missing {target}");
        assert!(
            mir::verify::verify(&bad).is_err(),
            "accepted corrupted {target}"
        );
        assert!(rust::emit_mir(&bad, OverflowChecks::Checked).is_err());
    }
}

#[test]
fn copy_storage_views_preserve_loans_and_cannot_escape() {
    for body in [
        "unsafe fn item(s:RawStorage<i32>):i32 {unsafe {return view(s,0)}} fn main():i32 {unsafe {var s=allocate<i32>(1);initialize(s,0,2);let f=item;let ref var v=f(s);*v=7;return take(s,0)}}",
        "fn number():i32 {return 7} fn main():i32 {let f=number;let ref v=f();return *v}",

        "fn main():i32 {unsafe {var s=allocate<i32>(1);initialize(s,0,7);let ref v=view(s,0);drop_slot(s,0);return *v}}",
        "fn main():i32 {unsafe {var s=allocate<i32>(1);initialize(s,0,7);let ref v=view(s,0);*v=2;return 0}}",
        "fn main():i32 {unsafe {var s=allocate<i32>(1);initialize(s,0,7);let ref var a=view_mut(s,0);let ref var b=view_mut(s,0);*b=2;return *a}}",
        "fn escape():i32 {unsafe {var s=allocate<i32>(1);initialize(s,0,7);return view(s,0)}} fn main():i32 {return escape()}",
        "fn main():i32 {unsafe {var s=allocate<i32>(1);initialize(s,0,7);let ref v=view(s,0);let n=take(s,0);return *v}}",
    ] {
        let files=Fixture::new(&source(body));
        assert!(files.checked().is_err(),"accepted {body}");
    }
}

#[test]
fn raw_owned_transfers_require_unsafe_owners_and_writable_access() {
    for body in [
        "struct Item {v:i32} fn main():i32 {unsafe {var s=allocate<Item>(1);let item=Item {v:7};let alias=item;write_owned(slot_address_mut(s,0),alias);return 0}}",
        "fn main():i32 {var n=7;let p=&n;unsafe {write_owned(p,2)};return 0}",

        "fn main():i32 {var n=7;let p=&var n;return read_owned(p)}",
        "fn main():i32 {let n=7;let p=&n;unsafe {return read_owned(p)}}",
        "fn main():i32 {unsafe {var s=allocate<i32>(1);initialize(s,0,7);return read_owned(slot_address(s,0))}}",
    ] { let files=Fixture::new(&source(body)); assert!(files.checked().is_err(),"accepted {body}"); }
}

#[test]
fn owning_raw_transfers_keep_one_cleanup_and_verified_types() {
    let files=Fixture::new(&source("struct Item {v:i32;destructor {}} fn main():i32 {unsafe {var s=allocate<Item>(1);let p=slot_address_mut(s,0);write_owned(p,Item {v:7});let item=read_owned(p);release(s);return item.v}}"));
    let p = files.checked().unwrap();
    let original = mir::lower(&p);
    for (result, events) in [
        interpret::execute_traced(&p, p.main.unwrap(), vec![], OverflowChecks::Checked, 100000),
        mir::interpret::execute_traced(
            &original,
            original.main.unwrap(),
            vec![],
            OverflowChecks::Checked,
            100000,
        ),
    ] {
        assert!(matches!(result,Ok(hir::Value::Integer(n)) if n.signed_value()==Some(7)));
        assert_eq!(events.iter().filter(|e|matches!(&e.value,hir::Value::Aggregate(fields) if matches!(fields.as_slice(),[hir::Value::Integer(n)] if n.signed_value()==Some(7)))).count(),1);
    }
    for (opcode, replacement) in [
        ("raw_take", "storage_initialize"),
        ("raw_initialize", "storage_take"),
    ] {
        let mut bad = original.clone();
        let mut changed = false;
        for f in &mut bad.functions {
            for block in &mut f.blocks {
                if let mir::Terminator::Evaluate {
                    operation: mir::Operation::Unary(op, _),
                    ..
                } = &mut block.terminator
                {
                    if *op == opcode {
                        *op = replacement;
                        changed = true;
                    }
                }
            }
        }
        assert!(changed);
        assert!(mir::verify::verify(&bad).is_err());
        assert!(rust::emit_mir(&bad, OverflowChecks::Checked).is_err());
    }
}
