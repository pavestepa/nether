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
fn imported_virtual_override_diagnostics_preserve_base_declaration() {
    let files=Fixture::new(&[
        ("base.nr","class Base<T> {virtual fn read(this,value:T):T {return value}}"),
        ("api.nr","export {Base} from \"./base\""),
        ("main.nr","import {Base} from \"./api\"\nclass Child:Base<i32> {override fn read(this,value:i32):bool {return true}} fn main():i32 {return 0}"),
    ]);
    let loaded = load(&files.0.join("main.nr"), "app");
    assert!(loaded.diagnostics.is_empty(), "{:?}", loaded.diagnostics);
    let errors = check(&loaded.module.unwrap()).unwrap_err();
    let diagnostic = &errors[0];
    assert_eq!(diagnostic.code, "E0411", "{errors:?}");
    assert!(diagnostic
        .notes
        .iter()
        .any(|note| note.span.source != diagnostic.span.source));
    fs::write(files.0.join("main.nr"),"import {Base} from \"./api\"\nclass Child:Base<i32> {override fn read(this,value:i32):i32 {return value}} fn main():i32 {return new Child().read(7)}").unwrap();
    let loaded = load(&files.0.join("main.nr"), "app");
    assert!(loaded.diagnostics.is_empty(), "{:?}", loaded.diagnostics);
    let errors = check(&loaded.module.unwrap()).unwrap_err();
    assert_eq!(errors[0].code, "E0900", "{errors:?}");
}

#[test]
fn private_member_context_survives_imports_reexports_and_code_pointers() {
    let files = Fixture::new(&[
        ("value.nr", "private fn delta():i32 {return 1} class C<T:Copy> {private value:T;constructor(n:T) {value=n};private fn secret<U:Copy>(x:U):U {return x};fn expose(): (i32)=>i32 {return C<T>.secret<i32>};fn read(this):T {return value};destructor {let n=value;let d=delta()}}"),
        ("api.nr", "export {C} from \"./value\""),
        ("main.nr", "import {C} from \"./api\"\nfn main():i32 {let c=new C<i32>(3);let f=C<i32>.expose();return c.read()+f(4)}"),
    ]);
    assert_eq!(files.code(), 7);
    for source in [
        "import {C} from \"./api\"\nfn main():i32 {return new C<i32>(7).value}",
        "import {C} from \"./api\"\nfn main():i32 {let f:(i32)=>i32=C<i32>.secret<i32>;return f(7)}",
        "import {C} from \"./api\"\nclass Other {fn read(c:C<i32>):i32 {return c.value}} fn main():i32 {return Other.read(new C<i32>(7))}",
    ] {
        fs::write(files.0.join("main.nr"), source).unwrap();
        let loaded = load(&files.0.join("main.nr"), "app");
        assert!(loaded.diagnostics.is_empty(), "{:?}", loaded.diagnostics);
        let errors = check(&loaded.module.unwrap()).unwrap_err();
        let diagnostic = errors.iter().find(|d| d.code == "E0352").unwrap();
        assert!(diagnostic.notes.iter().any(|note| note.span.source != diagnostic.span.source), "{errors:?}");
    }
}

#[test]
fn nominal_interface_constraints_survive_imports_and_reexports() {
    let files=Fixture::new(&[
        ("contracts.nr","interface Read {fn read(this):i32} interface Get<T> {fn get(this):T} struct U {} fn get<T:Get<U>,U:Copy>(x:T):U {return x.get()} fn read<T:Read>(x:T):i32 {return x.read()}"),
        ("values.nr","import {Read, Get} from \"./contracts\"\nclass S:Read,Get<i32> {value:i32;constructor(n:i32) {value=n};fn read(this):i32 {return value};fn get(this):i32 {return value}}"),
        ("api.nr","export {Read, read, get} from \"./contracts\"\nexport {S} from \"./values\""),
        ("main.nr","import {S, read, get} from \"./api\"\nfn main():i32 {let s=new S(7);return read(s)+get(s)+s.read()}"),
    ]);
    assert_eq!(files.code(), 21);
    let loaded = load(&files.0.join("main.nr"), "app");
    let p = check(&loaded.module.unwrap()).unwrap();
    let m = nether_core::mir::lower(&p);
    let value = nether_core::mir::interpret::execute(
        &m,
        m.main.unwrap(),
        Vec::new(),
        OverflowChecks::Checked,
        100_000,
    )
    .unwrap();
    let Value::Integer(n) = value else { panic!() };
    assert_eq!(n.signed_value(), Some(21));
    nether_core::rust::emit(&p, OverflowChecks::Checked).unwrap();
    fs::write(
        files.0.join("values.nr"),
        "class S {value:i32;constructor(n:i32) {value=n};fn read(this):i32 {return value}}",
    )
    .unwrap();
    let loaded = load(&files.0.join("main.nr"), "app");
    assert!(loaded.diagnostics.is_empty(), "{:?}", loaded.diagnostics);
    let errors = check(&loaded.module.unwrap()).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| e.code == "E0402" && !e.notes.is_empty()),
        "{errors:?}"
    );
}

#[test]
fn borrowed_classes_destructors_and_callable_contracts_survive_reexports() {
    let files=Fixture::new(&[
        ("api.nr","export {R, Cell, id, set} from \"./values\""),
        ("values.nr","struct R {value:i32;destructor {}} class Cell<T> {value:T;constructor(x:T) {value=x};fn get(this):T {return value};destructor {}} fn id<T>(x:T):T {return x} fn set<T>(var h:Cell<T>,x:T):() {h.value=x}"),
        ("main.nr","import {R, Cell, id, set} from \"./api\"\nfn main():i32 {let a=R {value:7};let b=R {value:3};let ref v=a;let ref w=b;var c=new Cell<R>(v);let f:(var Cell<R>,R)=>()=set;f(c,w);let g:(R)=>R=id;let result=g(c.get());return result.value}"),
    ]);
    assert_eq!(files.code(), 3);
    fs::write(files.0.join("main.nr"),"import {R, Cell, id, set} from \"./api\"\nfn main():i32 {let a=R {value:7};var b=R {value:3};let ref v=a;let ref w=b;var c=new Cell<R>(v);let f:(var Cell<R>,R)=>()=set;f(c,w);b=R {value:2};return c.value.value}").unwrap();
    let loaded = load(&files.0.join("main.nr"), "app");
    assert!(loaded.diagnostics.is_empty(), "{:?}", loaded.diagnostics);
    let diagnostics = check(&loaded.module.unwrap()).unwrap_err();
    assert!(
        diagnostics.iter().any(|d| d.code == "E0381"),
        "{diagnostics:?}"
    );
}

#[test]
fn local_storage_modes_are_inferred_before_imported_generic_calls() {
    let files = Fixture::new(&[
        ("api.nr", "export {Holder, relay} from \"./storage\""),
        ("storage.nr", "struct Holder<T> {value:T} fn relay<T>(value:T):T {return value}"),
        ("main.nr", "import {Holder, relay} from \"./api\"\nstruct R {value:i32;destructor {}} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref w=b;var h=Holder<R> {value:a};h.value=w;let result=relay(h);return result.value.value}"),
    ]);
    assert_eq!(files.code(), 3);
    fs::write(files.0.join("main.nr"), "import {Holder, relay} from \"./api\"\nstruct R {value:i32;destructor {}} fn main():i32 {let b=R {value:3};let ref w=b;var h=Holder<R> {value:R {value:7}};h.value=w;return h.value.value}").unwrap();
    let loaded = load(&files.0.join("main.nr"), "app");
    assert!(loaded.diagnostics.is_empty());
    let diagnostics = check(&loaded.module.unwrap()).unwrap_err();
    assert!(
        diagnostics.iter().any(|d| d.code == "E0396"),
        "{diagnostics:?}"
    );
}

#[test]
fn enum_refinement_survives_imported_storage_specialization() {
    let files = Fixture::new(&[
        ("api.nr", "export {R, E, clear} from \"./storage\""),
        ("storage.nr", "struct R {value:i32;destructor {}} enum E {Item(R),Empty} fn clear(e:E):i32 {var current=e;current=E.Empty;return match current {E.Item(x)=>x.value,E.Empty=>3}}"),
        ("main.nr", "import {R, E, clear} from \"./api\"\nfn main():i32 {let a=R {value:7};let ref v=a;return clear(E.Item(v))}"),
    ]);
    assert_eq!(files.code(), 3);
}

#[test]
fn mutable_storage_contracts_survive_imports_and_generic_reexports() {
    let files = Fixture::new(&[
        ("api.nr", "export {Holder, set} from \"./storage\""),
        ("storage.nr", "struct Holder<T> {value:T} fn set<T>(var h:Holder<T>,x:T):() {h.value=x}"),
        ("main.nr", "import {Holder, set} from \"./api\"\nstruct R {value:i32;destructor {}} fn main():i32 {let a=R {value:7};let b=R {value:3};let ref v=a;let ref w=b;var h=Holder<R> {value:v};set(h,w);return h.value.value}"),
    ]);
    assert_eq!(files.code(), 3);
    fs::write(files.0.join("main.nr"), "import {Holder, set} from \"./api\"\nstruct R {value:i32;destructor {}} fn main():i32 {let a=R {value:7};var b=R {value:3};let ref v=a;let ref w=b;var h=Holder<R> {value:v};set(h,w);b=R {value:2};return h.value.value}").unwrap();
    let loaded = load(&files.0.join("main.nr"), "app");
    assert!(loaded.diagnostics.is_empty());
    let diagnostics = check(&loaded.module.unwrap()).unwrap_err();
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == "E0381" && d.notes.len() >= 2),
        "{diagnostics:?}"
    );
}

#[test]
fn borrowed_aggregate_contracts_survive_generic_imports_and_reexports() {
    let files = Fixture::new(&[
        ("api.nr", "export {Holder, wrap, get} from \"./storage\""),
        ("storage.nr", "struct Holder<T> {value:T} fn wrap<T>(x:T):Holder<T> {return Holder<T> {value:x}} fn get<T>(x:Holder<T>):T {return x.value}"),
        ("main.nr", "import {Holder, wrap, get} from \"./api\"\nstruct R {value:i32;destructor {}} fn main():i32 {let a=R {value:7};let h=wrap(a);let result=get(h);return a.value+result.value}"),
    ]);
    assert_eq!(files.code(), 14);
    fs::write(files.0.join("main.nr"), "import {Holder, wrap, get} from \"./api\"\nstruct R {value:i32;destructor {}} fn main():i32 {var a=R {value:7};let ref v=a;let h=wrap(v);let result=get(h);a=R {value:2};return result.value}").unwrap();
    let loaded = load(&files.0.join("main.nr"), "app");
    assert!(loaded.diagnostics.is_empty());
    let diagnostics = check(&loaded.module.unwrap()).unwrap_err();
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == "E0381" && !d.notes.is_empty()),
        "{diagnostics:?}"
    );
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

#[test]
fn imported_generic_destructor_uses_private_definition_context() {
    let files=Fixture::new(&[
        ("main.nr","import {R} from \"./resource\"; fn main():i32 {let a=R<i32>.make(7);return a.read()}"),
        ("resource.nr","struct R<T:Copy> {private value:T;fn make(value:T):R<T> {return R<T> {value:value}};fn read(this):T {return value};destructor {let final_value=value}}"),
    ]);
    assert_eq!(files.code(), 7);
    let loaded = load(&files.0.join("main.nr"), "app");
    let program = check(&loaded.module.unwrap()).unwrap();
    let mir = nether_core::mir::lower(&program);
    let (a, ae) = nether_core::interpret::execute_traced(
        &program,
        program.main.unwrap(),
        vec![],
        OverflowChecks::Checked,
        10000,
    );
    let (b, be) = nether_core::mir::interpret::execute_traced(
        &mir,
        mir.main.unwrap(),
        vec![],
        OverflowChecks::Checked,
        10000,
    );
    assert_eq!(a, b);
    assert_eq!(ae, be);
    assert_eq!(ae.len(), 1);
    nether_core::rust::emit(&program, OverflowChecks::Checked).unwrap();
}

#[test]
fn imported_class_constructor_methods_and_destructor_keep_definition_context() {
    let files=Fixture::new(&[
        ("main.nr","import {Cell} from \"./cell\"; fn main():i32 {var c=new Cell<i32>(4);c.bump();return c.read()}"),
        ("cell.nr","private fn delta():i32 {return 3} class Cell<T:Copy> {private value:T;private n:i32;constructor(v:T) {value=v;n=delta()};fn bump(var this):() {n+=1};fn read(this):i32 {return n};destructor {let x=value}}"),
    ]);
    assert_eq!(files.code(), 4);
    fs::write(
        files.0.join("main.nr"),
        "import {Cell} from \"./cell\"; fn main():i32 {let c=new Cell<i32>(4);return c.n}",
    )
    .unwrap();
    let loaded = load(&files.0.join("main.nr"), "app");
    assert!(check(&loaded.module.unwrap())
        .unwrap_err()
        .iter()
        .any(|e| e.code == "E0352"));
}

#[test]
fn imported_generic_borrowed_results_preserve_lifetime_contracts() {
    let files=Fixture::new(&[
        ("main.nr","import {Cell, relay} from \"./api\"\nfn main():i32 {let a=Cell {value:7};let b=relay(a);return a.value+b.value}"),
        ("api.nr","export {Cell, relay} from \"./implementation\""),
        ("implementation.nr","struct Cell {value:i32} fn id<T>(x:T):T {return x} fn relay<T>(x:T):T {return id(x)}"),
    ]);
    assert_eq!(files.code(), 14);
    fs::write(files.0.join("main.nr"),"import {Cell, relay} from \"./api\"\nfn main():i32 {var a=Cell {value:7};let ref x=a;let b=relay(x);a=Cell {value:8};return b.value}").unwrap();
    let loaded = load(&files.0.join("main.nr"), "app");
    assert!(loaded.diagnostics.is_empty());
    let errors = check(&loaded.module.unwrap()).unwrap_err();
    assert!(errors.iter().any(|e| e.code == "E0381"), "{errors:?}");
}
