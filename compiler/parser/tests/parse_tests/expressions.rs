use super::*;

#[test]
fn explicit_generic_call_arguments_are_postfix_not_comparisons() {
    let module = parse_ok(
        r#"
fn main() {
    let value = foo<u32>(2);
    let mapped = make().map<Array<u32>, String>([1], "ready");
    let comparison = 1 < 2;
}
"#,
    );
    let Item::Fn(main) = &module.items[0] else {
        panic!("expected main")
    };
    let body = main.body.as_ref().unwrap();

    let Stmt::Let(value) = &body.stmts[0] else {
        panic!("expected let")
    };
    let ExprKind::Call { generic_args, .. } = &value.value.kind else {
        panic!("expected explicit generic call")
    };
    assert_eq!(generic_args.len(), 1);

    let Stmt::Let(mapped) = &body.stmts[1] else {
        panic!("expected let")
    };
    let ExprKind::MethodCall { generic_args, .. } = &mapped.value.kind else {
        panic!("expected explicit generic method call")
    };
    assert_eq!(generic_args.len(), 2);

    let Stmt::Let(comparison) = &body.stmts[2] else {
        panic!("expected let")
    };
    assert!(matches!(
        comparison.value.kind,
        ExprKind::Binary {
            op: BinaryOp::Lt,
            ..
        }
    ));
}

#[test]
fn string_template_produces_literal_and_expr_parts() {
    let module = parse_ok(r#"fn main() { let s = `hi ${1 + 2}!`; }"#);
    let Item::Fn(f) = &module.items[0] else {
        panic!("expected FnDecl")
    };
    let body = f.body.as_ref().unwrap();
    let Stmt::Let(let_stmt) = &body.stmts[0] else {
        panic!("expected let statement")
    };
    let ExprKind::StringTemplate(parts) = &let_stmt.value.kind else {
        panic!("expected StringTemplate")
    };
    assert_eq!(parts.len(), 3);
    match &parts[1] {
        nether_ast::TemplatePart::Expr(e) => {
            assert!(matches!(
                e.kind,
                ExprKind::Binary {
                    op: BinaryOp::Add,
                    ..
                }
            ));
        }
        other => panic!("expected an Expr part, got {other:?}"),
    }
}

#[test]
fn binary_operator_precedence() {
    // `1 + 2 * 3` must parse as `1 + (2 * 3)`.
    let module = parse_ok("fn main() { let x = 1 + 2 * 3; }");
    let Item::Fn(f) = &module.items[0] else {
        panic!("expected FnDecl")
    };
    let Stmt::Let(let_stmt) = &f.body.as_ref().unwrap().stmts[0] else {
        panic!("expected let")
    };
    let ExprKind::Binary {
        op: BinaryOp::Add,
        rhs,
        ..
    } = &let_stmt.value.kind
    else {
        panic!("expected a top-level Add");
    };
    assert!(matches!(
        rhs.kind,
        ExprKind::Binary {
            op: BinaryOp::Mul,
            ..
        }
    ));
}

#[test]
fn if_condition_does_not_swallow_following_block_as_struct_literal() {
    let module = parse_ok(
        r#"
fn main() {
    let x = 1;
    if x {
        println("truthy");
    }
}
"#,
    );
    let Item::Fn(f) = &module.items[0] else {
        panic!("expected FnDecl")
    };
    let body = f.body.as_ref().unwrap();
    // last thing in the block with no trailing `;` => tail, not a Stmt.
    let if_expr = body.tail.as_ref().expect("expected a tail expression");
    assert!(matches!(if_expr.kind, ExprKind::If { .. }));
}

#[test]
fn struct_literal_parses_outside_condition_position() {
    let module = parse_ok(r#"fn main() { let d = Dog { name }; }"#);
    let Item::Fn(f) = &module.items[0] else {
        panic!("expected FnDecl")
    };
    let Stmt::Let(let_stmt) = &f.body.as_ref().unwrap().stmts[0] else {
        panic!("expected let")
    };
    assert!(matches!(let_stmt.value.kind, ExprKind::StructLit { .. }));
}

#[test]
fn tuple_literal_and_index_access() {
    let module = parse_ok("fn main() { let t = (1, \"a\"); let x = t.0; }");
    let Item::Fn(f) = &module.items[0] else {
        panic!("expected FnDecl")
    };
    let stmts = &f.body.as_ref().unwrap().stmts;
    let Stmt::Let(first) = &stmts[0] else {
        panic!("expected let")
    };
    assert!(matches!(first.value.kind, ExprKind::Tuple(ref elems) if elems.len() == 2));
    let Stmt::Let(second) = &stmts[1] else {
        panic!("expected let")
    };
    assert!(matches!(second.value.kind, ExprKind::Field { .. }));
}

#[test]
fn control_flow_constructs_parse() {
    let module = parse_ok(
        r#"
fn main() {
    let mut i = 0;
    while i < 3 {
        i = i + 1;
    }
    for x in [1, 2, 3] {
        println(x);
    }
    loop {
        break;
    }
}
"#,
    );
    let Item::Fn(f) = &module.items[0] else {
        panic!("expected FnDecl")
    };
    let body = f.body.as_ref().unwrap();
    assert!(matches!(
        body.stmts[1],
        Stmt::Expr(Expr {
            kind: ExprKind::While { .. },
            ..
        })
    ));
    assert!(matches!(
        body.stmts[2],
        Stmt::Expr(Expr {
            kind: ExprKind::ForIn { .. },
            ..
        })
    ));
    // `loop { break; }` is last with no trailing `;` => tail, not a Stmt.
    let tail = body.tail.as_ref().expect("expected a tail expression");
    assert!(matches!(tail.kind, ExprKind::Loop { .. }));
}

#[test]
fn weak_and_array_type_annotations() {
    let module = parse_ok(
        r#"
struct Node {
    parent weak Node,
    children [Node]
}
"#,
    );
    let Item::Struct(decl) = &module.items[0] else {
        panic!("expected StructDecl")
    };
    let StructDeclKind::Struct(fields) = &decl.kind else {
        panic!("expected Struct")
    };
    assert!(matches!(fields[0].ty, nether_ast::TypeExpr::Weak(_, _)));
    assert!(matches!(fields[1].ty, nether_ast::TypeExpr::Array(_, _)));
}

#[test]
fn fixed_array_literal_parses_distinctly_from_growable_array_literal() {
    let module = parse_ok(
        r#"
fn main() {
    let growable = [1, 2, 3];
    let fixed = {4, 5, 6};
    let one = {7};
}
"#,
    );
    let Item::Fn(main) = &module.items[0] else {
        panic!("expected main")
    };
    let body = main.body.as_ref().unwrap();

    let Stmt::Let(growable) = &body.stmts[0] else {
        panic!("expected let")
    };
    let ExprKind::Array(elems) = &growable.value.kind else {
        panic!("expected ExprKind::Array")
    };
    assert_eq!(elems.len(), 3);

    let Stmt::Let(fixed) = &body.stmts[1] else {
        panic!("expected let")
    };
    let ExprKind::FixedArray(elems) = &fixed.value.kind else {
        panic!("expected ExprKind::FixedArray")
    };
    assert_eq!(elems.len(), 3);

    let Stmt::Let(one) = &body.stmts[2] else {
        panic!("expected let")
    };
    assert!(matches!(one.value.kind, ExprKind::FixedArray(_)));
}

#[test]
fn malformed_item_reports_diagnostic_and_recovers() {
    let source = "struct;\nstruct Dog { name String }\n";
    let (module, diags) = parse_with_diagnostics(source);
    assert!(!diags.is_empty());
    // recovery should still find the second, well-formed struct declaration
    assert!(module
        .items
        .iter()
        .any(|item| matches!(item, Item::Struct(t) if t.name.name.as_str() == "Dog")));
}
