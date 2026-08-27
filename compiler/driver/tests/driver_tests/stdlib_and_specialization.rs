use super::*;

#[test]
fn mod_std_alias_loads_the_bundled_stdlib_root_without_conflict() {
    // `mod std;` mounts the same bundled `stdlib/` tree the always-on
    // prelude already loads (`load_module_graph`), under the name `std` —
    // `resolve_use_module`'s `std`/`stdlib` fallback branches both point
    // at the same `bundled_stdlib_root()`. Everything currently bundled
    // (`stdlib/option.nr`/`result.nt`) is `impl`-only with no top-level
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
    let dir =
        std::env::temp_dir().join(format!("nether_specialization_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
default impl<T> Option<T> {
    describe(self) String {
        return "generic";
    }
}

impl Option<i32> {
    describe(self) String {
        return "int";
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
fn sum(items ...i32) i32 {
    let mut total = 0;
    for item in items {
        total = total + item;
    }
    return total;
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
    assert_eq!(String::from_utf8_lossy(&output.stdout), "0\n1\n6\n15\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Real, fixed memory-safety bug — a `for`/`match` arm bound by a bare
/// catch-all `HirPattern::Binding` (every `for`-loop desugaring's own
/// single arm, among other shapes) double-released its heap-typed
/// scrutinee: `nether_mir::build::pattern::lower_match` tracks the
/// scrutinee's one retained credit in *two* places — the match's own
/// `match_scope` (so an arm that never names it, e.g. a literal-pattern
/// arm, still eventually releases it) *and*, redundantly, the matching
/// arm's own bindings scope, since `lower_pattern_bindings` hands a
/// top-level `Binding` pattern the scrutinee's own local unchanged rather
/// than a fresh copy. Both scopes' own release logic then fire for the
/// *same* local, one retain paying for two releases — freeing a still-
/// referenced string one call too early and corrupting whatever
/// allocation reused that freed memory on a later call. Reproduces with a
/// plain, non-generic, non-variadic `Array<String>` parameter whose loop
/// body reassigns a `String` accumulator each iteration
/// (`result = \`${result}${part}\`;`) — unrelated to generics,
/// specialization, or variadics. Fixed by skipping the redundant
/// arm-scope entry whenever a binding's own local is literally the
/// scrutinee's (see `lower_match`'s own comment at the fix site).
#[test]
fn string_concatenation_by_reassignment_inside_a_loop_over_an_array_parameter_no_longer_corrupts_memory(
) {
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!("nether_known_bug_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
fn joined(parts Array<String>) String {
    let mut result = "";
    for part in parts {
        result = `${result}${part}`;
    }
    return result;
}

fn main() {
    println(joined([]));
    println(joined(["a"]));
    println(joined(["a", "b", "c"]));
    println(joined(["a", "b", "c"]));
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
    assert!(output.status.success());
    // Before the fix, the *second* repeated `["a", "b", "c"]` call already
    // printed garbled bytes (a corrupted allocation reused from the
    // previous call's prematurely freed string) or crashed outright.
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "\na\nabc\nabc\nabc\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
