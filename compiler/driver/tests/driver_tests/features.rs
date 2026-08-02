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
