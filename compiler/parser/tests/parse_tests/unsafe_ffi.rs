use super::*;

#[test]
fn unsafe_fn_and_unsafe_block_parse() {
    let module = parse_ok(
        r#"
unsafe fn danger() i32 { return 1; }
struct Worker;
impl Worker {
    unsafe run(self) i32 { return 2; }
}
fn main() {
    let value = unsafe { danger() };
}
"#,
    );
    let Item::Fn(danger) = &module.items[0] else {
        panic!("expected function")
    };
    assert!(danger.is_unsafe);
    let Item::Impl(block) = &module.items[2] else {
        panic!("expected impl")
    };
    assert!(block.methods[0].is_unsafe);
    let Item::Fn(main) = &module.items[3] else {
        panic!("expected function")
    };
    let Stmt::Let(value) = &main.body.as_ref().unwrap().stmts[0] else {
        panic!("expected let")
    };
    assert!(matches!(value.value.kind, ExprKind::Unsafe(_)));
}

#[test]
fn raw_pointer_types_parse_in_signatures_and_owned_form() {
    let module = parse_ok(
        r#"
fn takes(p *const i32, q *mut i32) { }
fn owned(p :*const i32) { }
"#,
    );
    let Item::Fn(takes) = &module.items[0] else {
        panic!("expected function")
    };
    assert!(matches!(
        takes.params[0].ty,
        nether_ast::TypeExpr::RawConstPtr(_, _)
    ));
    assert!(matches!(
        takes.params[1].ty,
        nether_ast::TypeExpr::RawMutPtr(_, _)
    ));
    let Item::Fn(owned) = &module.items[1] else {
        panic!("expected function")
    };
    assert!(matches!(
        owned.params[0].ty,
        nether_ast::TypeExpr::Unique(_, _)
    ));
}

#[test]
fn raw_borrow_and_raw_deref_parse() {
    let module = parse_ok(
        r#"
fn main() {
    let mut x = 1;
    let p = &raw mut x;
    let q = &raw const x;
    let value = unsafe { *p };
}
"#,
    );
    let Item::Fn(main) = &module.items[0] else {
        panic!("expected function")
    };
    let stmts = &main.body.as_ref().unwrap().stmts;
    let Stmt::Let(p) = &stmts[1] else {
        panic!("expected let")
    };
    assert!(matches!(
        p.value.kind,
        ExprKind::RawBorrow { mutable: true, .. }
    ));
    let Stmt::Let(q) = &stmts[2] else {
        panic!("expected let")
    };
    assert!(matches!(
        q.value.kind,
        ExprKind::RawBorrow {
            mutable: false,
            ..
        }
    ));
    let Stmt::Let(value) = &stmts[3] else {
        panic!("expected let")
    };
    let ExprKind::Unsafe(inner) = &value.value.kind else {
        panic!("expected unsafe block")
    };
    let deref_expr = inner.tail.as_ref().expect("expected a tail expression");
    assert!(matches!(deref_expr.kind, ExprKind::RawDeref(_)));
}

#[test]
fn extern_block_with_link_attribute_parses() {
    let module = parse_ok(
        r#"
#[link(name = "m")]
extern "C" {
    fn sqrt(x f64) f64;
    pub fn abs(n i32) i32;
}
"#,
    );
    let Item::Extern(block) = &module.items[0] else {
        panic!("expected extern block")
    };
    assert_eq!(block.abi, "C");
    assert_eq!(block.link.as_deref(), Some("m"));
    assert_eq!(block.functions.len(), 2);
    assert!(block.functions[0].body.is_none());
    assert_eq!(
        block.functions[1].visibility,
        nether_ast::Visibility::Public
    );
}
