use super::*;

#[test]
fn unsafe_impl_send_and_sync_parse() {
    let module = parse_ok(
        r#"
struct Handle { ptr *mut i32 }
unsafe impl Handle Send { }
unsafe impl Handle Sync { }
"#,
    );
    let Item::Impl(send_impl) = &module.items[1] else {
        panic!("expected impl block")
    };
    assert!(send_impl.is_unsafe);
    assert_eq!(send_impl.target.name.as_str(), "Handle");
    let Item::Impl(sync_impl) = &module.items[2] else {
        panic!("expected impl block")
    };
    assert!(sync_impl.is_unsafe);
}

#[test]
fn ordinary_impl_is_not_unsafe() {
    let module = parse_ok(
        r#"
struct Dog { name String }
impl Dog {
    bark(self) String { return self.name; }
}
"#,
    );
    let Item::Impl(block) = &module.items[1] else {
        panic!("expected impl block")
    };
    assert!(!block.is_unsafe);
}

#[test]
fn unsafe_not_followed_by_fn_or_impl_reports_diagnostic() {
    let (_, diags) = parse_with_diagnostics("unsafe struct Foo;\n");
    assert!(diags
        .iter()
        .any(|d| d.message.contains("must be followed by `fn`, or `unsafe` by `impl`")));
}

#[test]
fn thread_spawn_and_join_parse() {
    let module = parse_ok(
        r#"
fn main() {
    let handle = thread.spawn(move () => { println("hi"); });
    handle.join();
}
"#,
    );
    let Item::Fn(main) = &module.items[0] else {
        panic!("expected function")
    };
    let body = main.body.as_ref().unwrap();
    let Stmt::Let(handle) = &body.stmts[0] else {
        panic!("expected let")
    };
    let ExprKind::Call { callee, args, .. } = &handle.value.kind else {
        panic!("expected call")
    };
    let ExprKind::Path(path) = &callee.kind else {
        panic!("expected path callee")
    };
    let names: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["thread", "spawn"]);
    assert!(matches!(
        args[0].kind,
        ExprKind::Closure {
            move_capture: true,
            ..
        }
    ));
    // `handle.join()` — a bare local followed by one trailing call
    // segment — parses as a two-segment `Path` `Call`, not
    // `ExprKind::MethodCall`: `path_expr_from_ident` greedily folds any
    // `ident.ident` chain into one `Path` before `parse_postfix` ever
    // sees the `.`; disambiguating "namespace call" (`thread.spawn`)
    // from "local value method call" (`handle.join`) happens later, in
    // `nether_typecheck::check::call::check_value_path` (`bare_local_of`'s
    // own doc comment notes the same thing from the typecheck side).
    let Stmt::Expr(join_call) = &body.stmts[1] else {
        panic!("expected expr statement")
    };
    let ExprKind::Call { callee, args, .. } = &join_call.kind else {
        panic!("expected call")
    };
    let ExprKind::Path(path) = &callee.kind else {
        panic!("expected path callee")
    };
    let names: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["handle", "join"]);
    assert!(args.is_empty());
}
