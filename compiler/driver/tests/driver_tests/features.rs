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
fn read_text<T Read<String>>(value T) String { return value.read(); }
trait Identity<T> {
    identity(self, value T) T { return value; }
}
fn identify<T Identity<String>>(value T) String {
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
fn associated_constants_run_end_to_end() {
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!(
        "nether_associated_constants_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
trait SizedValue {
    pub const SIZE i32;
    pub const FALLBACK i32 = 7;
}
struct Packet;
impl Packet SizedValue {
    pub const SIZE i32 = 12;
}
impl Packet {
    pub const TAG i32 = 3;
}
fn size<T SizedValue>() i32 { return T.SIZE; }
fn main() {
    println(`${Packet.SIZE}`);
    println(`${Packet.FALLBACK}`);
    println(`${Packet.TAG}`);
    println(`${size<Packet>()}`);
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
    assert_eq!(String::from_utf8_lossy(&output.stdout), "12\n7\n3\n12\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn associated_types_run_end_to_end() {
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!(
        "nether_associated_types_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
trait Container { pub type Item; }
struct IntBox;
impl IntBox Container { pub type Item = i32; }
fn identity<T Container>(value T.Item) T.Item { return value; }
fn main() { println(`${identity<IntBox>(42)}`); }
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
    assert_eq!(String::from_utf8_lossy(&output.stdout), "42\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn const_generic_fixed_arrays_run_end_to_end() {
    ensure_runtime_built();
    let dir =
        std::env::temp_dir().join(format!("nether_const_generics_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
fn consume<const N usize>(values {i32, N}) usize { return N; }
fn first<const N usize>(values {i32, N}) i32 { return values[0]; }
fn main() {
    println(`${consume<3>({1, 2, 3})}`);
    println(`${first<3>({4, 5, 6})}`);
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
    assert_eq!(String::from_utf8_lossy(&output.stdout), "3\n4\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn square_bracket_and_curly_brace_array_literals_run_with_real_distinct_representations_end_to_end(
) {
    // `[1, 2, 3]` (growable, heap-backed `Array<T>`) and `{1, 2, 3}`
    // (inline, stack-allocated `{T, N}`) are two distinct literal forms
    // as of Stage 7 — this proves both compile, link, and run correctly
    // with their own real behavior: `.push()` only exists on the
    // growable form, and a fixed array's own length is part of its type.
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!(
        "nether_array_literal_kinds_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
fn sum3(values {i32, 3}) i32 {
    return values[0] + values[1] + values[2];
}
fn main() {
    let mut growable = [1, 2, 3];
    growable.push(4);
    println(`${growable.len()}`);

    let fixed = {10, 20, 30};
    println(`${sum3(fixed)}`);
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
    assert_eq!(String::from_utf8_lossy(&output.stdout), "4\n60\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn existential_and_opaque_witness_dispatch_run_end_to_end() {
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!("nether_existential_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
trait Sound { sound(self) String; }
struct Dog { name String }
impl Dog Sound { sound(self) String { return self.name; } }
fn erase(value Dog) any Sound { return value; }
fn make() some Sound { return Dog { name = "opaque" }; }
fn main() {
    let value any Sound = erase(Dog { name = "existential" });
    println(value.sound());
    println(make().sound());
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
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "existential\nopaque\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dev_dictionary_and_release_monomorphization_match_end_to_end() {
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!("nether_dictionary_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
trait Noise { noise(self) String { return "default"; } }
struct Wolf { name String }
struct Fox Noise { name String }
impl Wolf Noise { noise(self) String { return "woof"; } }
fn make_noise<T Noise>(value T) String { return value.noise(); }
fn main() {
    println(make_noise(Wolf { name = "w" }));
    println(make_noise(Fox { name = "f" }));
}
"#,
    )
    .unwrap();
    for opt_level in [0, 2] {
        let result = nether_driver::compile(
            &entry,
            &nether_driver::CompileOptions {
                opt_level,
                output_path: Some(dir.join(format!("main-o{opt_level}"))),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!result
            .diagnostics
            .iter()
            .any(nether_diagnostics::Diagnostic::is_error));
        let output = Command::new(result.executable_path.unwrap())
            .output()
            .unwrap();
        assert!(output.status.success(), "O{opt_level}: {:?}", output.status);
        assert_eq!(String::from_utf8_lossy(&output.stdout), "woof\ndefault\n");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn completed_async_tasks_and_await_run_end_to_end() {
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!("nether_async_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
async fn fetch() String { return "ready"; }
async fn answer() i32 { return 42; }
async fn main() {
    await timer.sleep(1);
    let pending = task.spawn(fetch());
    let text String = await pending;
    let value i32 = await answer();
    println(`${text}:${value}`);
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
    assert!(
        output.status.success(),
        "status: {:?}, stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "ready:42\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn await_inside_a_while_loop_resumes_the_same_await_check_on_every_iteration() {
    // Regression coverage for a real design bug caught and fixed before
    // it ever reached codegen (see `nether_mir::split_await_points`'s own
    // docs): a naive split that turned the block computing the awaited
    // task into the suspend point itself would re-run that computation —
    // here, `tick(i)` — on every single re-poll, discarding the original
    // pending task and starting a fresh one instead of re-checking it.
    // Looping over three iterations, each awaiting once, is exactly the
    // shape that would produce a wrong total (or hang) under that bug.
    ensure_runtime_built();
    let dir =
        std::env::temp_dir().join(format!("nether_async_loop_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
async fn tick(n i32) i32 { return n; }
async fn main() {
    let mut total = 0;
    let mut i = 0;
    while i < 3 {
        let v = await tick(i);
        total = total + v;
        i = i + 1;
    }
    println(`${total}`);
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
    assert!(
        output.status.success(),
        "status: {:?}, stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "3\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn await_inside_either_if_arm_dispatches_to_its_own_suspend_point() {
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!("nether_async_if_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
async fn small() i32 { return 1; }
async fn large() i32 { return 100; }
async fn main() {
    let flag = true;
    let v = if flag { await small() } else { await large() };
    println(`${v}`);
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
    assert!(
        output.status.success(),
        "status: {:?}, stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "1\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_unawaited_task_is_dropped_cleanly_without_ever_being_polled() {
    // A `Task<T>` that's constructed (via a bare call to an `async fn`)
    // but never `await`ed at all is released like any other unused
    // heap-kind `let` at scope exit — its frame's own `poll` never runs
    // even once, so this exercises `function::async_fn::build_async_start`'s
    // zero-initialization of every heap-kind local's frame field feeding
    // straight into `frame::frame_drop_shim`'s unconditional
    // release-if-non-null walk: every field must read back as "nothing to
    // release," not garbage, or this crashes instead of exiting cleanly.
    // (A genuinely *mid-flight* frame — suspended with a live heap-kind
    // local, then abandoned — needs real background scheduling to
    // construct without hanging the test on its own `await`; that's
    // Stage 4's Milestone 5, not yet implemented, so this covers the
    // "never polled at all" half of the pending-drop path today.)
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!(
        "nether_async_abandoned_task_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
async fn fetch() String { return "unused"; }
async fn main() {
    let held = "kept alive";
    if true {
        let abandoned = task.spawn(fetch());
        println(held);
    }
    println(held);
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
    assert!(
        output.status.success(),
        "status: {:?}, stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "kept alive\nkept alive\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_async_fn_returning_a_heap_struct_frees_it_exactly_once() {
    // Regression test for a real double-free caught only by actually
    // running the generated code (not by inspecting MIR/IR shape): a
    // heap-kind value fed into a fresh `Construct`/`Tuple`/`ConstructVariant`/
    // closure-capture, or moved via a plain reassignment, copies its
    // pointer into the new container without a retain (this crate's own
    // RVO convention — `nether_mir::arc`'s docs: "already start... at
    // refcount 1 for this binding"). In an ordinary function that's
    // harmless: the source local's own `alloca` is simply discarded when
    // the function returns. An async frame's fields don't disappear that
    // way — `d`'s own field inside `make_dog`'s frame stayed non-null
    // after `Dog { name = "Rex" }` moved `"Rex"`'s reference into the
    // struct's own field, so `frame::frame_drop_shim`'s walk released it
    // a *second* time on top of the struct's own eventual drop shim,
    // corrupting the allocator (see `nether_mir::build::expr`'s
    // `clear_moved_into_container` — the actual fix). Calling this twice
    // is what originally crashed; a single call alone already did too.
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!(
        "nether_async_struct_return_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
struct Dog { name String }
async fn make_dog() Dog {
    let d = Dog { name = "Rex" };
    return d;
}
async fn main() {
    let d1 = await make_dog();
    println(d1.name);
    let d2 = await make_dog();
    println(d2.name);
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
    assert!(
        output.status.success(),
        "status: {:?}, stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "Rex\nRex\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn spawned_tasks_make_real_concurrent_progress_not_program_order() {
    // Proves `task.spawn` genuinely schedules independent background
    // progress (`nether_rt_task_spawn`'s real `tokio::spawn`, replacing
    // the milestone-3-era no-op retain) rather than running eagerly at
    // its own call site: `slow_job` is spawned first but sleeps longer,
    // `fast_job` is spawned second but sleeps for only 1ms — under real
    // concurrent scheduling `fast_job` must finish before `slow_job`
    // despite being spawned later, which program order alone could never
    // produce. Also exercises the idempotent-poll fix
    // (`frame::FrameLayout::completed_state`): `task.spawn`'s own
    // background polling and `main`'s explicit `await` race to poll the
    // very same frame, and a completed poll function being re-entered a
    // second time used to redo everything from its last suspend point
    // onward, printing "done" twice.
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!(
        "nether_async_concurrent_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
async fn slow_job() {
    await timer.sleep(50);
    println("slow: done");
}
async fn fast_job() {
    await timer.sleep(1);
    println("fast: done");
}
async fn main() {
    let a = task.spawn(slow_job());
    let b = task.spawn(fast_job());
    await a;
    await b;
    println("main: done");
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
    assert!(
        output.status.success(),
        "status: {:?}, stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "fast: done\nslow: done\nmain: done\n",
        "fast_job (spawned second, sleeps 1ms) must finish before slow_job \
         (spawned first, sleeps 50ms) under real concurrent scheduling — \
         and each line must appear exactly once, not twice"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_spawned_task_progresses_in_the_background_even_if_never_awaited() {
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!(
        "nether_async_fire_and_forget_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
async fn background() {
    await timer.sleep(5);
    println("background done");
}
async fn main() {
    let handle = task.spawn(background());
    await timer.sleep(50);
    println("main done");
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
    assert!(
        output.status.success(),
        "status: {:?}, stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    // `handle` is never `await`ed — `background`'s own side effect is
    // only observable if Tokio's own scheduler drives it independently.
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "background done\nmain done\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_generic_async_fn_instantiated_at_two_types_runs_independently() {
    // Sanity check, not a targeted bug fix (per Stage 4's own roadmap
    // scoping): `frame::FrameLayout` is computed once per already-
    // monomorphized `MirFunction`, keyed by the same `MonoFnId` every
    // other per-instantiation codegen table already uses, so a generic
    // `async fn` instantiated at two different concrete types should
    // naturally get two independent frame layouts/start/poll functions
    // with no special-casing anywhere in the async codegen path. Spawns
    // both instantiations concurrently (one heap-kind — a struct — one
    // not) and awaits an actual suspend point in each, so a frame-layout
    // mixup between instantiations (wrong field offsets/sizes) would
    // show up as wrong output or a crash rather than passing by accident.
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!(
        "nether_async_generic_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
struct Box { value i32 }
async fn wrap<T>(x T) T {
    let held = x;
    await timer.sleep(1);
    return held;
}
async fn main() {
    let b = Box { value = 99 };
    let t1 = task.spawn(wrap(b));
    let t2 = task.spawn(wrap("concurrent"));
    let r1 = await t1;
    let r2 = await t2;
    println(`${r1.value}:${r2}`);
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
    assert!(
        output.status.success(),
        "status: {:?}, stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "99:concurrent\n");
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

/// Real, fixed memory-safety bug — `nether_codegen::FnCodegen`'s helpers
/// for a scratch value that's only known to be needed partway through
/// building a block (a discarded `()` statement value, an aggregate call
/// result, a struct/tuple/variant/array-literal construction slot) used
/// to call `self.m.alloca(...)` directly at the *current* (non-entry)
/// block. LLVM only treats an `alloca` in a function's entry block as a
/// fixed-offset stack slot; anywhere else it must lower to a genuine
/// *dynamic* stack adjustment, never freed before the function returns.
/// On this project's own AArch64 target, that lowering emits a spill
/// store to the (unmoved) current stack pointer even for a zero-sized
/// `{}` — which landed exactly on the bottom word of the function's fixed
/// frame, silently clobbering whatever local happened to live there. Here
/// that was a cached `&mut self` receiver address, cached once to reuse
/// across two `counter.increment()` calls: the second call read back
/// garbage instead of `counter`'s real address, so its mutation was lost
/// (or, with a different stack layout, this crashes outright) — found via
/// `otool -tv`, not by inspecting LLVM IR (already unoptimized and
/// correct on paper; the miscompile is specific to AArch64 instruction
/// selection for a non-entry `alloca`). Fixed by hoisting every such
/// scratch slot into the entry block (`nether_llvm::ModuleCx::entry_alloca`,
/// `FnCodegen::entry_alloca`/`unit_slot`).
#[test]
fn mutating_a_heap_struct_twice_then_calling_an_unrelated_function_no_longer_loses_a_mutation() {
    ensure_runtime_built();
    let dir = std::env::temp_dir().join(format!(
        "nether_stale_field_read_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.nr");
    std::fs::write(
        &entry,
        r#"
struct Counter {
    value i32
}

impl Counter {
    increment(mut self) {
        self.value = self.value + 1;
    }
}

fn double(x i32) i32 {
    return x * 2;
}

fn main() {
    let mut counter = Counter { value = 1 };
    counter.increment();
    counter.increment();
    println(`${counter.value}`);
    let seven = double(7);
    println(`${seven}`);
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
    // Before the fix: "2\n14\n" — the second `increment()` mutated
    // clobbered memory instead of `counter`.
    assert_eq!(String::from_utf8_lossy(&output.stdout), "3\n14\n");
    let _ = std::fs::remove_dir_all(&dir);
}
