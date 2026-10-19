use nether_core::{check::check, hir, interpret, mir};
use nether_frontend::{parser::parse, source::SourceId};
use nether_semantics::OverflowChecks;

fn checked(source: &str) -> Result<hir::Program, Vec<nether_frontend::source::Diagnostic>> {
    let parsed = parse(SourceId(0), source);
    assert!(
        parsed.diagnostics.is_empty(),
        "{source}: {:?}",
        parsed.diagnostics
    );
    check(&parsed.module.unwrap())
}
const CASES:&[(&str,i32)]=&[
    ("class A {value:i32;constructor(n:i32) {value=n};virtual fn read(this):i32 {return value}} fn main():i32 {let a=new A(7);return a.read()}",7),
    ("class A<T:Copy> {value:T;constructor(n:T) {value=n};virtual fn read(this):T {return value}} fn main():i32 {let a=new A<i32>(7);let f:(A<i32>)=>i32=A<i32>.read;return f(a)+a.read()}",14),
    ("class A {value:i32;constructor(n:i32) {value=n};virtual fn bump(var this):() {value+=1};fn read(this):i32 {return value}} fn main():i32 {var a=new A(7);a.bump();return a.read()}",8),
    ("class A {virtual fn identity<T:Copy>(this,value:T):T {return value}} fn main():i32 {let a=new A();let f:(A,i32)=>i32=A.identity<i32>;return f(a,7)}",7),
    ("interface Read {fn read(this):i32} class A:Read {virtual fn read(this):i32 {return 7}} fn read<T:Read>(x:T):i32 {return x.read()} fn main():i32 {return read(new A())}",7),
    ("class A {value:i32;constructor(n:i32) {value=n};virtual fn bump(var this):() {value+=1};destructor {bump()}} fn main():i32 {let a=new A(7);return a.value}",7),
    ("struct R {value:i32;destructor {}} class A {value:R;constructor(n:R) {value=n};virtual fn get(this):R {return value}} fn main():i32 {let a=new A(R {value:7});return a.get().value}",7),
];
#[test]
fn known_concrete_virtual_methods_keep_static_entry_contracts() {
    for (source, expected) in CASES {
        let p = checked(source).unwrap_or_else(|e| panic!("{source}: {e:?}"));
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
            matches!(a.0,Ok(hir::Value::Integer(n)) if n.signed_value()==Some(*expected as i128)),
            "{source}: {a:?}"
        );
        let rust = nether_core::rust::emit(&p, OverflowChecks::Checked).unwrap();
        assert!(!rust.contains("dyn "));
    }
}
#[test]
fn invalid_inheritance_is_diagnosed_before_the_lowering_boundary() {
    for (source,code) in [
        ("class A:A {}", "E0410"),
        ("class A<T>:B<T> {} class B<U>:A<U> {}", "E0410"),
        ("class A {} class B {} class C:A,B {}", "E0410"),
        ("struct A {} class B:A {}", "E0410"),
        ("class A {fn read(this):i32 {return 1}} class B:A {override fn read(this):i32 {return 2}}", "E0411"),
        ("class A {virtual fn read(this):i32 {return 1}} class B:A {override fn read(var this):i32 {return 2}}", "E0411"),
        ("class A {virtual fn read<T:Copy>(this,x:T):T {return x}} class B:A {override fn read<U>(this,x:U):U {return x}}", "E0411"),
    ] {
        let errors=checked(source).unwrap_err();
        assert_eq!(errors[0].code,code,"{source}: {errors:?}");
        if code=="E0411" {assert!(!errors[0].notes.is_empty(),"{errors:?}");}
    }
    // Valid declarations are not mistaken for working derived payload lowering.
    let errors=checked("class A {virtual fn read(this):i32 {return 1}} class B:A {override fn read(this):i32 {return 2}} fn main():i32 {return new B().read()}").unwrap_err();
    assert_eq!(errors[0].code, "E0900");
}
#[test]
fn native_static_virtual_entries_preserve_results_and_cleanup() {
    use std::{fs, process::Command};
    let mut rust = String::new();
    let mut traces = Vec::new();
    for (i, (source, _)) in CASES.iter().enumerate() {
        let p = checked(source).unwrap_or_else(|e| panic!("{source}: {e:?}"));
        let (_, events) = interpret::execute_traced(
            &p,
            p.main.unwrap(),
            vec![],
            OverflowChecks::Checked,
            100_000,
        );
        traces.push(
            events
                .into_iter()
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
    let dir = std::env::temp_dir().join(format!("nether-inheritance-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let source = dir.join("cases.rs");
    fs::write(&source, rust).unwrap();
    for level in ["0", "3"] {
        let binary = dir.join(format!("cases-{level}"));
        let output = Command::new(std::env::var("NETHER_RUSTC").unwrap_or_else(|_| "rustc".into()))
            .arg("--edition=2021")
            .args(["-C", &format!("opt-level={level}")])
            .arg(&source)
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        for (i, (_, expected)) in CASES.iter().enumerate() {
            let output = Command::new(&binary).arg(i.to_string()).output().unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert_eq!(
                output.status.code(),
                Some(*expected),
                "case {i} O{level}: {stderr}"
            );
            assert_eq!(
                stderr
                    .lines()
                    .filter(|s| s.starts_with("DROP:"))
                    .map(str::to_owned)
                    .collect::<Vec<_>>(),
                traces[i],
                "case {i} O{level}"
            );
        }
    }
    fs::remove_dir_all(dir).unwrap();
}
