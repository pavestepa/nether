use std::process::Command;

use nether_driver::CompileOptions;

/// Ensures `runtime/*`'s static libraries exist before this test's own
/// `nether_driver::check` call tries to link against them. End-to-end
/// tests must not silently stop after object emission.
fn ensure_runtime_built() {
    let status = Command::new("cargo")
        .args(["build", "--release", "-p", "nether-rt-arc", "-p", "nether-rt-string", "-p", "nether-rt-array", "-p", "nether-rt-io"])
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

fn main() {
    let a = Dog.new("Bobby");
    a.set_name("Husky");
    println(a.into_string());
    println(a.name);

    let nums = [1, 2, 3];
    println(`len: ${nums.len()}`);
}
"#,
    )
    .unwrap();

    let result = nether_driver::check(&path).expect("check should succeed");
    assert!(!result.diagnostics.iter().any(nether_diagnostics::Diagnostic::is_error), "unexpected diagnostics: {:?}", result.diagnostics);
    let exe = result.executable_path.expect("expected a linked executable now that runtime/* is built");

    let output = Command::new(&exe).output().unwrap_or_else(|e| panic!("failed to run {}: {e}", exe.display()));
    assert!(output.status.success(), "program exited with {}", output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout, "name: Husky\nHusky\nlen: 3\n");

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
        !result.diagnostics.iter().any(nether_diagnostics::Diagnostic::is_error),
        "unexpected diagnostics: {:?}",
        result.diagnostics
    );
    let exe = result.executable_path.expect("expected a linked multi-file executable");
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
    let output =
        Command::new(result.executable_path.unwrap()).output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "alpha\nbeta\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bundled_option_and_result_stdlib_runs_end_to_end() {
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!("nether_stdlib_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
use stdlib.option.option_map;
use stdlib.option.option_unwrap_or;
use stdlib.result.result_map;
use stdlib.result.result_unwrap_or;

fn main() {
    let mapped = option_map(Option.Some(4), (x: i32) => { x + 1 });
    println(`${option_unwrap_or(mapped, 0)}`);
    let result: Result<i32, String> = result_map(Result.Ok(6), (x: i32) => { x + 1 });
    println(`${result_unwrap_or(result, 0)}`);
}
"#,
    )
    .unwrap();
    let result = nether_driver::check(&entry).unwrap();
    assert!(
        !result.diagnostics.iter().any(nether_diagnostics::Diagnostic::is_error),
        "unexpected diagnostics: {:?}",
        result.diagnostics
    );
    let output = Command::new(result.executable_path.unwrap()).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "5\n7\n");
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

type Child { name: String }
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
impl Child: Identity<String> {}
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
    let holder = Holder { child: Child { name: "old" } };
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
        !result.diagnostics.iter().any(nether_diagnostics::Diagnostic::is_error),
        "unexpected diagnostics: {:?}",
        result.diagnostics
    );
    let output = Command::new(result.executable_path.unwrap()).output().unwrap();
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
    assert!(result.diagnostics.iter().any(nether_diagnostics::Diagnostic::is_error));
    let rendered = result
        .diagnostics
        .iter()
        .map(|diagnostic| nether_diagnostics::render(diagnostic, &result.source_map))
        .collect::<String>();
    assert!(rendered.contains("broken.nr:2:"), "{rendered}");
    assert!(rendered.contains("expected `bool`, found `i32`"), "{rendered}");
    assert!(result.object_path.is_none(), "front-end errors must stop before codegen");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn compile_options_control_output_optimization_and_linking() {
    let dir = std::env::temp_dir().join(format!(
        "nether_options_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nt");
    let output = dir.join("custom-binary");
    std::fs::write(&entry, "fn main() { println(\"ok\"); }\n")
        .unwrap();
    let options = CompileOptions {
        target_triple: nether_llvm::Codegen::host_triple(),
        opt_level: 2,
        output_path: Some(output),
        link: false,
    };
    let result = nether_driver::compile(&entry, &options).unwrap();
    assert!(
        result
            .diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.is_error())
    );
    let object = result.object_path.expect("expected object output");
    assert_eq!(object, dir.join("custom-binary.o"));
    assert!(object.exists());
    assert!(result.executable_path.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn every_checked_in_example_compiles_to_an_object() {
    let examples =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/some");
    let dir = std::env::temp_dir().join(format!(
        "nether_examples_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let mut paths = std::fs::read_dir(&examples)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "nt"))
        .collect::<Vec<_>>();
    paths.sort();
    assert_eq!(paths.len(), 4, "expected the four user examples");
    for path in paths {
        let output = dir.join(path.file_stem().unwrap());
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
