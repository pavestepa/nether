use nether_core::{check::check, hir::Value, interpret::execute};
use nether_frontend::modules::load;
use nether_semantics::OverflowChecks;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new(files: &[(&str, &str)]) -> Self {
        let root = std::env::temp_dir().join(format!(
            "nether-modules-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        for (path, text) in files {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        Self(root)
    }
    fn code(&self) -> i128 {
        let loaded = load(&self.0.join("main.nr"), "app");
        assert!(loaded.diagnostics.is_empty(), "{:?}", loaded.diagnostics);
        let program = check(&loaded.module.unwrap()).unwrap();
        let Value::Integer(value) = execute(
            &program,
            program.main.unwrap(),
            Vec::new(),
            OverflowChecks::Checked,
            10000,
        )
        .unwrap() else {
            panic!()
        };
        value.signed_value().unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn imports_and_reexports_resolve_before_type_checking() {
    let files = Fixture::new(&[
        (
            "main.nr",
            "import {add, VALUE} from \"app/api\"\nfn main():i32{return add(VALUE,1)}",
        ),
        ("api.nr", "export {add, VALUE} from \"./math\""),
        (
            "math.nr",
            "const VALUE:i32=6\nfn add(a:i32,b:i32):i32{return a+b}",
        ),
    ]);
    assert_eq!(files.code(), 7);
}
#[test]
fn cyclic_declarations_and_mutual_calls_are_allowed() {
    let files=Fixture::new(&[
        ("main.nr","import {even} from \"./even\"\nfn main():i32{return if even(10) {1} else {0}}"),
        ("even.nr","import {odd} from \"./odd\"\nfn even(n:i32):bool {if n==0 {return true};return odd(n-1)}"),
        ("odd.nr","import {even} from \"./even\"\nfn odd(n:i32):bool {if n==0 {return false};return even(n-1)}"),
    ]);
    assert_eq!(files.code(), 1);
}
#[test]
fn lexical_shadowing_and_source_ids_are_preserved() {
    let files = Fixture::new(&[
        (
            "main.nr",
            "import {VALUE} from \"./value\"\nfn main():i32 {let VALUE=VALUE+1;return VALUE}",
        ),
        ("value.nr", "const VALUE:i32=5"),
    ]);
    assert_eq!(files.code(), 6);
    fs::write(files.0.join("value.nr"), "const VALUE:i32=missing").unwrap();
    let loaded = load(&files.0.join("main.nr"), "app");
    let errors = check(&loaded.module.unwrap()).unwrap_err();
    let source = &loaded.sources[errors[0].span.source.0];
    assert!(source.name.ends_with("value.nr"));
}
#[test]
fn private_missing_conflicting_and_unfounded_cyclic_exports_are_rejected() {
    for (main, other) in [
        (
            "import {hidden} from \"./other\"",
            "private fn hidden():(){}",
        ),
        ("import {missing} from \"./other\"", "fn present():(){}"),
        ("import {f} from \"./other\"\nfn f():(){}", "fn f():(){}"),
        ("export {x} from \"./other\"", "export {x} from \"./main\""),
    ] {
        let files = Fixture::new(&[("main.nr", main), ("other.nr", other)]);
        let loaded = load(&files.0.join("main.nr"), "app");
        assert!(loaded.module.is_none());
        assert!(!loaded.diagnostics.is_empty());
    }
}
#[test]
fn package_root_escape_is_rejected_before_reading_the_target() {
    let files = Fixture::new(&[("main.nr", "import {x} from \"../outside\"")]);
    let loaded = load(&files.0.join("main.nr"), "app");
    assert_eq!(loaded.diagnostics[0].code, "E0501");
}
#[test]
fn static_methods_preserve_module_lookup_and_private_member_visibility() {
    let fixture=Fixture::new(&[("main.nr","import {Box} from \"./lib\"\nfn main():i32 {return Box<i32>.create(8).value}"),("lib.nr","fn value():i32 {return 1} #[Copy] struct Box<T:Copy> {value:T;fn create(x:T):Box<T> {let ignored=value();return Box<T> {value:x}};private fn secret():i32 {return 4}}")]);
    assert_eq!(fixture.code(), 8);
    fs::write(
        fixture.0.join("main.nr"),
        "import {Box} from \"./lib\"\nfn main():i32 {return Box<i32>.secret()}",
    )
    .unwrap();
    let loaded = load(&fixture.0.join("main.nr"), "app");
    assert!(loaded.diagnostics.is_empty());
    let errors = check(&loaded.module.unwrap()).unwrap_err();
    assert_eq!(errors[0].code, "E0352");
}
