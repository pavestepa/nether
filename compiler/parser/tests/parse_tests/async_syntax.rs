use super::*;

#[test]
fn async_functions_methods_and_prefix_await_parse() {
    let module = parse_ok(
        r#"
async fn fetch() i32 { return 7; }
struct Worker;
impl Worker {
    async run(self) i32 { return await fetch(); }
}
async fn main() { let value = await fetch(); }
"#,
    );
    let Item::Fn(fetch) = &module.items[0] else {
        panic!("expected function")
    };
    assert!(fetch.is_async);
    let Item::Impl(block) = &module.items[2] else {
        panic!("expected impl")
    };
    assert!(block.methods[0].is_async);
    let Item::Fn(main) = &module.items[3] else {
        panic!("expected function")
    };
    let Stmt::Let(value) = &main.body.as_ref().unwrap().stmts[0] else {
        panic!("expected let")
    };
    assert!(matches!(value.value.kind, ExprKind::Await(_)));
}
