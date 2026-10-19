use nether_core::{check::check, hir, interpret, mir};
use nether_frontend::{parser::parse, source::SourceId};
use nether_semantics::OverflowChecks;

const PREFIX: &str = "struct R {value:i32;destructor {}} struct D {value:R;fn get(this):R {return value};destructor {let valid=1/(value.value-6)}} ";
const CASES: &[(&str, i32)] = &[
    ("class C {value:R;constructor(x:R) {value=x}} fn wrap(x:R):C {return new C(x)} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref w=b;var c=wrap(a);c.value=w;return c.value.value}",3),

    ("fn first(x:R,y:R):R {return x} fn second(x:R,y:R):R {return y} fn main():i32 {let a=R {value:7};let b=R {value:3};let f=if false {first} else {second};let result=f(a,b);return a.value+b.value+result.value}",13),

    ("fn main():i32 {let a=R {value:7};let ref v=a;let d=D {value:v};let r=d.get();return r.value}",7),
    ("fn main():i32 {let a=R {value:7};let ref v=a;var d=D {value:v};let b=R {value:7};let ref w=b;d=D {value:w};return 3}",3),

    ("fn one(x:R):R {return x} fn two(x:R):R {return x} fn choose(b:bool):(R)=>R {return if b {one} else {two}} fn main():i32 {let a=R {value:7};let f=choose(false);let b=f(a);return a.value+b.value}",14),
    ("fn id(x:R):R {return x} fn choose():(R)=>R {return id} fn main():i32 {let a=R {value:7};let b=choose()(a);return a.value+b.value}",14),

    ("fn id(x:R):R {return x} fn main():i32 {let a=R {value:7};let f=id;let b=f(a);return a.value+b.value}",14),
    ("fn id(x:R):R {return x} fn apply(f:(R)=>R,x:R):R {return f(x)} fn main():i32 {let a=R {value:7};let f=id;let b=apply(f,a);return a.value+b.value}",14),
    ("struct H {value:R} fn wrap(x:R):H {return H {value:x}} fn main():i32 {let a=R {value:7};let f=wrap;let h=f(a);return a.value+h.value.value}",14),
    ("struct H {value:R} fn set(var h:H,x:R):() {h.value=x} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref w=b;var h=H {value:a};let f=set;f(h,w);return h.value.value}",3),
    ("struct S {value:R;fn get(this):R {return value}} fn main():i32 {let a=R {value:7};let ref v=a;let s=S {value:v};let f=S.get;let b=f(s);return b.value}",7),
    ("fn one(x:R):R {return x} fn two(x:R):R {return x} fn main():i32 {let a=R {value:7};let f=if false {one} else {two};let b=f(a);return a.value+b.value}",14),
    ("fn id<T>(x:T):T {return x} fn main():i32 {let a=R {value:7};let f:(R)=>R=id;let b=f(a);return a.value+b.value}",14),

    ("class C {value:R;constructor(x:R) {value=x};destructor {let valid=1/(value.value-6)}} fn main():i32 {let a=R {value:7};let ref v=a;let c=new C(v);return 3}",3),
    ("class C {value:R;constructor(x:R) {value=x};fn get(this):R {return value}} fn main():i32 {let a=R {value:7};let ref v=a;let c=new C(v);let r=c.get();return r.value}",7),
    ("class C {value:R;constructor(x:R) {value=x}} fn wrap(x:R):C {return new C(x)} fn main():i32 {let a=R {value:7};let c=wrap(a);return a.value+c.value.value}",14),
    ("class C {value:R;constructor(x:R) {value=x};destructor {let valid=1/(value.value-6)}} fn main():i32 {let a=R {value:7};let ref v=a;let c=new C(v);return 1/0}",101),
    ("class C {value:R;constructor(x:R) {value=x};destructor {let valid=1/(value.value-6)}} fn main():i32 {let a=R {value:6};let ref v=a;let c=new C(v);return 3}",101),
    ("fn main():i32 {let a=R {value:7};let ref v=a;let d=D {value:v};return d.value.value}",7),
    ("fn main():i32 {let a=R {value:7};let ref v=a;let d=D {value:v};return 3}",3),
    ("fn main():i32 {let a=R {value:7};if true {let ref v=a;let d=D {value:v}};return a.value}",7),
    ("fn consume(d:D):i32 {return d.value.value} fn main():i32 {let a=R {value:7};let ref v=a;let d=D {value:v};return consume(d)}",7),
    ("fn wrap(x:R):D {return D {value:x}} fn main():i32 {let a=R {value:7};let d=wrap(a);return a.value+d.value.value}",14),
    ("fn id<T>(x:T):T {return x} fn main():i32 {let a=R {value:7};let ref v=a;let d=D {value:v};let moved=id(d);return moved.value.value}",7),
    ("struct G<T> {tag:T;value:R;destructor {let valid=1/(value.value-6)}} fn main():i32 {let a=R {value:7};let ref v=a;let d=G<i32> {tag:0,value:v};return 3}",3),
    ("struct Outer {d:D} fn main():i32 {let a=R {value:7};let ref v=a;let outer=Outer {d:D {value:v}};return 3}",3),
    ("fn main():i32 {let a=R {value:7};let ref v=a;var d=D {value:v};d=D {value:v};return 3}",3),
    ("fn main():i32 {let a=R {value:7};let ref v=a;let d=D {value:v};return 1/0}",101),
    ("fn main():i32 {let a=R {value:6};let ref v=a;let d=D {value:v};return 3}",101),
];
fn program(body: &str) -> hir::Program {
    let source = format!("{PREFIX}{body}");
    let parsed = parse(SourceId(0), &source);
    assert!(
        parsed.diagnostics.is_empty(),
        "{source}: {:?}",
        parsed.diagnostics
    );
    check(&parsed.module.unwrap()).unwrap_or_else(|e| panic!("{source}: {e:?}"))
}
#[test]
fn destructor_keeps_stored_views_alive_through_cleanup() {
    for (body, expected) in CASES {
        let p = program(body);
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
        if *expected == 101 {
            assert!(a.0.is_err(), "{body}: {a:?}")
        } else {
            assert!(
                matches!(a.0,Ok(hir::Value::Integer(n)) if n.signed_value()==Some(*expected as i128)),
                "{body}: {a:?}"
            );
        }
    }
}
#[test]
fn destructor_dependencies_reject_early_target_invalidation() {
    for body in [
        "fn set(var d:D,x:R):() {d.value=x} fn main():i32 {let a=R {value:7};let ref v=a;var d=D {value:v};let b=R {value:7};let ref w=b;let ref var alias=d;set(alias,w);return 0}",
        "fn first(x:R,y:R):R {return x} fn second(x:R,y:R):R {return y} fn main():i32 {var a=R {value:7};let b=R {value:3};let ref v=a;let ref w=b;let f=if false {first} else {second};let result=f(v,w);a=R {value:2};return result.value}",
        "fn one(x:R):R {return x} fn two(x:R):R {return R {value:3}} fn main():i32 {let a=R {value:7};let f=if false {one} else {two};let b=f(a);return a.value+b.value}",
        "fn id(x:R):R {return x} fn bad():R {let a=R {value:7};let ref v=a;let f=id;return f(v)} fn main():i32 {return 0}",
        "struct H {value:R} fn set(var h:H,x:R):() {h.value=x} fn main():i32 {let a=R {value:7};var b=R {value:3};let ref v=a;let ref w=b;var h=H {value:v};let f=set;f(h,w);b=R {value:2};return h.value.value}",
        "class C {value:R;constructor(x:R) {value=x}} fn bad():C {let a=R {value:7};let ref v=a;return new C(v)} fn main():i32 {return 0}",
        "class C {value:R;constructor(x:R) {value=x};destructor {let n=value.value}} fn main():i32 {var a=R {value:7};let ref v=a;let c=new C(v);a=R {value:3};return 0}",
        "fn main():i32 {var a=R {value:7};let ref v=a;let d=D {value:v};a=R {value:3};return 0}",
        "fn main():i32 {var a=R {value:7};let ref v=a;let d=D {value:v};a.value=3;return 0}",
        "fn main():i32 {let a=R {value:7};let ref v=a;var d=D {value:v};let b=R {value:7};let ref w=b;d.value=w;return 0}",
        "fn main():i32 {let a=R {value:7};let ref v=a;var d=D {value:v};if true {let b=R {value:7};let ref w=b;d.value=w};return 0}",
        "fn main():i32 {let a=R {value:7};let ref v=a;var d=D {value:v};var b=R {value:7};let ref var alias=b;*alias=R {value:7};let ref w=b;d.value=w;return 0}",
        "fn bad():D {let a=R {value:7};let ref v=a;return D {value:v}} fn main():i32 {return 0}",
    ] {
        let parsed = parse(SourceId(0),&format!("{PREFIX}{body}"));
        assert!(parsed.diagnostics.is_empty(),"{:?}",parsed.diagnostics);
        assert!(check(&parsed.module.unwrap()).is_err(),"accepted {body}");
    }
}
#[test]
fn native_borrowed_destructors_observe_live_targets_and_exact_drop_order() {
    use std::{fs, process::Command};
    let mut rust = String::new();
    let mut traces = Vec::new();
    for (i, (body, _)) in CASES.iter().enumerate() {
        let p = program(body);
        let (_, events) = interpret::execute_traced(
            &p,
            p.main.unwrap(),
            vec![],
            OverflowChecks::Checked,
            100_000,
        );
        traces.push(
            events
                .iter()
                .map(|e| format!("DROP:{}", e.entry))
                .collect::<Vec<_>>(),
        );
        let emitted = nether_core::rust::emit(&p, OverflowChecks::Checked)
            .unwrap()
            .replace("fn main()", "pub fn run()");
        let emitted = emitted
            .lines()
            .map(|line| {
                if let Some(id) = line
                    .strip_prefix("// destructor-entry ")
                    .or_else(|| line.strip_prefix("// class-destructor-entry "))
                {
                    format!("eprintln!(\"DROP:{id}\");")
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
    for i in 0..CASES.len() {
        rust.push_str(&format!("{i}=>case{i}::run(),\n"));
    }
    rust.push_str("_=>panic!(),}}\n");
    let dir = std::env::temp_dir().join(format!(
        "nether-borrowed-destructors-{}",
        std::process::id()
    ));
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
        for (i, (body, expected)) in CASES.iter().enumerate() {
            let result = Command::new(&binary).arg(i.to_string()).output().unwrap();
            let stderr = String::from_utf8_lossy(&result.stderr);
            assert_eq!(
                result.status.code(),
                Some(*expected),
                "O{level}: {body}: {stderr}"
            );
            assert_eq!(
                stderr
                    .lines()
                    .filter(|s| s.starts_with("DROP:"))
                    .map(str::to_owned)
                    .collect::<Vec<_>>(),
                traces[i],
                "O{level}: {body}"
            );
        }
    }
    fs::remove_dir_all(dir).unwrap();
}
