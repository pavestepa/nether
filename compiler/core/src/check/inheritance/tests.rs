use super::*;
use nether_frontend::{parser::parse, source::SourceId};
fn module(source: &str) -> a::Module {
    let parsed = parse(SourceId(0), source);
    assert!(
        parsed.diagnostics.is_empty(),
        "{source}: {:?}",
        parsed.diagnostics
    );
    parsed.module.unwrap()
}
#[test]
fn valid_hierarchies_substitute_generics_and_keep_override_chains() {
    for source in [
        "const N:usize=2;class A<const M:usize=N> {virtual fn read(this,x:{i32;M}):i32 {return 1}} class B:A {override fn read(this,x:{i32;2}):i32 {return 2}}",
        "const N:usize=1;class A<const N:usize> {virtual fn read(this,x:{i32;N}):i32 {return 1}} class B<const N:usize>:A<N> {override fn read(this,x:{i32;N}):i32 {return 2}}",
        "class A<T> {virtual fn read<U:Copy>(this,x:T,y:U):T {return x}} class B<U>:A<U> {override fn read<V:Copy>(this,x:U,y:V):U {return x}}",
        "class A<const N:usize> {virtual fn read<const M:usize>(this,x:{i32;N},y:{i32;M}):i32 {return 1}} class B<const M:usize>:A<M> {override fn read<const K:usize>(this,x:{i32;M},y:{i32;K}):i32 {return 2}}",
        "class A {virtual fn read(this):i32 {return 1}} class B:A {override fn read(this):i32 {return 2}} class C:B {override fn read(this):i32 {return 3}}",
        "class C:B {} class B:A {} class A {}",
        "interface I {fn read(this):i32} class A {virtual fn read(this):i32 {return 1}} class B:A,I {override fn read(this):i32 {return 2}}",
        "class A<T> {virtual fn read(this,x:T):T {return x}} class B<U>:A<U> {override fn read(this,x:U):U {return x}} class C:B<i32> {override fn read(this,x:i32):i32 {return x}}",
        "class A<T,U=T> {virtual fn read(this,x:U):T {return x}} class B:A<i32> {override fn read(this,x:i32):i32 {return x}}",
        "class A<T,U> {virtual fn read(this,x:T,y:U):T {return x}} class B<T,U>:A<U,T> {override fn read(this,x:U,y:T):U {return x}}",
        "class A<const N:usize> {virtual fn read(this,x:{i32;N}):i32 {return 1}} class B<const M:usize>:A<M> {override fn read(this,x:{i32;M}):i32 {return 2}}",
        "const N:usize=2;class A<const M:usize> {virtual fn read(this,x:{i32;M}):i32 {return 1}} class B:A<N> {override fn read(this,x:{i32;2}):i32 {return 2}}",
        "class A<const N:usize=2> {virtual fn read(this,x:{i32;N}):i32 {return 1}} class B:A {override fn read(this,x:{i32;1+1}):i32 {return 2}}",
        "class A<T> {virtual fn read<U:Copy>(this,x:T,y:U):U {return y}} class B:A<i32> {override fn read<V:Copy>(this,x:i32,y:V):V {return y}}",
        "class A {virtual fn first<const N:usize>(this,x:{i32;N}):i32 {return 1}} class B:A {override fn first<const M:usize>(this,x:{i32;M}):i32 {return 2}}",
        "interface I {} interface J {} class A {virtual fn read<T:I & J>(this,x:T):i32 {return 1}} class B:A {override fn read<U:J & I>(this,x:U):i32 {return 2}}",
        "class A {private field:i32;private fn read(this):i32 {return 1}} class B:A {field:i32;fn read(this):i32 {return 2}}",
        "class A {virtual fn read(this):i32 {return 1}} class B:A {} class C:B {override fn read(this):i32 {return 2}}",
    ] {validate(&module(source)).unwrap_or_else(|e|panic!("{source}: {e:?}"));}
}
#[test]
fn invalid_hierarchies_fail_before_layout_or_instantiation() {
    for source in [
        "const N:usize=2;class A<const M:usize=N> {virtual fn read(this,x:{i32;M}):i32 {return 1}} class B<const N:usize>:A {override fn read(this,x:{i32;N}):i32 {return 2}}",
        "const N:usize=2;class A<const M:usize> {virtual fn read(this,x:{i32;M}):i32 {return 1}} class B:A<N> {} class C<const N:usize>:B {override fn read(this,x:{i32;N}):i32 {return 2}}",
        "const N:usize=1;class A<const N:usize> {virtual fn read(this,x:{i32;N}):i32 {return 1}} class B<const N:usize>:A<N> {override fn read(this,x:{i32;N*N}):i32 {return 2}}",
        "const N:usize=2;class A {virtual fn read(this,x:{i32;N}):i32 {return 1}} class B<const N:usize>:A {override fn read(this,x:{i32;N}):i32 {return 2}}",
        "class A:A {}",
        "class A<T>:B<T> {} class B<U>:A<U> {}",
        "class A:B {} class B:C {} class C:A {}",
        "class A {} class B {} class C:A,B {}",
        "interface I {} class A {} class B:I,A {}",
        "struct A {} class B:A {}",
        "class A {} struct B:A {}",
        "class B:Missing {}",
        "class A<T> {} class B:A {}",
        "class A {} class B:A<i32> {}",
        "class A<T> {} class B:A<2> {}",
        "class A<const N:usize> {} class B:A<{i32;2}> {}",
        "struct A {virtual fn read(this):i32 {return 1}}",
        "class A {virtual fn read():i32 {return 1}}",
        "class A {private virtual fn read(this):i32 {return 1}}",
        "class A {override fn read(this):i32 {return 1}}",
        "class A {fn read(this):i32 {return 1}} class B:A {override fn read(this):i32 {return 2}}",
        "class A {virtual fn read(this):i32 {return 1}} class B:A {fn read(this):i32 {return 2}}",
        "class A {virtual fn read(this):i32 {return 1}} class B:A {virtual fn read(this):i32 {return 2}}",
        "class A {private fn read(this):i32 {return 1}} class B:A {override fn read(this):i32 {return 2}}",
        "class A {virtual fn read(this):i32 {return 1}} class B:A {override fn read(var this):i32 {return 2}}",
        "class A {virtual fn read(this,var x:i32):i32 {return x}} class B:A {override fn read(this,x:i32):i32 {return x}}",
        "class A {virtual fn read(this):i32 {return 1}} class B:A {override fn read(this):bool {return true}}",
        "class A {virtual fn read(this):i32 {return 1}} class B:A {override fn read():i32 {return 2}}",
        "class A {virtual fn read<T:Copy>(this,x:T):T {return x}} class B:A {override fn read<U>(this,x:U):U {return x}}",
        "class A {virtual fn read<T=i32>(this,x:T):T {return x}} class B:A {override fn read<U=bool>(this,x:U):U {return x}}",
        "class A {virtual fn read<const N:usize>(this,x:{i32;N}):i32 {return 1}} class B:A {override fn read<const M:usize>(this,x:{i32;M*M}):i32 {return 2}}",
        "class A {value:i32} class B:A {value:i32}",
        "class A {value:i32} class B:A {fn value(this):i32 {return 1}}",
        "class A {fn value(this):i32 {return 1}} class B:A {value:i32}",
        "class A {fn read(this):i32 {return 1};fn read(this):i32 {return 2}}",
    ] {assert!(validate(&module(source)).is_err(),"accepted {source}");}
}
