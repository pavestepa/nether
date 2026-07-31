use nether_ast::Symbol;
use nether_llvm::Codegen;

fn compile(source: &str) -> (nether_llvm::Codegen, String) {
    let cg = Codegen::new();
    let ir = {
        let mut map = nether_diagnostics::SourceMap::new();
        let file = map.add_file("test.nr", source);
        let (module, parse_diags) = nether_parser::parse_module(source, file);
        assert!(
            parse_diags.is_empty(),
            "unexpected parse diagnostics: {parse_diags:?}"
        );
        let (resolved, resolve_diags) = nether_resolver::resolve(&module);
        assert!(
            resolve_diags.is_empty(),
            "unexpected resolve diagnostics: {resolve_diags:?}"
        );
        let (tables, check_diags) = nether_typecheck::check(&module, &resolved);
        assert!(
            check_diags.is_empty(),
            "unexpected typecheck diagnostics: {check_diags:?}"
        );
        let hir = nether_hir::lower(&module, &resolved, tables);
        let main_id = *hir
            .fn_by_name
            .get(&Symbol::new("main"))
            .expect("no `main` in test source");
        let mono = nether_monomorphization::monomorphize(&hir, main_id);
        let mut functions = nether_mir::build_mir(&mono, &resolved.definitions, &hir.signatures);
        nether_mir::insert_arc(&mut functions);
        let m = nether_codegen::generate(
            &cg,
            "test",
            &functions,
            &resolved.definitions,
            &hir.signatures,
        );
        m.verify().unwrap_or_else(|e| {
            panic!(
                "module failed to verify:\n{}\n\nerror: {e}",
                m.print_to_string()
            )
        });
        m.print_to_string()
    };
    (cg, ir)
}

#[test]
fn canonical_spec_example_compiles_to_a_verified_module() {
    let (_cg, ir) = compile(
        r#"
use lang.Lang;

fn main() {
    let a = Lang.new("Bobby");
    a.set_name("Husky");
    println(a.into_string());
}

type Lang {
    name: String
}

impl Lang {
    new(name: String): Lang {
        Lang { name }
    }

    set_name(mut self, new_name: String) {
        self.name = new_name;
    }
}

impl Lang: Into<String> {
    into_string(self): String {
        `name: ${self.name}`
    }
}
"#,
    );
    assert!(
        ir.contains("define"),
        "expected at least one defined function:\n{ir}"
    );
    assert!(
        ir.contains("declare"),
        "expected declared runtime functions:\n{ir}"
    );
    assert!(
        ir.contains("nether_rt_arc_alloc"),
        "expected a heap allocation call for Lang's construction:\n{ir}"
    );
    assert!(
        ir.contains("nether_rt_arc_retain"),
        "expected at least one Retain call:\n{ir}"
    );
    assert!(
        ir.contains("nether_rt_arc_release"),
        "expected at least one Release call:\n{ir}"
    );
    assert!(
        ir.contains("nether_rt_string_concat"),
        "expected the template string to lower to a concat call:\n{ir}"
    );
}

#[test]
fn arithmetic_and_control_flow_compiles() {
    let (_cg, ir) = compile(
        r#"
fn abs(x: i32): i32 {
    if x < 0 {
        0 - x
    } else {
        x
    }
}
fn main() {
    let mut i = 0;
    while i < 5 {
        i = i + abs(0 - i);
    }
}
"#,
    );
    assert!(
        ir.contains("icmp slt"),
        "expected a signed less-than comparison:\n{ir}"
    );
    assert!(ir.contains("br i1"), "expected a conditional branch:\n{ir}");
}

#[test]
fn closure_with_stack_and_heap_captures_compiles_to_an_indirect_call() {
    let (_cg, ir) = compile(
        r#"
fn main() {
    let base = 10;
    let prefix = "answer: ";
    let format = (x: i32) => {
        `${prefix}${base + x}`
    };
    println(format(5));
}
"#,
    );
    assert!(
        ir.contains("closure_call"),
        "expected an indirect closure call:\n{ir}"
    );
    assert!(
        ir.contains("nether_rt_arc_alloc"),
        "expected an ARC-managed closure environment:\n{ir}"
    );
    assert!(
        ir.contains("nether_rt_arc_retain"),
        "expected the heap capture to be retained:\n{ir}"
    );
}

#[test]
fn named_function_value_uses_the_same_closure_call_abi() {
    let (_cg, ir) = compile(
        r#"
fn add_one(x: i32): i32 { x + 1 }
fn main() {
    let f = add_one;
    println(`${f(4)}`);
}
"#,
    );
    assert!(
        ir.contains("fn_adapter"),
        "expected a closure-ABI adapter for a named function value:\n{ir}"
    );
    assert!(
        ir.contains("closure_call"),
        "expected an indirect first-class function call:\n{ir}"
    );
}

#[test]
fn match_on_enum_with_heap_payload_compiles() {
    let (_cg, ir) = compile(
        r#"
type Dog { name: String }
enum Wrapper { Boxed(Dog), Empty }
fn unwrap(w: Wrapper): String {
    match w {
        Boxed(d) => d.name,
        Empty => "none",
    }
}
fn main() {
    let w = Wrapper.Boxed(Dog { name: "Rex" });
    let s = unwrap(w);
    println(s);
}
"#,
    );
    assert!(ir.contains("define"), "expected a defined function:\n{ir}");
}

#[test]
fn emits_a_c_abi_main_calling_nether_main() {
    let (_cg, ir) = compile("fn main() { println(\"hi\"); }");
    assert!(
        ir.contains("define i32 @main()"),
        "expected a C-ABI main entry point:\n{ir}"
    );
    assert!(
        ir.contains("call void @nether_main()") || ir.contains("call { } @nether_main()"),
        "expected main to call nether_main:\n{ir}"
    );
}

#[test]
fn constructing_a_heap_struct_with_a_heap_field_generates_a_drop_shim() {
    let (_cg, ir) = compile(
        r#"
type Dog { name: String }
fn main() {
    let d = Dog { name: "Rex" };
    println(d.name);
}
"#,
    );
    assert!(
        ir.contains("define void @nether_shim_drop_"),
        "expected a generated drop shim releasing Dog's heap field:\n{ir}"
    );
    assert!(
        ir.contains("call ptr @nether_rt_arc_alloc(i64") && ir.contains("nether_shim_drop_"),
        "expected Dog's construction to pass its drop shim to nether_rt_arc_alloc:\n{ir}"
    );
}

#[test]
fn constructing_a_heap_struct_with_no_heap_fields_passes_a_null_drop_fn() {
    let (_cg, ir) = compile(
        r#"
type Point { x: i64, y: i64 }
fn main() {
    let p = Point { x: 1, y: 2 };
    println(`${p.x}`);
}
"#,
    );
    assert!(
        !ir.contains("nether_shim_drop_"),
        "Point has no heap fields, so no drop shim should be generated:\n{ir}"
    );
    assert!(
        ir.contains("call ptr @nether_rt_arc_alloc(i64"),
        "expected Point's construction to still heap-allocate:\n{ir}"
    );
}

#[test]
fn array_of_heap_elements_passes_retain_and_release_as_elem_callbacks() {
    let (_cg, ir) = compile(
        r#"
type Dog { name: String }
fn main() {
    let dogs = [Dog { name: "Rex" }];
    println(`${dogs.len()}`);
}
"#,
    );
    assert!(
        ir.contains("call ptr @nether_rt_array_new(i64")
            && ir.contains("ptr @nether_shim_retain_")
            && ir.contains("ptr @nether_shim_drop_"),
        "expected Array<Dog>'s slot-address callbacks to load and retain/release each stored pointer:\n{ir}"
    );
}

#[test]
fn array_of_primitives_passes_null_elem_callbacks() {
    let (_cg, ir) = compile(
        r#"
fn main() {
    let nums = [1, 2, 3];
    println(`${nums.len()}`);
}
"#,
    );
    assert!(
        ir.contains("call ptr @nether_rt_array_new(") && ir.contains("i64 3, ptr null, ptr null)"),
        "expected Array<i32> to need no element callbacks:\n{ir}"
    );
}

#[test]
fn constructing_a_weak_field_calls_weak_retain_not_retain() {
    let (_cg, ir) = compile(
        r#"
type Child { name: String }
type Parent { kid: weak Child }
fn main() {
    let c = Child { name: "Rex" };
    let p = Parent { kid: c };
}
"#,
    );
    assert!(
        ir.contains("call void @nether_rt_arc_weak_retain("),
        "expected the weak field's construction to call weak_retain:\n{ir}"
    );
}

#[test]
fn assigning_into_a_weak_field_calls_weak_release_and_weak_retain() {
    let (_cg, ir) = compile(
        r#"
type Child { name: String }
type Parent { kid: weak Child }
fn set_kid(p: Parent, c: Child) {
    p.kid = c;
}
fn main() {
    let p = Parent { kid: Child { name: "Rex" } };
    let c = Child { name: "Buddy" };
    set_kid(p, c);
}
"#,
    );
    assert!(
        ir.contains("call void @nether_rt_arc_weak_retain("),
        "expected weak_retain for the field's new value:\n{ir}"
    );
    assert!(
        ir.contains("call void @nether_rt_arc_weak_release("),
        "expected weak_release for the field's old value:\n{ir}"
    );
}

#[test]
fn reading_a_weak_field_calls_weak_upgrade_and_builds_an_option() {
    let (_cg, ir) = compile(
        r#"
type Child { name: String }
type Parent { kid: weak Child }
fn describe(p: Parent): String {
    match p.kid {
        Some(c) => c.name,
        None => "none",
    }
}
fn main() {
    let c = Child { name: "Rex" };
    let p = Parent { kid: c };
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
type Child { name: String }
type Parent { kid: weak Child }
fn main() {
    let c = Child { name: "Rex" };
    let p = Parent { kid: c };
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
    let (_cg, ir) = compile(
        r#"
type Child { name: String }
fn describe(x: Option<Child>): String {
    match x {
        Some(c) => c.name,
        None => "none",
    }
}
fn main() {
    let c = Child { name: "Rex" };
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
fn identity<T>(value: T): T { value }
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
type Dog { name: String }
impl Dog: Into<String> {
    into_string(self): String { self.name }
}
fn main() {
    let dog = Dog { name: "Rex" };
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
fn stringify<T: Into<String>>(value: T): String {
    value.into_string()
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
type Boxed<T> { value: T }
type pair<T, U>(T, U);
fn unbox<T>(value: Boxed<T>): T { value.value }
fn main() {
    let number = Boxed { value: 42 };
    let text = Boxed { value: "ready" };
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
