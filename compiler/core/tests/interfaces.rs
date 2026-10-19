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
    ("interface Read {fn read(this):i32} struct S:Read {value:i32;fn read(this):i32 {return value}} fn read<T:Read>(x:T):i32 {return x.read()} fn main():i32 {return read(S {value:7})}",7),
    ("interface Read {fn read(this):i32} interface Other {fn other(this):i32} struct S:Read,Other {fn read(this):i32 {return 3};fn other(this):i32 {return 4}} fn sum<T:Read & Other>(x:T):i32 {return x.read()+x.other()} fn main():i32 {return sum(S {})}",7),
    ("interface Read {fn read(this):i32} #[Copy] struct S:Read {value:i32;fn read(this):i32 {return value}} fn sum<T:Read & Copy>(x:T):i32 {let y=x;return x.read()+y.read()} fn main():i32 {return sum(S {value:4})}",8),
    ("interface Read {fn read(this):i32} interface Bump {fn bump(var this):()} struct S:Read,Bump {value:i32;fn read(this):i32 {return value};fn bump(var this):() {value+=1}} fn bump<T:Read & Bump>(var x:T):i32 {x.bump();return x.read()} fn main():i32 {var s=S {value:3};let n=bump(s);return n+s.read()}",8),
    ("interface Map<T,R> {fn map(this,x:T):R} struct S:Map<i32,bool> {fn map(this,x:i32):bool {return x>3}} fn map<T:Map<i32,bool>>(x:T):bool {return x.map(7)} fn main():i32 {return if map(S {}) {7} else {0}}",7),
    ("interface Read {fn read(this):i32} struct S:Read {value:i32;fn read(this):i32 {return value}} struct Holder<T:Read> {value:T;fn read(this):i32 {return value.read()}} fn main():i32 {let h=Holder<S> {value:S {value:7}};return h.read()}",7),
    ("interface Read {fn read(this):i32} class S:Read {value:i32;constructor(n:i32) {value=n};fn read(this):i32 {return value}} fn read<T:Read>(x:T):i32 {return x.read()} fn main():i32 {let s=new S(7);return read(s)+s.read()}",14),
    ("interface Read {fn read(this):i32} struct S:Read {value:i32;fn read(this):i32 {return value}} fn read<T:Read>(x:T):i32 {return x.read()} fn relay<U:Read>(x:U):i32 {return read(x)} fn main():i32 {return relay(S {value:7})}",7),
    ("interface Read {fn read(this):i32} struct S:Read {fn read(this):i32 {return 7}} fn read<T:Read>(x:T,n:i32):i32 {if n==0 {return x.read()};return read(x,n-1)} fn main():i32 {return read(S {},3)}",7),
    ("interface Identity {fn id<T:Copy>(this,x:T):T} struct S:Identity {fn id<U:Copy>(this,x:U):U {return x}} fn read<T:Identity>(x:T):i32 {return x.id(7)} fn main():i32 {return read(S {})}",7),
    ("interface First<const N:usize> {fn first(this,xs:{i32;N}):i32} struct S:First<2> {fn first(this,xs:{i32;2}):i32 {return xs[0]}} fn read<T:First<2>>(x:T):i32 {return x.first({7,8})} fn main():i32 {return read(S {})}",7),
    ("interface Factory {fn make():i32} struct S:Factory {fn make():i32 {return 7}} fn make<T:Factory>():i32 {return T.make()} fn main():i32 {return make<S>()}",7),
    ("interface Get<T> {fn get(this):T} struct R {value:i32;destructor {}} struct S:Get<R> {value:R;fn get(this):R {return value}} fn get<T:Get<R>>(x:T):i32 {return x.get().value} fn main():i32 {let s=S {value:R {value:7}};return s.value.value+get(s)}",14),
    ("interface Read {fn read(this):i32} struct S<T:Copy>:Read {value:T;fn read(this):i32 {return 7}} fn read<T:Read>(x:T):i32 {return x.read()} fn main():i32 {return read(S<i32> {value:3})}",7),
    ("interface Read {fn read(this):i32} struct S:Read {fn read(this):i32 {return 7}} fn read<T:Read=S>(x:T):i32 {return x.read()} fn main():i32 {return read(S {})}",7),
    ("interface Read {fn read(this):i32} struct S:Read {fn read(this):i32 {return 7}} fn read<T:Read & Read>(x:T):i32 {return x.read()} fn main():i32 {return read(S {})}",7),
    ("interface Get<T=i32> {fn get(this):T} struct S:Get {fn get(this):i32 {return 7}} fn get<T:Get>(x:T):i32 {return x.get()} fn main():i32 {return get(S {})}",7),
    ("interface Identity {fn id<const N:usize>(this,x:{i32;N}):i32} struct S:Identity {fn id<const M:usize>(this,x:{i32;M}):i32 {return x[0]}} fn read<T:Identity>(x:T):i32 {return x.id<2>({7,8})} fn main():i32 {return read(S {})}",7),
    ("interface Factory {fn make():i32} struct S<T>:Factory {value:T;fn make():i32 {return 7}} fn make<T:Factory>():i32 {let f:()=>i32=T.make;return f()} fn main():i32 {return make<S<i32>>()}",7),
    ("interface I {fn read(this):i32} interface J {fn read(this):i32} struct S:I,J {fn read(this):i32 {return 7}} fn read<T:I & J>(x:T):i32 {return x.read()} fn main():i32 {return read(S {})}",7),
    ("interface Get<T> {fn get(this):T} struct R {value:i32;destructor {}} struct S:Get<R> {value:R;fn get(this):R {return value}} struct Adapter<T:Get<R>> {value:T;fn get(this):R {return value.get()}} fn main():i32 {let s=Adapter<S> {value:S {value:R {value:7}}};return s.get().value+s.value.value.value}",14),
    ("interface Read {fn read(this):i32} struct S:Read {fn read(this):i32 {return 7}} struct H<T:Read> {value:T} interface Use<T> {fn use(this,value:T):i32} struct U:Use<H<S>> {fn use(this,value:H<S>):i32 {return value.value.read()}} fn read<T:Use<H<V>>,V:Read>(x:T,value:V):i32 {return x.use(H<V> {value:value})} fn main():i32 {return read(U {},S {})}",7),
    ("interface Read {fn read(this):i32} struct S:Read {fn read(this):i32 {return 7}} struct H<T:Read> {value:T} interface Use<T> {fn use(this,value:T):i32} struct U:Use<H<S>> {fn use(this,value:H<S>):i32 {return value.value.read()}} interface Run {fn run<T:Use<H<V>>,V:Read>(this,x:T,value:V):i32} struct Runner:Run {fn run<A:Use<H<B>>,B:Read>(this,x:A,value:B):i32 {return x.use(H<B> {value:value})}} fn run<T:Run>(x:T):i32 {return x.run(U {},S {})} fn main():i32 {return run(Runner {})}",7),
];
#[test]
fn nominal_interfaces_dispatch_statically_and_preserve_borrowed_contracts() {
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
fn rejects_missing_or_incompatible_nominal_implementations() {
    for source in [
        "interface I {fn get<T=T>(this,x:T):T} fn main():() {}",
        "interface I {fn get<T=U,U=i32>(this,x:T):T} fn main():() {}",
        "interface I<T=Missing> {} fn main():() {}",
        "interface I<T=T> {} fn main():() {}",
        "interface I<T=U,U=i32> {} fn main():() {}",
        "interface I {fn read(this):Missing} fn main():() {}",
        "interface I<T:Missing> {} fn main():() {}",
        "interface I {fn read<const N:usize>(this,xs:{i32;N}):i32} struct S:I {fn read<const M:usize>(this,xs:{i32;M*M}):i32 {return xs[0]}} fn main():() {}",
        "interface I {fn read<T=i32>(this,x:T):T} struct S:I {fn read<U=bool>(this,x:U):U {return x}} fn main():() {}",
        "interface I {fn read(this):i32} struct S:I {} fn main():() {}",
        "interface I {fn read(this):i32} struct S:I {private fn read(this):i32 {return 1}} fn main():() {}",
        "interface I {fn read(this):i32} struct S:I {fn read(var this):i32 {return 1}} fn main():() {}",
        "interface I {fn read(var this):i32} struct S:I {fn read(this):i32 {return 1}} fn main():() {}",
        "interface I {fn read(this):i32} struct S:I {fn read():i32 {return 1}} fn main():() {}",
        "interface I {fn read(this):i32} struct S:I {fn read(this):bool {return true}} fn main():() {}",
        "interface I {fn read(this,var x:i32):i32} struct S:I {fn read(this,x:i32):i32 {return x}} fn main():() {}",
        "interface I {fn read(this):i32} struct S {fn read(this):i32 {return 1}} fn read<T:I>(x:T):i32 {return x.read()} fn main():i32 {return read(S {})}",
        "interface I {fn read(this):i32} struct S:I {fn read(this):i32 {return 1}} fn read<T:I & Copy>(x:T):i32 {return x.read()} fn main():i32 {return read(S {})}",
        "interface I {fn read(this):i32} interface J {fn other(this):i32} struct S:I {fn read(this):i32 {return 1}} fn read<T:I & J>(x:T):i32 {return x.read()} fn main():i32 {return read(S {})}",
        "interface I<T> {fn get(this):T} struct S:I<bool> {fn get(this):i32 {return 1}} fn main():() {}",
        "interface I {fn read<T:Copy>(this,x:T):T} struct S:I {fn read<U>(this,x:U):U {return x}} fn main():() {}",
        "interface I {fn read(this):i32;fn read(this):i32} fn main():() {}",
        "interface I {} struct I {} fn main():() {}",
        "interface I {} struct S:I,I {} fn main():() {}",
        "interface I {} struct S:Missing {} fn main():() {}",
        "interface I {fn bump(var this):()} struct S:I {fn bump(var this):() {}} fn bump<T:I>(x:T):() {x.bump()} fn main():() {bump(S {})}",
        "interface I {fn read(this):i32} struct S:I {fn read(this):i32 {return 1};fn extra(this):i32 {return 2}} fn read<T:I>(x:T):i32 {return x.extra()} fn main():i32 {return read(S {})}",
        "struct S {fn read(this):i32 {return 1}} fn read<T>(x:T):i32 {return x.read()} fn main():i32 {return read(S {})}",
        "interface I {fn read(this):i32} #[Copy] struct S:I {fn read(this):i32 {return 1}} fn needs_copy<T:Copy>(x:T):T {return x} fn copy<T:I>(x:T):T {return needs_copy(x)} fn main():i32 {return copy(S {}).read()}",
    ] {
        assert!(checked(source).is_err(),"accepted {source}");
    }
}
#[test]
fn native_interface_calls_match_reference_results_and_drop_order() {
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
    let dir = std::env::temp_dir().join(format!("nether-interfaces-{}", std::process::id()));
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
