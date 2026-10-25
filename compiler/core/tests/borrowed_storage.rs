use nether_core::{check::check, hir, interpret, mir};
use nether_frontend::{
    parser::parse,
    source::{Diagnostic, SourceId},
};
use nether_semantics::OverflowChecks;
fn checked(source: &str) -> Result<hir::Program, Vec<Diagnostic>> {
    let parsed = parse(SourceId(0), source);
    assert!(
        parsed.diagnostics.is_empty(),
        "{source}: {:?}",
        parsed.diagnostics
    );
    check(&parsed.module.unwrap())
}
const PREFIX: &str = "struct R {value:i32;destructor {}} struct Holder {value:R} ";
const CASES:&[(&str,i128)]=&[
 ("fn flag():bool {return false} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref w=b;let h=if flag() {Holder {value:a}} else {Holder {value:w}};return h.value.value}",3),
 ("fn main():i32 {let a=R {value:7};let b=R {value:3};let ref w=b;let h=match 1 {0=>Holder {value:a},_=>Holder {value:w}};return h.value.value}",3),
 ("fn wrap(_borrowed_x:R):Holder {let x=_borrowed_x;return Holder {value:x}} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref w=b;var h=wrap(a);h.value=w;return h.value.value}",3),

 ("fn set(var h:Holder,_borrowed_x:R):() {let x=_borrowed_x;h.value=x} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref w=b;var h=Holder {value:a};set(h,w);return h.value.value}",3),
 ("fn set(var h:Holder,_borrowed_x:R):() {let x=_borrowed_x;let ref var alias=h;alias.value=x} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref w=b;var h=Holder {value:a};set(h,w);return h.value.value}",3),
 ("fn main():i32 {let a=R {value:7};let b=R {value:3};let ref w=b;var h=Holder {value:a};let ref var alias=h;alias.value=w;return h.value.value}",3),

 ("class C {value:R;constructor(x:R) {value=x}} fn flag():bool {return true} fn main():i32 {let x=R {value:7};var c=new C(R {value:3});if flag() {c.value=x} else {return x.value};return c.value.value}",7),

 ("fn main():i32 {let a:u8=7;let xs={a,3};return xs[1] as i32}",3),

 ("struct Counter {value:i32;fn tap(var this,x:R):R {value+=1;return x}} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref w=b;var count=Counter {value:0};let xs={count.tap(a),w};return a.value+xs[0].value+xs[1].value+count.value}",18),

 ("fn main():i32 {let a=R {value:7};let xs={a,a};return xs[0].value+xs[1].value}",14),
 ("struct G<T> {value:T} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref w=b;let xs={G<R> {value:a},G<R> {value:w}};return xs[0].value.value+xs[1].value.value}",10),

 ("fn main():i32 {let a=R {value:7};let b=R {value:3};let ref w=b;var h=Holder {value:a};h.value=w;return h.value.value}",3),
 ("fn main():i32 {let a=R {value:7};let b=R {value:3};let ref w=b;var h=(a,);h.0=w;return h.0.value}",3),
 ("fn main():i32 {let a=R {value:7};let b=R {value:3};let ref w=b;var xs={a};xs[0]=w;return xs[0].value}",3),
 ("struct Outer {h:Holder} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref w=b;var o=Outer {h:Holder {value:a}};o.h.value=w;return o.h.value.value}",3),
 ("fn flag():bool {return false} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref w=b;var h=Holder {value:a};if flag() {h.value=w};return h.value.value}",7),
 ("struct G<T> {value:T} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref w=b;var h=G<R> {value:a};h.value=w;return h.value.value}",3),
 ("fn main():i32 {let a=R {value:7};let b=R {value:3};let ref w=b;var h=Holder {value:a};h=Holder {value:w};return h.value.value}",3),
 ("fn main():i32 {let a=R {value:7};let b=R {value:3};let ref w=b;var first=Holder {value:a};var second=first;second.value=w;return second.value.value}",3),
 ("fn id<T>(x:T):T {return x} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref w=b;var h=Holder {value:a};h.value=w;let result=id(h);return result.value.value}",3),
 ("fn main():i32 {let a=R {value:7};let b=R {value:3};let ref w=b;let xs={a,w};return xs[0].value+xs[1].value}",10),
 ("fn main():i32 {let a=R {value:7};let b=R {value:3};let ref w=b;let xs={w,a};return xs[0].value+xs[1].value}",10),
 ("fn main():i32 {let a=R {value:7};let b=R {value:3};let ref w=b;let xs={Holder {value:a},Holder {value:w}};return xs[0].value.value+xs[1].value.value}",10),
 ("fn main():i32 {let a=R {value:7};let b=R {value:3};let c=R {value:2};let ref w=c;var h=Holder {value:a};if true {var h=Holder {value:b};h.value=w};return h.value.value}",7),
 ("fn main():i32 {let a=R {value:7};let b=R {value:3};let ref w=b;var xs={a,a};xs[1]=w;return xs[0].value+xs[1].value}",10),

 ("enum E {Item(R),Empty} fn set(var e:E,_borrowed_x:R):() {let x=_borrowed_x;let ref var target=e;*target=E.Item(x)} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref v=a;let ref w=b;var e=E.Item(v);e=E.Empty;set(e,w);return match e {E.Item(x)=>x.value,E.Empty=>0}}",3),
 ("enum E {Item(R),Empty} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref v=a;let ref w=b;var e=E.Item(v);e=E.Empty;let ref var target=e;*target=E.Item(w);return match e {E.Item(x)=>x.value,E.Empty=>0}}",3),
 ("enum E {Item(R),Empty} fn main():i32 {let a=R {value:7};let ref v=a;var e=E.Item(v);e=E.Empty;return match e {E.Item(x) if 1/0==0=>x.value,E.Item(x)=>x.value,E.Empty if false=>1/0,E.Empty=>3}}",3),
 ("enum E {Item(R),Empty} fn clear<T>(x:T):i32 {return 3} fn main():i32 {let a=R {value:7};let ref v=a;var e=E.Item(v);e=E.Empty;let saved=match e {E.Item(x)=>x.value,E.Empty=>3};return saved+clear(0)}",6),

 ("enum E {Item(R),Empty} fn main():i32 {var a=R {value:7};let ref v=a;var e=E.Item(v);e=E.Empty;a=R {value:3};return match e {E.Item(x)=>x.value,E.Empty=>a.value}}",3),
 ("enum E {Item(R),Empty} fn main():i32 {let a=R {value:7};let ref v=a;var e=E.Item(v);e=E.Empty;return match e {E.Item(x)=>x.value,E.Empty=>3}}",3),
 ("enum E {Item(R),Empty} fn unknown():bool {return true} fn main():i32 {let a=R {value:7};let ref v=a;var e=E.Item(v);if unknown() {e=E.Empty} else {e=E.Empty};return match e {E.Item(x)=>x.value,E.Empty=>3}}",3),
 ("enum E {Item(R),Empty} fn main():i32 {let a=R {value:7};let ref v=a;var e=E.Item(v);var n=0;while n<2 {e=E.Empty;n+=1};e=E.Empty;return match e {E.Item(x)=>x.value,E.Empty=>3}}",3),
 ("enum E {Item(R),Empty} fn clear():i32 {let a=R {value:7};let ref v=a;var current=E.Item(v);current=E.Empty;return match current {E.Item(x)=>x.value,E.Empty=>3}} fn main():i32 {return clear()}",3),
 ("fn get<T>(x:T):T {if false {return x};return x} fn main():i32 {let a=R {value:7};let result=get(a);return a.value+result.value}",14),
 ("fn get(_borrowed_x:R):R {let x=_borrowed_x;if false {let local=R {value:3};return local};return x} fn main():i32 {let a=R {value:7};let result=get(a);return a.value+result.value}",14),
 ("fn main():i32 {let a=R {value:7};let ref v=a;var h=Holder {value:v};if false {let local=R {value:3};h.value=local};return h.value.value}",7),

 ("class C {r:R;constructor() {r=R {value:3}}} fn set(var h:Holder,x:C):() {h.value=x.r} fn main():i32 {let a=R {value:7};let b=new C();let ref v=a;let ref w=b;var h=Holder {value:v};set(h,w);return h.value.value}",3),

 ("fn set(var h:Holder,xs:{R;2},i:usize):() {h.value=xs[i]} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref v=a;let ref w=b;var h=Holder {value:v};set(h,{v,w},1);return h.value.value}",3),

 ("fn set(var h:Holder,_borrowed_x:R):Holder {let x=_borrowed_x;h.value=x;return Holder {value:h.value}} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref v=a;let ref w=b;var h=Holder {value:v};let result=set(h,w);return result.value.value+h.value.value}",6),
 ("fn set(var h:Holder,s:Holder):() {h.value=s.value} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref v=a;let ref w=b;var h=Holder {value:v};set(h,Holder {value:w});return h.value.value}",3),
 ("fn set(var h:Holder,_borrowed_x:R,n:i32):() {let x=_borrowed_x;if n==0 {h.value=x} else {relay(h,x,n-1)}} fn relay(var h:Holder,x:R,n:i32):() {set(h,x,n)} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref v=a;let ref w=b;var h=Holder {value:v};set(h,w,2);return h.value.value}",3),
 ("fn set(var h:Holder,_borrowed_x:R):() {let x=_borrowed_x;h.value=x} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref v=a;let ref w=b;var h=Holder {value:v};var n=0;while n<2 {set(h,w);n+=1};return h.value.value}",3),

 ("fn set(var h:Holder,_borrowed_x:R):() {let x=_borrowed_x;h.value=x} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref v=a;var h=Holder {value:v};set(h,b);return b.value+h.value.value}",6),
 ("struct G<T> {value:T} fn set<T>(var h:G<T>,_borrowed_x:T):() {let x=_borrowed_x;h.value=x} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref v=a;let ref w=b;var h=G<R> {value:v};set(h,w);return h.value.value}",3),
 ("struct S {value:R;fn set(var this,_borrowed_x:R):() {let x=_borrowed_x;value=x}} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref v=a;let ref w=b;var h=S {value:v};h.set(w);return h.value.value}",3),
 ("fn set(var h:Holder,_borrowed_x:R,flag:bool):() {let x=_borrowed_x;if flag {h.value=x} else {set(h,x,true)}} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref v=a;let ref w=b;var h=Holder {value:v};set(h,w,false);return h.value.value}",3),
 ("fn set(var h:Holder,_borrowed_x:R,flag:bool):() {let x=_borrowed_x;if flag {h.value=x}} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref v=a;let ref w=b;var h=Holder {value:v};set(h,w,false);return h.value.value}",7),
 ("fn set(var h:Holder,_borrowed_x:R):R {let x=_borrowed_x;h.value=x;return h.value} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref v=a;let ref w=b;var h=Holder {value:v};let result=set(h,w);return result.value+h.value.value}",6),
 ("struct Outer {h:Holder} fn set(var h:Holder,_borrowed_x:R):() {let x=_borrowed_x;h.value=x} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref v=a;let ref w=b;var o=Outer {h:Holder {value:v}};set(o.h,w);return o.h.value.value}",3),
 ("fn set(var h:Holder,_borrowed_x:R):() {let x=_borrowed_x;let ref var target=h;*target=Holder {value:x}} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref v=a;let ref w=b;var h=Holder {value:v};set(h,w);return h.value.value}",3),

 ("fn set(var h:Holder,_borrowed_x:R):() {let x=_borrowed_x;h.value=x} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref v=a;let ref w=b;var h=Holder {value:v};set(h,w);return h.value.value}",3),
 ("fn set(var h:Holder,_borrowed_x:R):() {let x=_borrowed_x;h.value=x} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref v=a;let ref w=b;var h=Holder {value:v};let old=h.value;set(h,w);return old.value+h.value.value}",10),
 ("fn set(var h:Holder,_borrowed_x:R):() {let x=_borrowed_x;h.value=x} fn relay(var h:Holder,x:R):() {set(h,x)} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref v=a;let ref w=b;var h=Holder {value:v};relay(h,w);return h.value.value}",3),
 ("struct A {xs:{R;2}} fn set(var a:A,_borrowed_x:R,i:usize):() {let x=_borrowed_x;a.xs[i]=x} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref v=a;let ref w=b;var holder=A {xs:{v,v}};set(holder,w,1);return holder.xs[0].value+holder.xs[1].value}",10),

 ("fn id<T>(x:T):T {return x} fn main():i32 {let a=R {value:7};let ref v=a;var h=(v,);let g=id(h);return g.0.value}",7),
 ("fn id(h:Holder):Holder {return h} fn main():i32 {let a=R {value:7};let ref v=a;let h=Holder {value:v};let outer=id(h);return h.value.value+outer.value.value}",14),
 ("fn pair(_borrowed_x:R,_borrowed_y:R):(R,R) {let x=_borrowed_x;let y=_borrowed_y;return (x,y)} fn main():i32 {let a=R {value:7};let ref va=a;let b=R {value:3};let ref vb=b;let h=pair(va,vb);return a.value+b.value+h.0.value+h.1.value}",20),
 ("class C {value:i32;constructor(n:i32) {value=n};destructor {}} fn wrap(_borrowed_x:C):(C,) {let x=_borrowed_x;return (x,)} fn main():i32 {let a=new C(7);let ref va=a;let h=wrap(va);return if same_identity(va,h.0) {h.0.value} else {0}}",7),
 ("fn set(var xs:{R;2},_borrowed_x:R,i:usize):{R;2} {let x=_borrowed_x;xs[i]=x;return xs} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref x=a;let ref y=b;var arr={x,x};let xs=set(arr,y,1);return xs[0].value+xs[1].value}",10),
 ("fn wrap(_borrowed_x:R):Holder {let x=_borrowed_x;return Holder {value:x}} fn main():i32 {let a=R {value:7};let ref va=a;let h=wrap(va);return a.value+h.value.value}",14),
 ("fn wrap(_borrowed_x:R):Holder {let x=_borrowed_x;return Holder {value:x}} fn relay(x:R):Holder {return wrap(x)} fn main():i32 {let a=R {value:7};let ref va=a;let h=relay(va);return a.value+h.value.value}",14),
 ("fn id<T>(x:T):T {return x} fn main():i32 {let a=R {value:7};let ref v=a;let t=(v,);let h=id(t);return h.0.value}",7),
 ("fn wrap<T>(_borrowed_x:T):(T,) {let x=_borrowed_x;return (x,)} fn main():i32 {let a=R {value:7};let ref va=a;let h=wrap(va);return a.value+h.0.value}",14),
 ("fn wrap(_borrowed_x:R):{R;1} {let x=_borrowed_x;return {x}} fn main():i32 {let a=R {value:7};let ref va=a;let h=wrap(va);return a.value+h[0].value}",14),
 ("enum E {Item(R),Empty} fn wrap(_borrowed_x:R):E {let x=_borrowed_x;return E.Item(x)} fn main():i32 {let a=R {value:7};let ref va=a;let e=wrap(va);return a.value+match e {E.Item(x)=>x.value,E.Empty=>0}}",14),
 ("fn wrap(flag:bool,_borrowed_x:R):Holder {let x=_borrowed_x;if flag {return Holder {value:x}};return wrap(true,x)} fn main():i32 {let a=R {value:7};let ref va=a;let h=wrap(false,va);return a.value+h.value.value}",14),
 ("struct Mixed {view:R;owned:R} fn wrap(_borrowed_x:R):Mixed {let x=_borrowed_x;return Mixed {view:x,owned:R {value:3}}} fn main():i32 {let a=R {value:7};let ref va=a;let h=wrap(va);return a.value+h.view.value+h.owned.value}",17),
 ("struct G<T> {value:T} fn get<T>(h:G<T>):T {return h.value} fn main():i32 {let a=R {value:7};let ref v=a;let h=G<R> {value:v};let result=get(h);return result.value}",7),
 ("fn consume<T>(h:T):i32 {return 3} fn main():i32 {let a=R {value:7};let ref v=a;let h=(v,);return consume(h)+h.0.value}",10),
 ("struct G<T> {value:T;fn get(this):T {return value}} fn main():i32 {let a=R {value:7};let ref v=a;let h=G<R> {value:v};let result=h.get();return result.value}",7),
 ("fn get(h:Holder):R {return h.value} fn relay(h:Holder):R {return get(h)} fn main():i32 {let a=R {value:7};let ref v=a;let h=Holder {value:v};let result=relay(h);return result.value}",7),
 ("struct Outer {h:Holder} fn get(o:Outer):R {return o.h.value} fn main():i32 {let a=R {value:7};let ref v=a;let h=Holder {value:v};let ref inner=h;let o=Outer {h:inner};let result=get(o);return result.value}",7),
 ("fn read(h:Holder):i32 {return h.value.value} fn main():i32 {let a=R {value:7};let ref v=a;let h=Holder {value:v};return read(h)}",7),
 ("fn read(h:Holder):i32 {return h.value.value} fn main():i32 {let a=R {value:7};let ref v=a;let h=Holder {value:v};return read(h)+h.value.value}",14),
 ("fn get(h:Holder):R {return h.value} fn main():i32 {let a=R {value:7};let ref v=a;let h=Holder {value:v};let result=get(h);return result.value}",7),
 ("fn get(h:Holder):R {return h.value} fn main():i32 {let a=R {value:7};let ref v=a;let h=Holder {value:v};let result=get(h);return h.value.value+result.value}",14),
 ("struct S {value:R;fn read(this):i32 {return value.value};fn get(this):R {return value}} fn main():i32 {let a=R {value:7};let ref v=a;let h=S {value:v};let result=h.get();return h.read()+result.value}",14),
 ("fn get(xs:{R;2},i:usize):R {return xs[i]} fn main():i32 {let a=R {value:7};let ref v=a;let arr={v,v};let result=get(arr,1);return result.value}",7),
 ("enum E {Item(R),Empty} fn read(e:E):i32 {return match e {E.Item(x)=>x.value,E.Empty=>0}} fn main():i32 {let a=R {value:7};let ref v=a;return read(E.Item(v))}",7),
 ("enum E {Item(R),Empty} fn main():i32 {let a=R {value:7};let e=E.Item(a);return a.value+match e {E.Item(x)=>x.value,E.Empty=>0}}",14),
 ("enum E {Item {view:R,owned:R},Empty} fn main():i32 {let a=R {value:7};let ref v=a;let e=E.Item {owned:R {value:3},view:v};return match e {E.Item {view:x,owned:y}=>x.value+y.value,E.Empty=>0}}",10),
 ("enum E {Item(Holder)} fn main():i32 {let a=R {value:7};let ref v=a;let e=E.Item(Holder {value:v});let moved=e;return match moved {E.Item(h)=>h.value.value}}",7),
 ("enum E {Item(R),Empty} fn main():i32 {let a=R {value:7};let ref v=a;let e=if true {E.Item(v)} else {E.Empty};return match e {E.Item(x)=>x.value,E.Empty=>3}}",7),
 ("fn get(_borrowed_x:R):R {let x=_borrowed_x;let h=(x,);return h.0} fn main():i32 {let a=R {value:7};let ref va=a;let result=get(va);return a.value+result.value}",14),
 ("fn main():i32 {let a=R {value:7};let b=R {value:3};let ref v=a;var h=Holder {value:v};let old=h.value;let ref var container=h;container.value=b;return old.value+container.value.value}",10),
 ("fn main():i32 {let a=R {value:7};let b=R {value:3};let ref x=a;let ref y=b;let h=if false {Holder {value:x}} else {Holder {value:y}};return h.value.value}",3),
 ("struct Plain {value:i32} fn main():i32 {let a=Plain {value:7};let xs=(a,);return a.value+xs.0.value}",14),
 ("fn id(x:R):R {return x} fn main():i32 {let a=R {value:7};let xs={id(a)};return a.value+xs[0].value}",14),
 ("fn id(x:R):R {return x} fn main():i32 {let a=R {value:7};let ref v=a;let h=Holder {value:v};let result=id(h.value);return result.value}",7),
 ("fn main():i32 {let a=R {value:7};let ref v=a;let h=Holder {value:v};let ref outer=h;let xs=(outer,);return xs.0.value.value}",7),

 ("fn main():i32 {let a=R {value:7};let h=Holder {value:a};return a.value+h.value.value}",14),
 ("fn main():i32 {let a=R {value:7};let ref v=a;let h=Holder {value:v};let moved=h;return moved.value.value}",7),
 ("fn main():i32 {let a=R {value:7};let xs={a,a};return a.value+xs[0].value+xs[1].value}",21),
 ("fn main():i32 {let a=R {value:7};let xs=(a,R {value:3});let ys=xs;return a.value+ys.0.value+ys.1.value}",17),
 ("fn main():i32 {let a=R {value:7};let ref v=a;let x=(v,);let y=x;return x.0.value+y.0.value}",14),
 ("fn main():i32 {let a=R {value:7};let ref v=a;let xs={v,v};let i:usize=1;let y=xs[i];return y.value}",7),
 ("fn main():i32 {let a=R {value:7};let b=R {value:3};let ref v=a;var h=Holder {value:v};h.value=b;return h.value.value+a.value}",10),
 ("fn main():i32 {let a=R {value:7};let ref v=a;let (x,)= (v,);return x.value}",7),
 ("fn main():i32 {let a=R {value:7};let ref v=a;let xs=({v,v},);return xs.0[1].value}",7),
 ("fn id(x:R):R {return x} fn main():i32 {let a=R {value:7};let h=Holder {value:id(a)};return a.value+h.value.value}",14),
 ("class C {value:i32;constructor(n:i32) {value=n};destructor {}} struct Box {value:C} fn main():i32 {let a=new C(7);let h=Box {value:a};return if same_identity(a,h.value) {h.value.value} else {0}}",7),
];
const PANICS:&[(&str,i128)]=&[
 ("fn fail(x:R):R {let fail=1/0;return x} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref w=b;let xs={a,w,fail(a)};return a.value+xs[0].value}",101),

 ("fn set(var h:Holder,_borrowed_x:R):() {let x=_borrowed_x;h.value=x;let fail=1/0} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref v=a;let ref w=b;var h=Holder {value:v};set(h,w);return h.value.value}",101),

 ("struct F {value:i32;destructor {let fail=1/value}} struct Mixed {view:R;owned:R} fn wrap(_borrowed_x:R):Mixed {let x=_borrowed_x;let fail=F {value:0};return Mixed {view:x,owned:R {value:3}}} fn main():i32 {let a=R {value:7};let ref va=a;let h=wrap(va);return a.value+h.view.value}",101),
 ("enum E {Item {view:R,owned:R,fail:i32}} fn main():i32 {let a=R {value:7};let ref v=a;let e=E.Item {view:v,owned:R {value:3},fail:1/0};return 0}",101),
 ("fn main():i32 {let a=R {value:7};let ref v=a;let h=(v,R {value:3},1/0);return 0}",101),
 ("struct Mix {owned:R;borrowed:R;fail:i32} fn main():i32 {let a=R {value:7};let ref v=a;let h=Mix {borrowed:v,owned:R {value:3},fail:1/0};return 0}",101),
 ("fn get(_borrowed_x:R):R {let x=_borrowed_x;let h=(x,);let fail=1/0;return h.0} fn main():i32 {let a=R {value:7};let ref va=a;let h=(get(va),);return a.value+h.0.value}",101),
];
#[test]
fn partial_borrowed_construction_cleans_only_completed_owners() {
    for (body, _) in PANICS {
        let p = checked(&format!("{PREFIX}{body}")).unwrap();
        let m = mir::lower(&p);
        let a = interpret::execute_traced(
            &p,
            p.main.unwrap(),
            vec![],
            OverflowChecks::Checked,
            100_000,
        );
        let b = mir::interpret::execute_traced(
            &m,
            m.main.unwrap(),
            vec![],
            OverflowChecks::Checked,
            100_000,
        );
        assert_eq!(a, b, "{body}");
        assert!(a.0.is_err());
    }
}
#[test]
fn readonly_storage_moves_descriptors_and_preserves_cleanup() {
    for (body, expected) in CASES {
        let source = format!("{PREFIX}{body}");
        let p = checked(&source).unwrap_or_else(|e| panic!("{source}: {e:?}"));
        let m = mir::lower(&p);
        let a = interpret::execute_traced(
            &p,
            p.main.unwrap(),
            vec![],
            OverflowChecks::Checked,
            100_000,
        );
        let b = mir::interpret::execute_traced(
            &m,
            m.main.unwrap(),
            vec![],
            OverflowChecks::Checked,
            100_000,
        );
        assert_eq!(a, b, "{source}");
        assert!(
            matches!(a.0,Ok(hir::Value::Integer(n)) if n.signed_value()==Some(*expected)),
            "{source}: {a:?}"
        );
        nether_core::rust::emit(&p, OverflowChecks::Checked).unwrap();
    }
}
#[test]
fn storage_cannot_escape_gain_mutability_or_mix_modes() {
    for body in [
 "fn main():i32 {let b=R {value:3};let ref w=b;var h=Holder {value:R {value:7}};h.value=w;return h.value.value}",
 "fn main():i32 {let a=R {value:7};var b=R {value:3};let ref w=b;var h=Holder {value:a};h.value=w;b=R {value:2};return h.value.value}",
 "fn main():i32 {let a=R {value:7};var h=Holder {value:a};if true {let b=R {value:3};let ref w=b;h.value=w};return h.value.value}",
 "fn main():i32 {let a=R {value:7};let b=R {value:3};let ref w=b;var h=Holder {value:a};h.value.value=4;h.value=w;return h.value.value}",
 "fn main():i32 {let a=R {value:7};let b=R {value:3};let ref w=b;let xs={R {value:2},w};return xs[0].value+xs[1].value}",
 "fn main():i32 {var a=R {value:7};let b=R {value:3};let ref w=b;let xs={a,w};a=R {value:2};return xs[0].value}",

 "enum E {Item(R),Empty} fn set(var e:E,_borrowed_x:R):() {let x=_borrowed_x;let ref var target=e;*target=E.Item(x)} fn main():i32 {let a=R {value:7};var b=R {value:3};let ref v=a;let ref w=b;var e=E.Item(v);e=E.Empty;set(e,w);b=R {value:2};return match e {E.Item(x)=>x.value,E.Empty=>0}}",
 "enum E {Item(R),Empty} fn main():i32 {let a=R {value:7};var b=R {value:3};let ref v=a;let ref w=b;var e=E.Item(v);e=E.Empty;let ref var target=e;*target=E.Item(w);b=R {value:2};return match e {E.Item(x)=>x.value,E.Empty=>0}}",
 "enum E {Item(R),Empty} fn set(var e:E,_borrowed_x:R):() {let x=_borrowed_x;let ref var target=e;*target=E.Item(x)} fn main():i32 {let a=R {value:7};let ref v=a;var e=E.Item(v);e=E.Empty;if true {let b=R {value:3};let ref w=b;set(e,w)};return match e {E.Item(x)=>x.value,E.Empty=>0}}",

 "enum E {Item(R),Empty} fn unknown():bool {return true} fn main():i32 {var a=R {value:7};let ref v=a;var e=E.Item(v);if unknown() {e=E.Empty};a=R {value:3};return match e {E.Item(x)=>x.value,E.Empty=>3}}",
 "enum E {Item(R),Empty} fn main():i32 {var a=R {value:7};let ref v=a;var e=E.Item(v);var n=0;while n<2 {if n==1 {e=E.Empty};n+=1};a=R {value:3};return match e {E.Item(x)=>x.value,E.Empty=>3}}",
 "enum E {Item(R),Empty} fn main():i32 {var a=R {value:7};let ref v=a;var e=E.Empty;e=E.Item(v);a=R {value:3};return match e {E.Item(x)=>x.value,E.Empty=>3}}",
 "fn get(x:R,flag:bool):R {if flag {let local=R {value:3};return local};return x} fn main():i32 {let a=R {value:7};let result=get(a,true);return a.value+result.value}",

 "class C {r:R;constructor() {r=R {value:3}}} fn set(var h:Holder,x:C):() {h.value=x.r} fn main():i32 {let a=R {value:7};var b=new C();let ref v=a;let ref w=b;var h=Holder {value:v};set(h,w);b=new C();return h.value.value}",

 "fn set(var h:Holder,xs:{R;2},i:usize):() {h.value=xs[i]} fn main():i32 {let a=R {value:7};var b=R {value:3};let ref v=a;let ref w=b;var h=Holder {value:v};set(h,{v,w},1);b=R {value:2};return h.value.value}",

 "fn set(var h:Holder,_borrowed_x:R):() {let x=_borrowed_x;h.value=x} fn main():i32 {let a=R {value:7};var b=R {value:3};let ref v=a;let ref w=b;var h=Holder {value:v};set(h,w);b=R {value:2};return h.value.value}",
 "fn set(var h:Holder,_borrowed_x:R):() {let x=_borrowed_x;h.value=x} fn main():i32 {let a=R {value:7};let ref v=a;var h=Holder {value:v};if true {let b=R {value:3};let ref w=b;set(h,w)};return h.value.value}",
 "fn set(var h:Holder):() {let b=R {value:3};h.value=b} fn main():i32 {let a=R {value:7};let ref v=a;var h=Holder {value:v};set(h);return h.value.value}",
 "fn set(var h:Holder,_borrowed_x:R):() {let x=_borrowed_x;h.value=x} fn main():i32 {let a=R {value:7};let ref v=a;var h=Holder {value:v};set(h,R {value:3});return h.value.value}",
 "fn set(var h:Holder,_borrowed_x:R):() {let x=_borrowed_x;h.value=x} fn relay(var h:Holder,x:R):() {set(h,x)} fn main():i32 {let a=R {value:7};var b=R {value:3};let ref v=a;let ref w=b;var h=Holder {value:v};relay(h,w);b=R {value:2};return h.value.value}",
 "fn set(var h:Holder,_borrowed_x:R):() {let x=_borrowed_x;h.value=x} fn main():i32 {var a=R {value:7};let b=R {value:3};let ref v=a;let ref w=b;var h=Holder {value:v};let old=h.value;set(h,w);a=R {value:2};return old.value+h.value.value}",
 "struct A {xs:{R;2}} fn set(var a:A,_borrowed_x:R,i:usize):() {let x=_borrowed_x;a.xs[i]=x} fn main():i32 {let a=R {value:7};var b=R {value:3};let ref v=a;let ref w=b;var holder=A {xs:{v,v}};set(holder,w,1);b=R {value:2};return holder.xs[1].value}",
 "fn set(var h:Holder,_borrowed_x:R,flag:bool):() {let x=_borrowed_x;if flag {h.value=x}} fn main():i32 {let a=R {value:7};var b=R {value:3};let ref v=a;let ref w=b;var h=Holder {value:v};set(h,w,true);b=R {value:2};return h.value.value}",
 "fn set(var h:Holder,_borrowed_x:R,flag:bool):() {let x=_borrowed_x;if flag {h.value=x} else {set(h,x,true)}} fn main():i32 {let a=R {value:7};var b=R {value:3};let ref v=a;let ref w=b;var h=Holder {value:v};set(h,w,false);b=R {value:2};return h.value.value}",
 "fn set(var h:Holder):() {let b=R {value:3};h.value=b;let fail=1/0} fn main():i32 {let a=R {value:7};let ref v=a;var h=Holder {value:v};set(h);return h.value.value}",

 "fn id(h:Holder):Holder {return h} fn main():i32 {var a=R {value:7};let ref v=a;let h=Holder {value:v};let outer=id(h);let saved=h.value.value;a=R {value:2};return outer.value.value}",
 "fn set(var xs:{R;2},_borrowed_x:R,i:usize):{R;2} {let x=_borrowed_x;xs[i]=x;return xs} fn main():i32 {let a=R {value:7};var b=R {value:3};let ref x=a;let ref y=b;let xs=set({x,x},y,1);b=R {value:2};return xs[1].value}",
 "fn pair(_borrowed_x:R,_borrowed_y:R):(R,R) {let x=_borrowed_x;let y=_borrowed_y;return (x,y)} fn main():i32 {let a=R {value:1};var b=R {value:2};let ref x=a;let ref y=b;let h=pair(x,y);b=R {value:3};return h.1.value}",
 "fn wrap(_borrowed_x:R):Holder {let x=_borrowed_x;return Holder {value:x}} fn main():i32 {var a=R {value:1};let ref v=a;let h=wrap(v);a=R {value:2};return h.value.value}",
 "fn wrap<T>(_borrowed_x:T):(T,) {let x=_borrowed_x;return (x,)} fn bad():(R,) {let a=R {value:1};let ref v=a;return wrap(v)} fn main():i32 {return 0}",
 "fn wrap(flag:bool,_borrowed_x:R):Holder {let x=_borrowed_x;return if flag {Holder {value:x}} else {Holder {value:R {value:2}}}} fn main():i32 {let a=R {value:1};let h=wrap(true,a);return a.value+h.value.value}",
 "fn wrap(_borrowed_x:R):Holder {let x=_borrowed_x;return Holder {value:x}} fn main():i32 {let a=R {value:1};let ref v=a;var h=wrap(v);h.value.value=2;return h.value.value}",
 "struct G<T> {value:T} fn get<T>(h:G<T>):T {return h.value} fn main():i32 {var a=R {value:1};let ref v=a;let h=G<R> {value:v};let result=get(h);a=R {value:2};return result.value}",
 "struct Outer {h:Holder} fn get(o:Outer):R {return o.h.value} fn main():i32 {var a=R {value:1};let ref v=a;let h=Holder {value:v};let ref inner=h;let o=Outer {h:inner};let result=get(o);a=R {value:2};return result.value}",
 "fn get(h:Holder):R {return h.value} fn main():i32 {var a=R {value:1};let ref v=a;let h=Holder {value:v};let result=get(h);a=R {value:2};return result.value}",
 "fn bad():R {let a=R {value:1};let ref v=a;let h=Holder {value:v};return get(h)} fn get(h:Holder):R {return h.value} fn main():i32 {return bad().value}",
 "fn change(var h:Holder):() {h.value=R {value:2}} fn main():i32 {let a=R {value:1};let ref v=a;var h=Holder {value:v};change(h);return h.value.value}",
 "enum E {Item(R)} fn main():i32 {var a=R {value:1};let ref v=a;let e=E.Item(v);a=R {value:2};return match e {E.Item(x)=>x.value}}",
 "enum E {Item {view:R}} fn bad():E {let a=R {value:1};let ref v=a;return E.Item {view:v}} fn main():i32 {return 0}",

 "fn main():i32 {var a=R {value:1};let ref v=a;let h=Holder {value:v};let ref outer=h;a=R {value:2};return outer.value.value}",
 "fn main():i32 {var a=R {value:1};let ref v=a;let h=Holder {value:v};let ref outer=h;let xs=(outer,);a=R {value:2};return xs.0.value.value}",
 "fn main():i32 {let a=R {value:1};let ref v=a;var h=Holder {value:v};if true {let b=R {value:2};h.value=b};return h.value.value}",

 "fn main():i32 {var a=R {value:1};let ref v=a;let h=Holder {value:v};a=R {value:2};return h.value.value}",
 "fn main():i32 {var a=R {value:1};let ref v=a;var h=Holder {value:v};h.value.value=2;return h.value.value}",
 "fn main():i32 {let a=R {value:1};let ref v=a;let xs={v,R {value:2}};return xs[0].value}",
 "fn main():i32 {let a=R {value:1};let ref v=a;var h=Holder {value:v};h=Holder {value:R {value:2}};return h.value.value}",
 "fn main():i32 {let a=R {value:1};let ref v=a;let h=if true {Holder {value:v}} else {Holder {value:R {value:2}}};return h.value.value}",
 "fn bad():Holder {let a=R {value:1};let ref v=a;return Holder {value:v}} fn main():i32 {return bad().value.value}",

 ] {let source=format!("{PREFIX}{body}");assert!(checked(&source).is_err(),"accepted {source}");}
}
#[test]
fn native_storage_drops_owned_fields_once_and_never_drops_views() {
    use std::{collections::BTreeSet, fs, process::Command};
    let cases = CASES.iter().chain(PANICS.iter()).collect::<Vec<_>>();
    let mut rust = String::new();
    let mut traces = Vec::new();
    for (i, (body, _)) in cases.iter().copied().enumerate() {
        let p = checked(&format!("{PREFIX}{body}")).unwrap();
        let (_, events) = interpret::execute_traced(
            &p,
            p.main.unwrap(),
            vec![],
            OverflowChecks::Checked,
            100_000,
        );
        let trace = events
            .iter()
            .map(|e| {
                let hir::Value::Aggregate(fields) = &e.value else {
                    panic!()
                };
                let hir::Value::Integer(n) = fields[0] else {
                    panic!()
                };
                format!("DROP:{}:{}", e.entry, n.signed_value().unwrap())
            })
            .collect::<Vec<_>>();
        traces.push(trace);
        let emitted = nether_core::rust::emit(&p, OverflowChecks::Checked)
            .unwrap()
            .replace("fn main()", "pub fn run()");
        let emitted = emitted
            .lines()
            .map(|line| {
                if let Some(id) = line.strip_prefix("// destructor-entry ") {
                    format!("eprintln!(\"DROP:{id}:{{}}\",(*p).f0);")
                } else if let Some(id) = line.strip_prefix("// class-destructor-entry ") {
                    format!("eprintln!(\"DROP:{id}:{{}}\",(*(*heap).data.as_ptr()).f0);")
                } else if line == "// class-allocation" {
                    "eprintln!(\"ALLOC:{:p}\",heap);".into()
                } else if line == "// class-deallocation" {
                    "eprintln!(\"FREE:{:p}\",heap);".into()
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        rust.push_str(&format!("mod case{i} {{\n{emitted}\n}}\n"));
    }
    rust.push_str(
        "fn main() {match std::env::args().nth(1).unwrap().parse::<usize>().unwrap() {\n",
    );
    for i in 0..cases.len() {
        rust.push_str(&format!("{i}=>case{i}::run(),\n"));
    }
    rust.push_str("_=>panic!(),}}\n");
    let dir = std::env::temp_dir().join(format!("nether-storage-native-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let source = dir.join("cases.rs");
    fs::write(&source, rust).unwrap();
    for level in ["0", "3"] {
        let binary = dir.join(format!("cases-{level}"));
        let result = Command::new(std::env::var("NETHER_RUSTC").unwrap_or_else(|_| "rustc".into()))
            .arg("--edition=2021")
            .args(["-C", &format!("opt-level={level}")])
            .arg(&source)
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        for (i, (_, expected)) in cases.iter().copied().enumerate() {
            let result = Command::new(&binary).arg(i.to_string()).output().unwrap();
            let stderr = String::from_utf8_lossy(&result.stderr);
            assert_eq!(
                result.status.code(),
                Some(*expected as i32),
                "case {i} O{level}: {stderr}"
            );
            let trace = stderr
                .lines()
                .filter(|s| s.starts_with("DROP:"))
                .map(str::to_owned)
                .collect::<Vec<_>>();
            assert_eq!(trace, traces[i], "case {i} O{level}");
            let mut live = BTreeSet::new();
            let mut allocations = 0;
            for line in stderr.lines() {
                if let Some(p) = line.strip_prefix("ALLOC:") {
                    assert!(live.insert(p));
                    allocations += 1;
                } else if let Some(p) = line.strip_prefix("FREE:") {
                    assert!(live.remove(p));
                }
            }
            assert!(live.is_empty());
            assert_eq!(allocations, usize::from(cases[i].0.contains("class C")));
        }
    }
    fs::remove_dir_all(dir).unwrap();
}
