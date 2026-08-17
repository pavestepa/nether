use super::*;

#[test]
fn language_features_run_together_end_to_end() {
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!("nether_features_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
struct point { x i32 }
fn bump(value mut point) { value.x = value.x + 1; }
struct container { point point }
fn bump_nested(value mut container) {
    value.point.x = value.point.x + 1;
}
struct item { value i32 }

struct Child Identity<String> { name String }
struct Parent { child weak Child }
struct Holder { child Child }
fn weak_name(parent Parent) String {
    return match parent.child {
        Some(child) => child.name,
        None => "gone",
    };
}
fn weak_argument(value weak Child) String {
    return match value {
        Some(child) => child.name,
        None => "gone",
    };
}

enum Color { Red, Named(String) }
fn color_name(color Color) String {
    return match color {
        Red => "red",
        Named(name) => name,
    };
}

fn identity<T>(value T) T { return value; }
fn double(value i32) i32 { return value * 2; }
struct Boxed<T> { value T }
fn unbox<T>(value Boxed<T>) T { return value.value; }
impl Boxed {
    get(self) T { return self.value; }
}
trait Read<T> {
    read(self) T;
}
impl Boxed Read<T> {
    read(self) T { return self.value; }
}
fn read_text<T: Read<String>>(value T) String { return value.read(); }
trait Identity<T> {
    identity(self, value T) T { return value; }
}
fn identify<T: Identity<String>>(value T) String {
    return value.identity("generic-interface");
}

fn main() {
    let mut position = point { x = 1 };
    bump(mut position);
    println(`${position.x}`);
    let mut nested = container { point = point { x = 3 } };
    bump_nested(mut nested);
    println(`${nested.point.x}`);
    let mut items = [item { value = 5 }];
    items[0].value = 6;
    println(`${items[0].value}`);
    let mut tuple = (7, 8);
    tuple.1 = 9;
    println(`${tuple.1}`);

    let child = Child { name = "live" };
    let mut holder = Holder { child = Child { name = "old" } };
    holder.child.name = "changed";
    println(holder.child.name);
    let parent = Parent { child };
    println(weak_name(parent));
    let observer weak Child = child;
    println(match observer { Some(value) => value.name, None => "gone" });
    println(weak_argument(child));
    let expired weak Child = Child { name = "temporary" };
    println(match expired { Some(value) => value.name, None => "gone" });

    let children = [Child { name = "one" }, Child { name = "two" }];
    println(`${children.len()}`);
    println(color_name(Color.Named("blue")));
    println(`${identity(true)}`);

    let prefix = "n=";
    let render = (value i32) => { `${prefix}${value}` };
    println(render(3));
    let operation = double;
    println(`${operation(4)}`);
    println(`${unbox(Boxed { value = 9 })}`);
    println(Boxed { value = "generic-method" }.get());
    println(read_text(Boxed { value = "generic-interface-owner" }));
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
fn owned_domain_receiver_method_call_runs_end_to_end() {
    // Regression coverage for Stage 2's `ReceiverDomain` fix: an instance
    // method call on a genuinely owned (`:T`) receiver goes through HIR's
    // `CallMethod`, deferred to `monomorphization::resolve_generic_method_call`
    // — before this fix, `owner_def_id`/the method-set lookup there didn't
    // strip `Type::Unique`, so this would panic instead of running. Also
    // exercises an ARC-domain and owned-domain overload of the same method
    // name coexisting on one type (language-spec §8.4).
    ensure_runtime_built();
    let dir =
        std::env::temp_dir().join(format!("nether_owned_receiver_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
struct Dog { name String }
impl Dog {
    greet(self) String { return `arc:${self.name}`; }
    greet(: self) String { return `owned:${self.name}`; }
}
fn main() {
    let arc_dog = Dog { name = "Rex" };
    println(arc_dog.greet());

    let owned_dog: Dog = :Dog { name = "Buddy" };
    println(owned_dog.greet());
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
        "arc:Rex\nowned:Buddy\n",
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ref_and_mut_ref_parameters_run_end_to_end() {
    // Regression coverage for Stage 2, slice 2: `:&T`/`:&mut T` parameters
    // are `Type::Ref`/`Type::MutRef`, which `alloc_kind` already mapped to
    // `AllocKind::Stack` and `Signatures::has_managed_content` already
    // treats as not needing a drop — so `mir::insert_arc` was expected to
    // emit zero `Retain`/`Release` for them and `codegen` to pass them as
    // plain pointers, with no MIR/codegen code changes needed. This is the
    // empirical check that claim actually holds: a `:&T` parameter reading
    // a field, and a `:&mut T` parameter mutating one, both compiled,
    // linked and run for real.
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!("nether_ref_param_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
struct Dog { name String }
fn describe(d: &Dog) String { return `dog named ${d.name}`; }
fn rename(d: &mut Dog, new_name String) { d.name = new_name; }
fn main() {
    let mut dog: Dog = :Dog { name = "Rex" };
    println(describe(dog));
    rename(dog, "Buddy");
    println(describe(dog));
    println(describe(dog));
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
        "dog named Rex\ndog named Buddy\ndog named Buddy\n",
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn borrowing_method_call_through_a_mut_ref_param_runs_end_to_end() {
    // Regression coverage for Stage 2, slice 3: a `: &mut self` method
    // called through a `:&mut T` parameter — not just a field write —
    // actually mutates the caller's own value.
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!(
        "nether_ref_method_call_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
struct Dog { name String }
impl Dog {
    get_name_ref(: &self) String { return self.name; }
    rename_ref(: &mut self, new_name String) { self.name = new_name; }
}
fn describe(d: &Dog) String { return `dog named ${d.get_name_ref()}`; }
fn rename(d: &mut Dog, new_name String) { d.rename_ref(new_name); }
fn main() {
    let mut dog: Dog = :Dog { name = "Rex" };
    println(describe(dog));
    rename(dog, "Buddy");
    println(describe(dog));
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
        "dog named Rex\ndog named Buddy\n",
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn to_conversion_runs_end_to_end() {
    // Regression coverage for Stage 2, slice 4: `to(value)` lowers to
    // nothing but the argument itself, re-typed — this is the empirical
    // check that a `:T -> T` and a `t -> :t` conversion both actually
    // compile, link and run correctly, not just type-check.
    ensure_runtime_built();
    let dir =
        std::env::temp_dir().join(format!("nether_to_conversion_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
struct Dog { name String }
fn main() {
    let owned_dog: Dog = :Dog { name = "Rex" };
    let arc_dog Dog = to(owned_dog);
    println(arc_dog.name);

    let n = 42;
    let owned_n: i32 = to(n);
    println(`${n}`);
    println(`${owned_n}`);
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
    assert_eq!(String::from_utf8_lossy(&output.stdout), "Rex\n42\n42\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stored_heap_borrows_run_end_to_end() {
    ensure_runtime_built();
    let dir =
        std::env::temp_dir().join(format!("nether_stored_borrow_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
struct Dog { name String }
fn main() {
    let mut dog: Dog = :Dog { name = "Rex" };
    if true {
        let view: &mut Dog = dog;
        view.name = "Buddy";
        println(view.name);
    }
    println(dog.name);
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
    assert_eq!(String::from_utf8_lossy(&output.stdout), "Buddy\nBuddy\n");
    let _ = std::fs::remove_dir_all(&dir);
}
