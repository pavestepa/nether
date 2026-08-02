use std::process::Command;

use nether_driver::CompileOptions;

/// Ensures `runtime/*`'s static libraries exist before this test's own
/// `nether_driver::check` call tries to link against them. End-to-end
/// tests must not silently stop after object emission.
fn ensure_runtime_built() {
    let status = Command::new("cargo")
        .args([
            "build",
            "--release",
            "-p",
            "nether-rt-arc",
            "-p",
            "nether-rt-string",
            "-p",
            "nether-rt-array",
            "-p",
            "nether-rt-io",
        ])
        .status()
        .expect("failed to invoke cargo to build the runtime crates");
    assert!(status.success(), "building runtime/* failed");
}

#[test]
fn a_full_program_compiles_links_and_runs_with_the_expected_output() {
    ensure_runtime_built();

    let dir = std::env::temp_dir().join("nether_driver_test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("e2e.nr");
    std::fs::write(
        &path,
        r#"
type Dog {
    name: String
}

impl Dog {
    new(name: String): Dog {
        Dog { name }
    }

    set_name(mut self, new_name: String) {
        self.name = new_name;
    }
}

impl Dog: Into<String> {
    into_string(self): String {
        `name: ${self.name}`
    }
}

fn explicit<T>(value: i32): i32 {
    value
}

fn main() {
    let mut a = Dog.new("Bobby");
    a.set_name("Husky");
    println(a.into_string());
    println(a.name);

    let nums = [1, 2, 3];
    println(`len: ${nums.len()}`);
    println(`${explicit<u32>(7)}`);
}
"#,
    )
    .unwrap();

    let result = nether_driver::check(&path).expect("check should succeed");
    assert!(
        !result
            .diagnostics
            .iter()
            .any(nether_diagnostics::Diagnostic::is_error),
        "unexpected diagnostics: {:?}",
        result.diagnostics
    );
    let exe = result
        .executable_path
        .expect("expected a linked executable now that runtime/* is built");

    let output = Command::new(&exe)
        .output()
        .unwrap_or_else(|e| panic!("failed to run {}: {e}", exe.display()));
    assert!(
        output.status.success(),
        "program exited with {}",
        output.status
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout, "name: Husky\nHusky\nlen: 3\n7\n");

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(result.object_path.unwrap());
    let _ = std::fs::remove_file(&exe);
}

#[test]
fn local_modules_compile_link_and_run() {
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!("nether_module_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let module_path = dir.join("lang.nr");
    let entry_path = dir.join("main.nr");
    std::fs::write(
        &module_path,
        r#"
type Lang { name: String }
impl Lang {
    new(name: String): Lang { Lang { name } }
    greeting(self): String { `hello ${self.name}` }
}
"#,
    )
    .unwrap();
    std::fs::write(
        &entry_path,
        r#"
mod lang;
use lang.Lang;
fn main() {
    let lang = Lang.new("Nether");
    println(lang.greeting());
}
"#,
    )
    .unwrap();

    let result = nether_driver::check(&entry_path).expect("multi-file compilation should run");
    assert!(
        !result
            .diagnostics
            .iter()
            .any(nether_diagnostics::Diagnostic::is_error),
        "unexpected diagnostics: {:?}",
        result.diagnostics
    );
    let exe = result
        .executable_path
        .expect("expected a linked multi-file executable");
    let output = Command::new(&exe).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "hello Nether\n");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn modules_keep_private_top_level_names_separate() {
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!(
        "nether_module_namespace_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("alpha.nt"),
        r#"
type Item { text: String }
fn helper(): String { Item { text: "alpha" }.text }
fn from_alpha(): String { helper() }
"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("beta.nt"),
        r#"
type Item { value: String }
fn helper(): String { Item { value: "beta" }.value }
fn from_beta(): String { helper() }
"#,
    )
    .unwrap();
    let entry = dir.join("main.nt");
    std::fs::write(
        &entry,
        r#"
mod alpha;
mod beta;
use alpha.from_alpha;
use beta.from_beta;
fn main() {
    println(from_alpha());
    println(from_beta());
}
"#,
    )
    .unwrap();
    let result = nether_driver::check(&entry).unwrap();
    assert!(
        result
            .diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.is_error()),
        "unexpected diagnostics: {:?}",
        result.diagnostics
    );
    let output = Command::new(result.executable_path.unwrap())
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "alpha\nbeta\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rust_style_mod_files_relative_roots_and_cross_module_impls_run() {
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!("nether_rust_modules_test_{}", std::process::id()));
    std::fs::create_dir_all(dir.join("labels")).unwrap();
    let entry = dir.join("main.nt");
    std::fs::write(
        &entry,
        r#"
mod impl_into_i32;
mod labels;
use self.labels.label;

type Cat { name: String }
impl Cat {
    new(name: String): Cat { Cat { name } }
}
impl Cat: Into<String> {
    into_string(self): String { self.name }
}

fn main() {
    let cat = Cat.new("Kikki");
    println(label(cat));
    println(`${cat.into_i32()}`);
}
"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("impl_into_i32.nt"),
        r#"
use super.Cat;
impl Cat {
    into_i32(self): i32 { 1 }
}
"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("labels/mod.nt"),
        r#"
use crate.Cat;
fn label(cat: Cat): String { cat.name }
"#,
    )
    .unwrap();

    let result = nether_driver::check(&entry).unwrap();
    assert!(
        result
            .diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.is_error()),
        "unexpected diagnostics: {:?}",
        result.diagnostics
    );
    let output = Command::new(result.executable_path.unwrap())
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "Kikki\n1\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_mod_file_is_a_source_anchored_diagnostic() {
    let dir = std::env::temp_dir().join(format!("nether_missing_mod_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nt");
    std::fs::write(&entry, "mod absent;\nfn main() {}\n").unwrap();
    let result = nether_driver::check(&entry).unwrap();
    let diagnostic = result
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.message.contains("child module `absent`"))
        .expect("missing module diagnostic");
    assert!(diagnostic
        .labels
        .iter()
        .any(|label| label.span.file == result.module.file));
    assert!(result.object_path.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bundled_option_and_result_stdlib_runs_end_to_end() {
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!("nether_stdlib_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    // `Option.unwrap_or`/`.map` and `Result.unwrap_or`/`.map` both come
    // from the bundled prelude (`stdlib/mod.nt` always loads
    // `stdlib/option.nt`/`stdlib/result.nt`, which declare them with the
    // explicit `impl<T> Option<T> { ... }`/`impl<T, E> Result<T, E> {
    // ... }` form — the only way to bind a type parameter for a builtin
    // owner with no local declaration) — no `use`/`impl` of their own
    // methods needed here at all.
    std::fs::write(
        &entry,
        r#"
fn main() {
    let mapped = Option.Some(4).map((x: i32) => { x + 1 });
    println(`${mapped.unwrap_or(0)}`);
    let ok: Result<i32, String> = Result.Ok(6);
    println(`${ok.map((x: i32) => { x + 1 }).unwrap_or(0)}`);
}
"#,
    )
    .unwrap();
    let result = nether_driver::check(&entry).unwrap();
    assert!(
        !result
            .diagnostics
            .iter()
            .any(nether_diagnostics::Diagnostic::is_error),
        "unexpected diagnostics: {:?}",
        result.diagnostics
    );
    let output = Command::new(result.executable_path.unwrap())
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "5\n7\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bare_enum_variant_names_from_use_module_enum_variant_run_end_to_end() {
    // `stdlib/mod.nt` writes `use option.Option.Some;` / `use
    // option.Option.None;` / `use result.Result.Ok;` / `use
    // result.Result.Error;` — a 3-segment `use module.Enum.Variant;`
    // path reaching one level into an enum for one of its variants,
    // promoted into the bundled prelude so every file gets bare
    // `Some`/`None`/`Ok`/`Error` in *expression* position (not just in
    // `match` patterns, which already worked via `find_unique_variant`
    // with no `use` needed at all).
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!("nether_bare_variant_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
fn describe(x: Option<i32>): String {
    match x {
        Some(v) => `got ${v}`,
        None => "nothing",
    }
}

fn main() {
    let a = Some(4);
    let b: Option<i32> = None;
    println(describe(a));
    println(describe(b));

    let r: Result<i32, String> = Ok(7);
    match r {
        Ok(v) => println(`ok ${v}`),
        Error(e) => println(`err ${e}`),
    }
}
"#,
    )
    .unwrap();
    let result = nether_driver::check(&entry).unwrap();
    assert!(
        !result
            .diagnostics
            .iter()
            .any(nether_diagnostics::Diagnostic::is_error),
        "unexpected diagnostics: {:?}",
        result.diagnostics
    );
    let output = Command::new(result.executable_path.unwrap())
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "got 4\nnothing\nok 7\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn use_enum_variant_path_rejects_an_unknown_module_member() {
    let dir = std::env::temp_dir().join(format!("nether_variant_bad_member_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("helper.nt"), "enum Color { Red, Green }\n").unwrap();
    let entry = dir.join("main.nt");
    std::fs::write(
        &entry,
        "mod helper;\nuse self.helper.Bogus.Thing;\nfn main() {}\n",
    )
    .unwrap();
    let result = nether_driver::check(&entry).unwrap();
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.message.contains("module does not define `Bogus`")),
        "{:?}",
        result.diagnostics
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn use_enum_variant_path_rejects_an_unknown_variant() {
    let dir = std::env::temp_dir().join(format!("nether_variant_bad_variant_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("helper.nt"), "enum Color { Red, Green }\n").unwrap();
    let entry = dir.join("main.nt");
    std::fs::write(
        &entry,
        "mod helper;\nuse self.helper.Color.Purple;\nfn main() {}\n",
    )
    .unwrap();
    let result = nether_driver::check(&entry).unwrap();
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.message.contains("enum `Color` has no member `Purple`")),
        "{:?}",
        result.diagnostics
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn use_enum_variant_path_rejects_a_non_enum_target() {
    let dir = std::env::temp_dir().join(format!("nether_variant_not_enum_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("helper.nt"), "type Shape;\n").unwrap();
    let entry = dir.join("main.nt");
    std::fs::write(
        &entry,
        "mod helper;\nuse self.helper.Shape.Thing;\nfn main() {}\n",
    )
    .unwrap();
    let result = nether_driver::check(&entry).unwrap();
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.message.contains("`Shape` is not an enum")),
        "{:?}",
        result.diagnostics
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mod_std_alias_loads_the_bundled_stdlib_root_without_conflict() {
    // `mod std;` mounts the same bundled `stdlib/` tree the always-on
    // prelude already loads (`load_module_graph`), under the name `std` —
    // `resolve_use_module`'s `std`/`stdlib` fallback branches both point
    // at the same `bundled_stdlib_root()`. Everything currently bundled
    // (`stdlib/option.nt`/`result.nt`) is `impl`-only with no top-level
    // name to `use`, so this exercises the *file-loading* half of the
    // alias specifically: an explicit `mod std;` must not conflict with
    // (double-register, duplicate-diagnostic) the same file the prelude
    // already loaded unconditionally, and `Option`/`Result`'s bundled
    // methods must keep working with it present.
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!("nether_std_alias_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
mod std;

fn main() {
    println(`${Option.Some(4).unwrap_or(0)}`);
    let ok: Result<i32, String> = Result.Ok(6);
    println(`${ok.map((x: i32) => { x + 1 }).unwrap_or(0)}`);
}
"#,
    )
    .unwrap();
    let result = nether_driver::check(&entry).unwrap();
    assert!(
        !result
            .diagnostics
            .iter()
            .any(nether_diagnostics::Diagnostic::is_error),
        "unexpected diagnostics: {:?}",
        result.diagnostics
    );
    let output = Command::new(result.executable_path.unwrap())
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "4\n7\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn concrete_specialization_overrides_the_generic_impl() {
    // `impl Option<i32> { ... }` overrides `impl<T> Option<T> { ... }`
    // only for `T = i32`; every other `T` keeps using the generic
    // version. Monomorphization re-resolves the call after receiver-type
    // substitution rather than trusting a fixed target picked once at HIR
    // lowering time (`HirExprKind::CallMethod`) specifically so this
    // override can also apply *through* a call site inside another
    // still-generic function — not exercised here, since Nether cannot
    // currently call a generic method/function from inside another
    // still-generic one at all (`collect_generic_bindings` refuses to
    // bind one generic parameter to another still-symbolic one — a
    // separate, pre-existing gap unrelated to specialization; e.g. even
    // `fn wrap<U>(x: U): U { identity(x) }` is rejected today). This test
    // covers every call shape that gap does not block.
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!("nether_specialization_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
impl<T> Option<T> {
    describe(self): String {
        "generic"
    }
}

impl Option<i32> {
    describe(self): String {
        "int"
    }
}

fn main() {
    println(Option.Some(4).describe());
    println(Option.Some("text").describe());
    println(Option.Some(true).describe());
}
"#,
    )
    .unwrap();
    let result = nether_driver::check(&entry).unwrap();
    assert!(
        !result
            .diagnostics
            .iter()
            .any(nether_diagnostics::Diagnostic::is_error),
        "unexpected diagnostics: {:?}",
        result.diagnostics
    );
    let output = Command::new(result.executable_path.unwrap())
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "int\ngeneric\ngeneric\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn variadic_parameter_collects_trailing_arguments_into_an_array() {
    // `args: ...i32` is sugar for an ordinary `Array<i32>` parameter — the
    // call site collects zero or more trailing arguments into it
    // automatically (`nether_hir::lower::lower_variadic_aware_args`).
    // Called repeatedly with different argument counts in one program,
    // since a real (now-fixed) bug only showed up under exactly that
    // pattern — see this repo's `docs/generics.md` note on it for the
    // details of what does and doesn't reproduce it.
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!("nether_variadic_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
fn sum(items: ...i32): i32 {
    let mut total = 0;
    for item in items {
        total = total + item;
    }
    total
}

fn main() {
    println(`${sum()}`);
    println(`${sum(1)}`);
    println(`${sum(1, 2, 3)}`);
    println(`${sum(1, 2, 3, 4, 5)}`);
}
"#,
    )
    .unwrap();
    let result = nether_driver::check(&entry).unwrap();
    assert!(
        !result
            .diagnostics
            .iter()
            .any(nether_diagnostics::Diagnostic::is_error),
        "unexpected diagnostics: {:?}",
        result.diagnostics
    );
    let output = Command::new(result.executable_path.unwrap())
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "0\n1\n6\n15\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Known, unresolved, pre-existing memory-safety bug — `#[ignore]`d
/// because it reliably `SIGSEGV`s (or `SIGBUS`es) the *whole* test
/// process if run without isolation (all `#[test]` fns in this binary
/// share one process), not because it's slow. Run in isolation with
/// `cargo test -p nether-driver --test driver_tests -- --ignored
/// string_concatenation_by_reassignment_inside_a_loop_over_an_array_parameter_corrupts_memory`
/// to reproduce; do not remove `#[ignore]` until it's fixed.
///
/// Minimal repro: a function taking an `Array<String>` (or `...String`
/// — variadics are not implicated; see `docs/generics.md`), iterated with
/// `for x in items { result = \`${result}${x}\`; }`, reassigning a
/// `String` local via template-string concatenation each turn. Called
/// once from `main` with an already-concrete receiver it's fine; called
/// from inside another function taking the array as its own parameter,
/// it corrupts memory non-deterministically (sometimes wrong output,
/// sometimes a crash, depending on unrelated heap state). Predates every
/// change in this session — reproduces on plain arrays with no
/// specialization, no `impl` blocks, and no variadics involved at all.
#[test]
#[ignore]
fn string_concatenation_by_reassignment_inside_a_loop_over_an_array_parameter_corrupts_memory() {
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!("nether_known_bug_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
fn joined(parts: Array<String>): String {
    let mut result = "";
    for part in parts {
        result = `${result}${part}`;
    }
    result
}

fn main() {
    println(joined([]));
    println(joined(["a"]));
    println(joined(["a", "b", "c"]));
}
"#,
    )
    .unwrap();
    let result = nether_driver::check(&entry).unwrap();
    assert!(!result
        .diagnostics
        .iter()
        .any(nether_diagnostics::Diagnostic::is_error));
    let output = Command::new(result.executable_path.unwrap())
        .output()
        .unwrap();
    // Expected once fixed. Today this either fails this assertion with
    // garbled stdout or the whole process dies to a signal first.
    assert_eq!(String::from_utf8_lossy(&output.stdout), "\na\nabc\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn language_features_run_together_end_to_end() {
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!("nether_features_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
type point { x: i32 }
fn bump(mut value: point) { value.x = value.x + 1; }
type container { point: point }
fn bump_nested(mut value: container) {
    value.point.x = value.point.x + 1;
}
type item { value: i32 }

type Child: Identity<String> { name: String }
type Parent { child: weak Child }
type Holder { child: Child }
fn weak_name(parent: Parent): String {
    match parent.child {
        Some(child) => child.name,
        None => "gone",
    }
}
fn weak_argument(value: weak Child): String {
    match value {
        Some(child) => child.name,
        None => "gone",
    }
}

enum Color { Red, Named(String) }
fn color_name(color: Color): String {
    match color {
        Red => "red",
        Named(name) => name,
    }
}

fn identity<T>(value: T): T { value }
fn double(value: i32): i32 { value * 2 }
type Boxed<T> { value: T }
fn unbox<T>(value: Boxed<T>): T { value.value }
impl Boxed {
    get(self): T { self.value }
}
interface Read<T> {
    read(self): T;
}
impl Boxed: Read<T> {
    read(self): T { self.value }
}
fn read_text<T: Read<String>>(value: T): String { value.read() }
interface Identity<T> {
    identity(self, value: T): T { value }
}
fn identify<T: Identity<String>>(value: T): String {
    value.identity("generic-interface")
}

fn main() {
    let mut position = point { x: 1 };
    bump(mut position);
    println(`${position.x}`);
    let mut nested = container { point: point { x: 3 } };
    bump_nested(mut nested);
    println(`${nested.point.x}`);
    let mut items = [item { value: 5 }];
    items[0].value = 6;
    println(`${items[0].value}`);
    let mut tuple = (7, 8);
    tuple.1 = 9;
    println(`${tuple.1}`);

    let child = Child { name: "live" };
    let mut holder = Holder { child: Child { name: "old" } };
    holder.child.name = "changed";
    println(holder.child.name);
    let parent = Parent { child };
    println(weak_name(parent));
    let observer: weak Child = child;
    println(match observer { Some(value) => value.name, None => "gone" });
    println(weak_argument(child));
    let expired: weak Child = Child { name: "temporary" };
    println(match expired { Some(value) => value.name, None => "gone" });

    let children = [Child { name: "one" }, Child { name: "two" }];
    println(`${children.len()}`);
    println(color_name(Color.Named("blue")));
    println(`${identity(true)}`);

    let prefix = "n=";
    let render = (value: i32) => { `${prefix}${value}` };
    println(render(3));
    let operation = double;
    println(`${operation(4)}`);
    println(`${unbox(Boxed { value: 9 })}`);
    println(Boxed { value: "generic-method" }.get());
    println(read_text(Boxed { value: "generic-interface-owner" }));
    println(identify(child));
}
"#,
    )
    .unwrap();
    let result = nether_driver::check(&entry).unwrap();
    assert!(
        !result
            .diagnostics
            .iter()
            .any(nether_diagnostics::Diagnostic::is_error),
        "unexpected diagnostics: {:?}",
        result.diagnostics
    );
    let output = Command::new(result.executable_path.unwrap())
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "2\n4\n6\n9\nchanged\nlive\nlive\nlive\ngone\n2\nblue\ntrue\nn=3\n8\n9\ngeneric-method\ngeneric-interface-owner\ngeneric-interface\n",
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn invalid_program_diagnostics_remain_source_anchored() {
    let dir = std::env::temp_dir().join(format!("nether_diagnostics_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("broken.nr");
    std::fs::write(
        &entry,
        "fn main() {\n    let value: bool = 1;\n    println(value);\n}\n",
    )
    .unwrap();
    let result = nether_driver::check(&entry).unwrap();
    assert!(result
        .diagnostics
        .iter()
        .any(nether_diagnostics::Diagnostic::is_error));
    let rendered = result
        .diagnostics
        .iter()
        .map(|diagnostic| nether_diagnostics::render(diagnostic, &result.source_map))
        .collect::<String>();
    assert!(rendered.contains("broken.nr:2:"), "{rendered}");
    assert!(
        rendered.contains("expected `bool`, found `i32`"),
        "{rendered}"
    );
    assert!(
        result.object_path.is_none(),
        "front-end errors must stop before codegen"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn compile_options_control_output_optimization_and_linking() {
    let dir = std::env::temp_dir().join(format!("nether_options_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nt");
    let output = dir.join("custom-binary");
    std::fs::write(&entry, "fn main() { println(\"ok\"); }\n").unwrap();
    let options = CompileOptions {
        target_triple: nether_llvm::Codegen::host_triple(),
        opt_level: 2,
        output_path: Some(output),
        link: false,
    };
    let result = nether_driver::compile(&entry, &options).unwrap();
    assert!(result
        .diagnostics
        .iter()
        .all(|diagnostic| !diagnostic.is_error()));
    let object = result.object_path.expect("expected object output");
    assert_eq!(object, dir.join("custom-binary.o"));
    assert!(object.exists());
    assert!(result.executable_path.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn every_checked_in_example_compiles_to_an_object() {
    let examples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let dir = std::env::temp_dir().join(format!("nether_examples_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut paths = std::fs::read_dir(examples.join("some"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "nt"))
        .collect::<Vec<_>>();
    paths.extend([
        examples.join("hello_world/main.nt"),
        examples.join("shelter/main.nt"),
        examples.join("metrics/main.nt"),
        examples.join("adventure/main.nt"),
    ]);
    paths.sort();
    assert_eq!(paths.len(), 8, "expected all checked-in user entry points");
    for (index, path) in paths.into_iter().enumerate() {
        let output = dir.join(format!("example_{index}"));
        let options = CompileOptions {
            output_path: Some(output),
            link: false,
            ..CompileOptions::default()
        };
        let result = nether_driver::compile(&path, &options).unwrap();
        assert!(
            result
                .diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.is_error()),
            "{} failed: {:?}",
            path.display(),
            result.diagnostics
        );
        assert!(result.object_path.is_some());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn larger_example_projects_compile_link_and_run() {
    ensure_runtime_built();

    let examples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let dir =
        std::env::temp_dir().join(format!("nether_large_examples_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cases = [
        (
            "shelter",
            concat!(
                "shelter=North Star\n",
                "featured=Luna\n",
                "1. Luna (2)\n",
                "2. Milo (4)\n",
                "3. Nova (1)\n",
                "reserved until Saturday\n",
                "card: Echo, age 3\n",
                "no pet selected\n",
            ),
        ),
        (
            "metrics",
            concat!(
                "count=4, total=20, range=2..8\n",
                "count=4, total=60, range=6..24\n",
                "count=3, total=14, range=1..9\n",
                "counter=11\n",
                "metrics complete\n",
                "total=20, max=8\n",
            ),
        ),
        (
            "adventure",
            concat!(
                "hero Arin: hp=20, score=0\n",
                "active quest: Ancient Gate\n",
                "A wild shadow approaches Arin\n",
                "received 6 damage\n",
                "found treasure worth 12\n",
                "Arin: hp=14, score=27\n",
                "quest complete, reward=125\n",
                "The road continues\n",
            ),
        ),
    ];

    for (name, expected_stdout) in cases {
        let executable = dir.join(name);
        let options = CompileOptions {
            output_path: Some(executable.clone()),
            link: true,
            ..CompileOptions::default()
        };
        let result = nether_driver::compile(&examples.join(name).join("main.nt"), &options)
            .unwrap_or_else(|error| panic!("{name} failed to compile: {error}"));
        assert!(
            result
                .diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.is_error()),
            "{name} diagnostics: {:?}",
            result.diagnostics
        );
        let output = Command::new(&executable)
            .output()
            .unwrap_or_else(|error| panic!("failed to run {name}: {error}"));
        assert!(
            output.status.success(),
            "{name} exited with {}",
            output.status
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            expected_stdout,
            "{name} produced unexpected output"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}
