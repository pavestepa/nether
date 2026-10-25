use nether_core::{check::check, hir::Value, interpret, mir};
use nether_frontend::{parser::parse, source::SourceId};
use nether_semantics::OverflowChecks;
fn checked(
    source: &str,
) -> Result<nether_core::hir::Program, Vec<nether_frontend::source::Diagnostic>> {
    let p = parse(SourceId(0), source);
    assert!(p.diagnostics.is_empty(), "{:?}", p.diagnostics);
    check(&p.module.unwrap())
}
#[test]
fn borrowed_entries_mutate_the_caller_and_recursive_entries_are_reused() {
    for (source,expected) in [
        ("struct Cell {value:i32} fn bump(var x:Cell):() {x.value+=1} fn read(x:Cell):i32 {return x.value} fn main():i32 {var x=Cell {value:1};bump(x);return read(x)+x.value}",4),
        ("struct Cell {value:i32} fn bump(var x:Cell,n:i32):() {if n==0 {return};x.value+=1;bump(x,n-1)} fn main():i32 {var x=Cell {value:1};bump(x,5);return x.value}",6),
        ("struct Cell {value:i32} fn read(x:Cell):i32 {return x.value} fn main():i32 {let x=Cell {value:3};return read(x)+read(x)+x.value}",9),
        ("struct Cell {value:i32} fn select<T>(x:T,n:i32):i32 {return n} fn main():i32 {let x=Cell {value:1};return select(x,7)+x.value}",8),
        ("struct Cell {value:i32} fn change(var x:Cell,var y:Cell):() {x=y;x.value=9} fn main():i32 {var a=Cell {value:1};var b=Cell {value:2};change(a,b);return a.value*10+b.value}",19),
        ("struct Cell {value:i32} fn bump(var x:Cell):() {x.value+=1} fn main():i32 {var pair=(Cell {value:1},Cell {value:2});bump(pair.0);return pair.0.value+pair.1.value}",4),
    ] {
        let hir=checked(source).unwrap_or_else(|e|panic!("{source}: {e:?}"));
        assert!(hir.functions.iter().any(|f| f.parameters.iter().any(|p|f.locals[*p].view.is_some())));
        let cfg=mir::lower(&hir);
        let a=interpret::execute(&hir,hir.main.unwrap(),vec![],OverflowChecks::Checked,10000).unwrap();
        let b=mir::interpret::execute(&cfg,cfg.main.unwrap(),vec![],OverflowChecks::Checked,10000).unwrap();
        assert_eq!(a,b,"{source}");let Value::Integer(a)=a else {panic!()};assert_eq!(a.signed_value(),Some(expected),"{source}");
        nether_core::rust::emit(&hir,OverflowChecks::Checked).unwrap();
    }
}
#[test]
fn borrowed_entries_preserve_uniqueness_and_readonly_access() {
    for source in [
        "struct Cell {value:i32} fn bump(var x:Cell):() {x.value+=1} fn main():i32 {let x=Cell {value:1};bump(x);return x.value}",
        "struct Cell {value:i32} fn read_write(var x:Cell,y:Cell):i32 {x.value+=1;return y.value} fn main():i32 {var x=Cell {value:1};return read_write(x,x)}",
        "struct Cell {value:i32} fn consume(x:Cell):Cell {var y=x;y.value+=1;return y} fn main():i32 {let x=Cell {value:1};let y=consume(x);return x.value+y.value}",
    ] {assert!(checked(source).is_err(),"accepted {source}");}
}

#[test]
fn consuming_calls_can_be_followed_by_definite_reinitialization() {
    let prefix = "struct Cell {value:i32} struct Hold {cell:Cell} fn take(x:Cell):i32 {let held=Hold {cell:x};return held.cell.value+1} ";
    for (body, expected) in [
        ("fn main():i32 {var x=Cell {value:1};let n=take(x);x=Cell {value:4};return n+x.value}",6),
        ("fn main():i32 {var x=Cell {value:1};let n=take(x);if true {x=Cell {value:4}} else {x=Cell {value:5}};return n+x.value}",6),
        ("fn main():i32 {var x=Cell {value:1};var n=0;while n<6 {n+=take(x);x=Cell {value:1}};return n+x.value}",7),
        ("fn main():i32 {var x=Cell {value:1};var n=0;while n<6 {n+=take(x);x=Cell {value:1}};return n+x.value}",7),
    ] {
        let source=format!("{prefix}{body}");
        let p=checked(&source).unwrap_or_else(|errors| panic!("{source}: {errors:?}"));
        let cfg=mir::lower(&p);
        let a=interpret::execute(&p,p.main.unwrap(),vec![],OverflowChecks::Checked,10000).unwrap();
        let b=mir::interpret::execute(&cfg,cfg.main.unwrap(),vec![],OverflowChecks::Checked,10000).unwrap();
        assert_eq!(a,b);
        let Value::Integer(a)=a else {panic!()};
        assert_eq!(a.signed_value(),Some(expected));
    }
    for body in [
        "fn main():i32 {var x=Cell {value:1};var n=0;while n<6 {n+=take(x)};return n}",
        "fn main():i32 {var x=Cell {value:1};let n=take(x);if n>100 {x=Cell {value:4}};return n+x.value}",
        "fn main():i32 {var x=Cell {value:1};return take(x)+x.value}",
    ] {
        let source=format!("{prefix}{body}");
        assert!(checked(&source).is_err(),"accepted {source}");
    }
}
