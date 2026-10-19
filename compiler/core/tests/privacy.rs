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
const CASES: &[(&str, i32)] = &[
    ("class C {private value:i32;constructor(n:i32) {value=n};private fn secret(this):i32 {return value};fn read(this):i32 {return secret()};destructor {value+=1}} fn main():i32 {let c=new C(7);return c.read()}", 7),
    ("class C {private value:i32;constructor(n:i32) {value=n};fn read_other(this,other:C):i32 {return other.value}} fn main():i32 {let a=new C(3);let b=new C(7);return a.read_other(b)}", 7),
    ("struct S {private value:i32;fn make(n:i32):S {return S {value:n}};fn read(this):i32 {return value}} fn main():i32 {return S.make(7).read()}", 7),
    ("struct S {private value:i32;fn make(n:i32):S {return S {value:n}};fn read(this):i32 {let S {value:n}=this;return n}} fn main():i32 {return S.make(7).read()}", 7),
    ("struct S {private fn secret():i32 {return 7};fn expose():()=>i32 {return S.secret}} fn main():i32 {let f=S.expose();return f()}", 7),
    ("class C<T:Copy> {private value:T;constructor(n:T) {value=n};private fn secret(this):T {return value};fn read(this):T {return secret()};destructor {let n=value}} fn main():i32 {let a=new C<i32>(3);let b=new C<i64>(4);return a.read()+b.read() as i32}", 7),
    ("struct S<T:Copy> {private value:T;fn make(n:T):S<T> {return S<T> {value:n}};private fn secret<U:Copy>(x:U):U {return x};fn pass<U:Copy>(x:U):U {let f:(U)=>U=S<T>.secret<U>;return f(x)};fn read(this):T {return value}} fn main():i32 {let s=S<i32>.make(3);return s.read()+S<i32>.pass<i32>(4)}", 7),
    ("class C {private value:i32;constructor(n:i32) {value=n};fn bump(var this):() {let ref var alias=this;alias.value+=1};fn read(this):i32 {return value}} fn main():i32 {var c=new C(6);c.bump();return c.read()}", 7),
    ("private fn helper():i32 {return 7} struct S {fn read():i32 {return helper()}} fn main():i32 {return S.read()}", 7),
    ("struct R {value:i32;destructor {}} class C {private value:R;constructor(n:R) {value=n};fn get(this):R {return value};destructor {let n=value.value}} fn main():i32 {let c=new C(R {value:7});return c.get().value}", 7),
    ("struct S {private hidden:i32;visible:i32;fn make():S {return S {hidden:3,visible:7}}} fn main():i32 {let s=S.make();let S {visible:n,..}=s;return n}", 7),
    ("class C<T:Copy> {private fn secret<U:Copy>(x:U):U {return x};fn expose():(i32)=>i32 {return C<T>.secret<i32>}} fn main():i32 {let f=C<i32>.expose();return f(7)}",7),
    ("class C<T:Copy> {private fn secret<U:Copy>(x:U):U {return x};fn expose():(i32)=>i32 {return C<T>.secret<i32>}} fn relay<T:Copy>(x:T):i32 {let f=C<T>.expose();return f(7)} fn main():i32 {return relay(1)}",7),
];
#[test]
fn declaring_types_keep_private_access_in_specialized_entries() {
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
fn private_members_are_not_module_visible_or_granted_by_callers() {
    for source in [
        "class C {private value:i32;constructor() {value=7}} fn main():i32 {let c=new C();return c.value}",
        "class C {private value:i32;constructor() {value=7}} class D {fn read(c:C):i32 {return c.value}} fn main():i32 {return D.read(new C())}",
        "class C {private fn secret():i32 {return 7}} fn main():i32 {return C.secret()}",
        "class C {private fn secret(this):i32 {return 7}} fn main():i32 {return new C().secret()}",
        "class C {private fn secret(this):i32 {return 7}} fn main():i32 {let f=C.secret;return f(new C())}",
        "class C {private fn secret():i32 {return 7}} fn main():i32 {let f=C.secret;return f()}",
        "class C<T:Copy> {private value:T;constructor(n:T) {value=n}} fn read<T:Copy>(c:C<T>):T {return c.value} fn main():i32 {return read(new C<i32>(7))}",
        "class C<T:Copy> {private fn secret<U:Copy>(x:U):U {return x}} fn main():i32 {return C<i32>.secret<i32>(7)}",
        "class C<T:Copy> {private fn secret<U:Copy>(x:U):U {return x}} fn main():i32 {let f:(i32)=>i32=C<i32>.secret<i32>;return f(7)}",
        "class C {private value:i32;constructor() {value=7};fn call(this):i32 {return helper(this)}} fn helper(c:C):i32 {return c.value} fn main():i32 {return new C().call()}",
        "struct S {private value:i32} fn main():i32 {let s=S {value:7};return 7}",
        "struct S {private value:i32;fn make():S {return S {value:7}}} fn main():i32 {let s=S.make();let S {value:n}=s;return n}",
        "class C {private value:i32;constructor() {value=7}} fn main():i32 {var c=new C();let ref var alias=c;alias.value=3;return 7}",
        "struct S {private value:i32;fn make():S {return S {value:7}}} struct D {fn read(s:S):i32 {return s.value}} fn main():i32 {return D.read(S.make())}",
] {
        let errors = checked(source).expect_err(source);
        assert!(errors.iter().any(|d| d.code == "E0352" && !d.notes.is_empty()), "{source}: {errors:?}");
    }
}
#[test]
fn native_private_entries_preserve_results_and_drop_order() {
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
    let dir = std::env::temp_dir().join(format!("nether-privacy-{}", std::process::id()));
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
