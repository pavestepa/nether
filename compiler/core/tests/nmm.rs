//! Acceptance cases of the Nether Memory Model (docs/core-memory-managment.md §25).
use nether_core::{check::check, hir::Value, interpret, mir};
use nether_frontend::{parser::parse, source::SourceId};
use nether_semantics::OverflowChecks;

const PRELUDE: &str = "struct Cell {value:i32} struct Hold {cell:Cell} \
    fn inspect(u:Cell):i32 {return u.value} \
    fn modify(var u:Cell):() {u.value+=1} \
    fn store(x:Cell):Hold {return Hold {cell:x}} ";

fn check_source(body: &str) -> Result<nether_core::hir::Program, Vec<String>> {
    let source = format!("{PRELUDE}{body}");
    let parsed = parse(SourceId(0), &source);
    assert!(
        parsed.diagnostics.is_empty(),
        "{source}: {:?}",
        parsed.diagnostics
    );
    check(&parsed.module.unwrap())
        .map_err(|errors| errors.into_iter().map(|e| e.code.to_string()).collect())
}

fn run(body: &str) -> i128 {
    let program = check_source(body).unwrap_or_else(|e| panic!("{body}: {e:?}"));
    let cfg = mir::lower(&program);
    let a = interpret::execute(
        &program,
        program.main.unwrap(),
        vec![],
        OverflowChecks::Checked,
        10_000,
    )
    .unwrap();
    let b = mir::interpret::execute(
        &cfg,
        cfg.main.unwrap(),
        vec![],
        OverflowChecks::Checked,
        10_000,
    )
    .unwrap();
    assert_eq!(a, b, "{body}");
    let Value::Integer(n) = a else { panic!() };
    nether_core::rust::emit(&program, OverflowChecks::Checked).unwrap();
    n.signed_value().unwrap()
}

#[test]
fn n01_n02_aliases_and_sequential_reborrows() {
    for (body, expected) in [
        // N01: several readonly aliases of one object; parent restored afterwards.
        ("fn main():i32 {var a=Cell {value:1};let b=a;let c=b;let t=inspect(b)+inspect(c);modify(a);return t+a.value}", 4),
        // N02: each child ends before its parent is used again.
        ("fn main():i32 {var a=Cell {value:1};var b=a;b.value+=1;var c=b;modify(c);b.value+=1;a.value+=1;return a.value}", 5),
        // Mutable alias of an alias chain through calls.
        ("fn twice(var u:Cell):() {modify(u);modify(u)} fn main():i32 {var a=Cell {value:1};twice(a);return inspect(a)}", 3),
    ] {
        assert_eq!(run(body), expected, "{body}");
    }
}

#[test]
fn n05_n06_results_carry_ownership_and_access() {
    for (body, expected) in [
        // N05: a fresh result permits a mutable binding.
        ("fn make():Cell {return Cell {value:20}} fn main():i32 {var u=make();modify(u);return u.value}", 21),
        ("fn build():Cell {var local=Cell {value:20};modify(local);return local} fn main():i32 {var u=build();modify(u);return u.value}", 22),
        // N06: a `var` parameter returned keeps its write access.
        ("fn identity(var u:Cell):Cell {return u} fn main():i32 {var owner=Cell {value:1};var alias=identity(owner);modify(alias);return inspect(owner)}", 2),
        // A readonly result is a view of the argument.
        ("fn identity(u:Cell):Cell {return u} fn main():i32 {var o=Cell {value:5};let r=identity(o);let n=inspect(r);modify(o);return n+o.value}", 11),
    ] {
        assert_eq!(run(body), expected, "{body}");
    }
}

#[test]
fn consuming_parameters_require_owners() {
    for (body, expected) in [
        // Fresh value and owner without later use: ownership is transferred.
        ("fn main():i32 {let h=store(Cell {value:3});return h.cell.value}", 3),
        ("fn main():i32 {let o=Cell {value:4};let h=store(o);return h.cell.value}", 4),
        ("fn wrap(x:Cell,n:i32):Hold {if n==0 {return store(x)};return relay(x,n-1)} fn relay(x:Cell,n:i32):Hold {return wrap(x,n)} fn main():i32 {let h=wrap(Cell {value:8},2);return h.cell.value}", 8),
        // Transitive consumption through a call chain.
        ("fn wrap(x:Cell):Hold {return store(x)} fn main():i32 {let h=wrap(Cell {value:6});return h.cell.value}", 6),
    ] {
        assert_eq!(run(body), expected, "{body}");
    }
}

#[test]
fn n03_n09_access_and_ownership_violations_are_rejected() {
    for (body, code) in [
        // N03: no promotion of a readonly path, whatever the route.
        ("fn main():i32 {let a=Cell {value:1};var b=a;return 0}", "E0317"),
        ("fn f(u:Cell):() {modify(u)} fn main():i32 {return 0}", "E0317"),
        ("fn f(u:Cell):() {var e=u}  fn main():i32 {return 0}", "E0317"),
        ("fn relay(u:Cell):Cell {return u} fn relay2(u:Cell):Cell {return relay(u)} fn main():i32 {var o=Cell {value:1};var e=relay2(o);return 0}", "E0317"),
        // Loans: a live readonly view blocks writes, a live mutable view blocks all access.
        ("fn main():i32 {var o=Cell {value:1};let r=o;modify(o);return inspect(r)}", "E0381"),
        ("fn main():i32 {var o=Cell {value:1};var w=o;modify(o);modify(w);return 0}", "E0381"),
        // N07: a view of a local object cannot leave the function.
        ("fn bad():Cell {let o=Cell {value:1};let r=o;return r}  fn main():i32 {return 0}", "E0395"),
        // N08: use after the owner was consumed, or consumption under a live view.
        ("fn main():i32 {let o=Cell {value:1};let h=store(o);return o.value}", "E0384"),
        ("fn main():i32 {var o=Cell {value:1};let r=o;let h=store(o);return r.value}", "E0381"),
        ("fn main():i32 {let o=Cell {value:7};let v=o;let h=store(v);return h.cell.value}", "NMM_NO_OWNER_TOKEN"),
        ("fn main():i32 {let o=Cell {value:7};let ref v=o;let h=store(v);return h.cell.value}", "NMM_NO_OWNER_TOKEN"),
        ("fn wrap(x:Cell):Hold {return store(x)} fn main():i32 {let o=Cell {value:7};let v=o;let h=wrap(v);return h.cell.value}", "NMM_NO_OWNER_TOKEN"),
        ("fn main():i32 {let o=Cell {value:7};let v=o;let f=store;let h=f(v);return h.cell.value}", "NMM_NO_OWNER_TOKEN"),
        ("fn wrap<T>(x:T):(T,) {return (x,)} fn main():i32 {let o=Cell {value:7};let v=o;let h=wrap(v);return h.0.value}", "NMM_NO_OWNER_TOKEN"),
        ("fn borrow(x:Cell):Hold {let v=x;return Hold {cell:v}} fn main():i32 {let o=Cell {value:7};let h=borrow(o);let result=store(h.cell);return result.cell.value}", "NMM_NO_OWNER_TOKEN"),
        // N09: a `var` parameter is a mutable borrow and cannot be consumed.
        ("fn bad(var x:Cell):() {let h=Hold {cell:x}} fn main():i32 {return 0}", "E0398"),
    ] {
        let errors = check_source(body).expect_err(&format!("accepted {body}"));
        assert!(errors.iter().any(|c| c == code), "{body}: {errors:?}");
    }
}

#[test]
fn borrowed_storage_is_non_consuming_and_keeps_the_original_owner() {
    assert_eq!(run("fn borrow(x:Cell):Hold {let v=x;return Hold {cell:v}} fn main():i32 {let o=Cell {value:7};let h=borrow(o);return o.value+h.cell.value}"), 14);
}
