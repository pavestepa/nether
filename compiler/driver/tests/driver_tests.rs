use std::process::Command;
use std::sync::OnceLock;

use nether_driver::CompileOptions;

#[path = "driver_tests/diagnostics_and_examples.rs"]
mod diagnostics_and_examples;
#[path = "driver_tests/features.rs"]
mod features;
#[path = "driver_tests/returned_references.rs"]
mod returned_references;
#[path = "driver_tests/stdlib_and_specialization.rs"]
mod stdlib_and_specialization;

/// Ensures `runtime/*`'s static libraries exist before this test's own
/// `nether_driver::check` call tries to link against them. End-to-end
/// tests must not silently stop after object emission.
fn ensure_runtime_built() {
    static RUNTIME_BUILT: OnceLock<()> = OnceLock::new();
    RUNTIME_BUILT.get_or_init(|| {
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
    });
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
struct Dog {
    name String
}

impl Dog {
    new(name String) Dog {
        return Dog { name };
    }

    set_name(mut self, new_name String) {
        self.name = new_name;
    }
}

impl Dog Into<String> {
    into_string(self) String {
        return `name: ${self.name}`;
    }
}

fn explicit<T>(value i32) i32 {
    return value;
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
struct Lang { name String }
impl Lang {
    new(name String) Lang { return Lang { name }; }
    greeting(self) String { return `hello ${self.name}`; }
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
        dir.join("alpha.nr"),
        r#"
struct Item { text String }
fn helper() String { return Item { text = "alpha" }.text; }
fn from_alpha() String { return helper(); }
"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("beta.nr"),
        r#"
struct Item { value String }
fn helper() String { return Item { value = "beta" }.value; }
fn from_beta() String { return helper(); }
"#,
    )
    .unwrap();
    let entry = dir.join("main.nr");
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
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
mod impl_into_i32;
mod labels;
use self.labels.label;

struct Cat { name String }
impl Cat {
    new(name String) Cat { return Cat { name }; }
}
impl Cat Into<String> {
    into_string(self) String { return self.name; }
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
        dir.join("impl_into_i32.nr"),
        r#"
use super.Cat;
impl Cat {
    into_i32(self) i32 { return 1; }
}
"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("labels/mod.nr"),
        r#"
use crate.Cat;
fn label(cat Cat) String { return cat.name; }
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
    let entry = dir.join("main.nr");
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
    // from the bundled prelude (`stdlib/mod.nr` always loads
    // `stdlib/option.nr`/`stdlib/result.nr`, which declare them with the
    // explicit `impl<T> Option<T> { ... }`/`impl<T, E> Result<T, E> {
    // ... }` form — the only way to bind a type parameter for a builtin
    // owner with no local declaration) — no `use`/`impl` of their own
    // methods needed here at all.
    std::fs::write(
        &entry,
        r#"
fn main() {
    let mapped = Option.Some(4).map((x i32) => { x + 1 });
    println(`${mapped.unwrap_or(0)}`);
    let ok Result<i32, String> = Result.Ok(6);
    println(`${ok.map((x i32) => { x + 1 }).unwrap_or(0)}`);
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
    // `stdlib/mod.nr` writes `use option.Option.Some;` / `use
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
fn describe(x Option<i32>) String {
    return match x {
        Some(v) => `got ${v}`,
        None => "nothing",
    };
}

fn main() {
    let a = Some(4);
    let b Option<i32> = None;
    println(describe(a));
    println(describe(b));

    let r Result<i32, String> = Ok(7);
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
    let dir =
        std::env::temp_dir().join(format!("nether_variant_bad_member_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("helper.nr"), "enum Color { Red, Green }\n").unwrap();
    let entry = dir.join("main.nr");
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
    let dir =
        std::env::temp_dir().join(format!("nether_variant_bad_variant_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("helper.nr"), "enum Color { Red, Green }\n").unwrap();
    let entry = dir.join("main.nr");
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
    std::fs::write(dir.join("helper.nr"), "struct Shape;\n").unwrap();
    let entry = dir.join("main.nr");
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
