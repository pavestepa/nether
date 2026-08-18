use super::*;

#[test]
fn reading_a_weak_field_calls_weak_upgrade_and_builds_an_option() {
    // `Option` is an ordinary prelude `enum` now (`stdlib/option.nr`), not
    // a compiler builtin — this crate's `compile` helper resolves a bare
    // parsed `Module` directly (`nether_resolver::resolve`, no driver, no
    // prelude loading), so the test declares its own stand-in with the
    // same shape. Codegen only ever sees a resolved `DefId`, so this
    // exercises the exact same lowering as the real bundled `Option`.
    let (_cg, ir) = compile(
        r#"
enum Option<T> {
    Some(T),
    None,
}
struct Child { name String }
struct Parent { kid weak Child }
fn describe(p Parent) String {
    return match p.kid {
        Some(c) => c.name,
        None => "none",
    };
}
fn main() {
    let c = Child { name = "Rex" };
    let p = Parent { kid = c };
    println(describe(p));
}
"#,
    );
    assert!(
        ir.contains("call i8 @nether_rt_arc_weak_upgrade("),
        "expected the weak field read to call weak_upgrade:\n{ir}"
    );
}

#[test]
fn a_heap_structs_own_drop_shim_releases_its_weak_fields_via_weak_release() {
    let (_cg, ir) = compile(
        r#"
struct Child { name String }
struct Parent { kid weak Child }
fn main() {
    let c = Child { name = "Rex" };
    let p = Parent { kid = c };
}
"#,
    );
    assert!(
        ir.contains("define void @nether_shim_drop_"),
        "expected a generated drop shim for Parent:\n{ir}"
    );
    assert!(
        ir.split("define void @nether_shim_drop_")
            .skip(1)
            .any(|shim| shim
                .split("\n}")
                .next()
                .is_some_and(|body| body.contains("nether_rt_arc_weak_release"))),
        "expected Parent's own drop shim to weak_release its `kid` field:\n{ir}"
    );
}

#[test]
fn emits_a_valid_object_file() {
    let (_cg, _ir) = compile("fn main() { println(\"hi\"); }");
    // Re-run generate against a fresh Codegen since ModuleCx borrows it —
    // `compile`'s own module already verified; this exercises the actual
    // object-emission path end to end.
    let cg = Codegen::new();
    let mut map = nether_diagnostics::SourceMap::new();
    let source = "fn main() { println(\"hi\"); }";
    let file = map.add_file("test.nr", source);
    let (module, _) = nether_parser::parse_module(source, file);
    let (resolved, _) = nether_resolver::resolve(&module);
    let (tables, _) = nether_typecheck::check(&module, &resolved);
    let hir = nether_hir::lower(&module, &resolved, tables);
    let main_id = *hir.fn_by_name.get(&Symbol::new("main")).unwrap();
    let mono = nether_monomorphization::monomorphize(&hir, main_id);
    let mut functions = nether_mir::build_mir(&mono, &resolved.definitions, &hir.signatures);
    nether_mir::insert_arc(&mut functions);
    let m = nether_codegen::generate(
        &cg,
        "objtest",
        &functions,
        &resolved.definitions,
        &hir.signatures,
    );
    m.verify().expect("module should verify");

    let out = std::env::temp_dir().join("nether_codegen_test.o");
    m.emit_object(&out).expect("emit_object should succeed");
    assert!(std::fs::metadata(&out).unwrap().len() > 0);
    let _ = std::fs::remove_file(&out);
}

#[test]
fn option_of_heap_type_compiles_and_matches_its_payload() {
    // Same stand-in as above — see that test's comment.
    let (_cg, ir) = compile(
        r#"
enum Option<T> {
    Some(T),
    None,
}
struct Child { name String }
fn describe(x Option<Child>) String {
    return match x {
        Some(c) => c.name,
        None => "none",
    };
}
fn main() {
    let c = Child { name = "Rex" };
    println(describe(Option.Some(c)));
}
"#,
    );
    assert!(
        ir.contains("payload"),
        "expected codegen for Option's payload field:\n{ir}"
    );
}

#[test]
fn generic_identity_return_is_concrete_at_codegen() {
    let (_cg, ir) = compile(
        r#"
fn identity<T>(value T) T { return value; }
fn main() {
    println(`${identity(42)}`);
    println(`${identity(true)}`);
}
"#,
    );
    assert!(
        ir.contains("nether_identity"),
        "expected specialized identity functions:\n{ir}"
    );
}

#[test]
fn custom_into_string_is_used_by_templates_and_variadic_println() {
    let (_cg, ir) = compile(
        r#"
struct Dog { name String }
impl Dog Into<String> {
    into_string(self) String { return self.name; }
}
fn main() {
    let dog = Dog { name = "Rex" };
    println("dog: ", dog);
    println(`again: ${dog}`);
}
"#,
    );
    assert!(
        ir.contains("into_string"),
        "expected conversion calls for Dog:\n{ir}"
    );
}

#[test]
fn generic_into_string_supports_primitive_and_string_instantiations() {
    let (_cg, ir) = compile(
        r#"
fn stringify<T Into<String>>(value T) String {
    return value.into_string();
}
fn main() {
    println(stringify(42));
    println(stringify("ready"));
}
"#,
    );
    assert!(
        ir.contains("nether_rt_i64_to_string"),
        "expected the primitive specialization to use ToString:\n{ir}"
    );
    assert!(
        ir.contains("nether_stringify"),
        "expected concrete stringify specializations:\n{ir}"
    );
}

#[test]
fn generic_struct_instantiations_have_concrete_fields_and_layouts() {
    let (_cg, ir) = compile(
        r#"
struct Boxed<T> { value T }
struct pair<T, U>(T, U);
fn unbox<T>(value Boxed<T>) T { return value.value; }
fn main() {
    let number = Boxed { value = 42 };
    let text = Boxed { value = "ready" };
    let both = pair(number, text);
    println(unbox(both.0), ": ", unbox(both.1));
}
"#,
    );
    assert!(
        ir.contains("nether_shim_drop_"),
        "Boxed<String> should receive a concrete field drop shim:\n{ir}"
    );
}
