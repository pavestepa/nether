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
const IMPORT:&str="import {RawStorage,allocate,capacity,initialize,take,drop_slot,release,replace,size_of,align_of,slot_size,slot_align} from \"core/intrinsic\"\n";
fn source(body: &str) -> String {
    format!("{IMPORT}{body}")
}
fn cases() -> Vec<(String, Option<i32>)> {
    let buffer = include_str!("../../../examples/scalar_buffer/main.nr");
    vec![
        (include_str!("../../../examples/raw_storage/main.nr").into(),Some(7)),
        (buffer.into(),Some(7)),
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
    ]
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
        "fn main():i32 {let s=allocate<i32>(1);return 0}",
        "fn main():i32 {unsafe {let s=allocate<i32>(1);let alias=s;release(alias)};return 0}",
        "fn main():i32 {unsafe {let s=allocate<i32>(1);release(s);return capacity(s) as i32}}",
        "fn main():i32 {unsafe {var s=allocate<i32>(1);let alias=s;initialize(alias,0,7)};return 0}",
        "fn main():i32 {unsafe {let s=allocate<i32>(1);let p=s.data};return 0}",
        "fn main():i32 {let n=7;let fake=RawStorage<i32> {data:&var n,cap:1};return 0}",
        "struct R {v:i32} fn main():i32 {unsafe {let s=allocate<R>(1)};return 0}",
        "#[Copy] struct R {v:i32} fn main():i32 {unsafe {let s=allocate<R>(1)};return 0}",
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
            let code = rust::emit(&p, mode)
                .unwrap()
                .replace("fn main() {", "fn unused_main() {");
            generated.push_str(&format!("mod case{id} {{\n{code}\n"));
            generated.push_str(&format!(
                r#"
pub fn check(fail:isize) {{
 let before=super::LIVE.load(super::Ordering::SeqCst);
 let allocated=super::ALLOCATED.load(super::Ordering::SeqCst);
 super::FAIL_AFTER.store(fail,super::Ordering::SeqCst);
 let result=unsafe{{f{}(&mut Context{{clock:0,panicking:false}})}};
 super::FAIL_AFTER.store(-1,super::Ordering::SeqCst);
 let after=super::LIVE.load(super::Ordering::SeqCst);
 let allocated_after=super::ALLOCATED.load(super::Ordering::SeqCst);
 assert_eq!(before,after,"allocation leak in case {id}");
 if fail>=0 {{assert!(result.is_err());}} else {{ {} }}
 {}
}}
}}
"#,
                p.main.unwrap(),
                expected.map_or("assert!(result.is_err());".into(), |n| format!(
                    "assert_eq!(result.unwrap(),{n});"
                )),
                if id == 3 || id == 4 {
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
