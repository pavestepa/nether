use nether_core::{check::check, hir::Value, interpret};
use nether_frontend::{parser::parse, source::SourceId};
use nether_semantics::OverflowChecks;
#[test]
fn static_methods_use_the_same_generic_call_contracts_as_free_functions() {
    let source="#[Copy] struct Box<T:Copy> {value:T;fn create(value:T):Box<T> {return Box<T> {value:value}};fn extract(value:Box<T>):T {return value.value}} fn main():i32 {let boxed=Box<i32>.create(17);return Box.extract(boxed)}";
    let parsed = parse(SourceId(0), source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let p = check(&parsed.module.unwrap()).unwrap();
    let Value::Integer(result) =
        interpret::execute(&p, p.main.unwrap(), vec![], OverflowChecks::Checked, 10000).unwrap()
    else {
        panic!()
    };
    assert_eq!(result.signed_value(), Some(17));
}

#[test]
fn instance_methods_borrow_receivers_including_copy_types() {
    for (source,expected) in [
        ("struct Cell {value:i32;fn bump(var this):() {value+=1};fn read(this):i32 {return value};fn twice(var this):() {bump();this.bump()}} fn main():i32 {var c=Cell {value:1};c.twice();return c.read()}",3),
        ("#[Copy] struct Cell {value:i32;fn bump(var this):() {this.value+=1};fn read(this):i32 {return this.value}} fn main():i32 {var c=Cell {value:1};c.bump();return c.read()+c.value}",4),
        ("struct Box<T> {value:T;fn read(this):i32 {return 7}} fn main():i32 {let b=Box<i32> {value:3};return b.read()}",7),
        ("#[Copy] struct Box<T:Copy> {value:T;fn get(this):T {return value}} fn main():i32 {return (Box<i32> {value:9}).get()}",9),
        ("struct Cell {value:i32;fn read(this):i32 {let value=4;return value+this.value}} fn main():i32 {let c=Cell {value:3};return c.read()}",7),
    ] {
        let parsed=nether_frontend::parser::parse(nether_frontend::source::SourceId(0),source);
        assert!(parsed.diagnostics.is_empty(),"{:?}",parsed.diagnostics);
        let p=nether_core::check::check(&parsed.module.unwrap()).unwrap_or_else(|e|panic!("{source}: {e:?}"));
        let cfg=nether_core::mir::lower(&p);
        let a=nether_core::interpret::execute(&p,p.main.unwrap(),vec![],nether_semantics::OverflowChecks::Checked,10000).unwrap();
        let b=nether_core::mir::interpret::execute(&cfg,cfg.main.unwrap(),vec![],nether_semantics::OverflowChecks::Checked,10000).unwrap();
        assert_eq!(a,b,"{source}");
        let nether_core::hir::Value::Integer(a)=a else {panic!()};assert_eq!(a.signed_value(),Some(expected),"{source}");
        nether_core::llvm::emit(&p,nether_semantics::OverflowChecks::Checked).unwrap();
    }
    for source in [
        "struct Cell {value:i32;fn bump(this):() {value+=1}} fn main():() {}",
        "struct Cell {value:i32;fn bump(var this):() {value+=1}} fn main():() {let c=Cell {value:1};c.bump()}",
        "struct Cell {value:i32;fn consume(this):Cell {return this}} fn main():() {}",
    ] {
        let parsed=nether_frontend::parser::parse(nether_frontend::source::SourceId(0),source);
        assert!(parsed.diagnostics.is_empty());
        assert!(nether_core::check::check(&parsed.module.unwrap()).is_err(),"accepted {source}");
    }
}
