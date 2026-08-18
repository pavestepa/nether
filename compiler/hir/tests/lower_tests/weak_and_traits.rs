use super::*;

#[test]
fn assigning_into_a_weak_field_does_not_desugar_the_target() {
    let hir = lower_source(
        r#"
struct Child { name String }
struct Parent { kid weak Child }
fn set_kid(p mut Parent, c Child) {
    p.kid = c;
}
fn main() {}
"#,
    );
    let set_kid = fn_body(&hir, "set_kid");
    let assign = find_expr(set_kid, &|k| matches!(k, HirExprKind::Assign { .. }))
        .expect("expected the field assignment");
    let HirExprKind::Assign { target, .. } = &assign.kind else {
        unreachable!()
    };
    assert!(
        matches!(target.kind, HirExprKind::Field { .. }),
        "the assignment target must stay a plain field place, not a __weak_upgrade call"
    );
}

#[test]
fn string_template_desugars_to_concat_with_tostring_wrapping_non_strings() {
    let hir = lower_source(r#"fn main() { let s = `n = ${1 + 2}!`; }"#);
    let main_body = fn_body(&hir, "main");
    let concat = find_expr(main_body, &|k| matches!(k, HirExprKind::Concat(_))).unwrap();
    let HirExprKind::Concat(pieces) = &concat.kind else {
        unreachable!()
    };
    assert_eq!(pieces.len(), 3);
    assert!(matches!(
        pieces[0].kind,
        HirExprKind::Literal(nether_ast::Literal::Str(_))
    ));
    assert!(
        matches!(pieces[1].kind, HirExprKind::ToString(_)),
        "the i32 expression should be wrapped in ToString"
    );
    assert!(matches!(
        pieces[2].kind,
        HirExprKind::Literal(nether_ast::Literal::Str(_))
    ));
}

#[test]
fn for_in_desugars_to_a_while_loop_with_no_forin_node() {
    let hir = lower_source(
        r#"
fn main() {
    for x in [1, 2, 3] {
        println(x);
    }
}
"#,
    );
    let main_body = fn_body(&hir, "main");
    let while_loop = find_expr(main_body, &|k| matches!(k, HirExprKind::While { .. }));
    assert!(while_loop.is_some(), "expected the desugared while loop");
    // The condition must be `idx < array.len()`.
    if let Some(w) = while_loop {
        if let HirExprKind::While { cond, .. } = &w.kind {
            assert!(matches!(
                cond.kind,
                HirExprKind::Binary {
                    op: nether_ast::BinaryOp::Lt,
                    ..
                }
            ));
        }
    }
}

#[test]
fn inherited_trait_default_gets_a_distinct_hir_fn_per_owner() {
    let hir = lower_source(
        r#"
trait Sound {
    sound() String {
        return "...";
    }
}
struct Dog Sound { name String }
struct Cat Sound { name String }
"#,
    );
    // Confirm two distinct HirFnIds exist for `sound`, one per owner.
    let sound_ids: Vec<_> = hir
        .methods
        .iter()
        .filter(|((_, name, _), _)| name.as_str() == "sound")
        .map(|(_, set)| set.generic.unwrap())
        .collect();
    assert_eq!(
        sound_ids.len(),
        2,
        "Dog and Cat should each get their own lowered `sound` HirFunction"
    );
    assert_ne!(sound_ids[0], sound_ids[1]);
}

#[test]
fn generic_bound_method_call_becomes_call_generic_method() {
    let hir = lower_source(
        r#"
trait Sound {
    sound() String {
        return "...";
    }
}
fn make_noise<T Sound>(x T) {
    println(x.sound());
}
"#,
    );
    let body = fn_body(&hir, "make_noise");
    let found = find_expr(
        body,
        &|k| matches!(k, HirExprKind::CallGenericMethod { method_name, .. } if method_name.as_str() == "sound"),
    );
    assert!(
        found.is_some(),
        "expected a CallGenericMethod for `x.sound()`"
    );
}

#[test]
fn static_method_called_through_value_does_not_receive_self() {
    let hir = lower_source(
        r#"
struct Animal { name String }
impl Animal {
    new(name String) Animal { return Animal { name }; }
    static_method() String { return "A"; }
}
fn main() {
    let animal = Animal.new("Cat");
    animal.static_method();
}
"#,
    );
    let static_id = hir
        .methods
        .iter()
        .find(|((_, name, _), _)| name.as_str() == "static_method")
        .and_then(|(_, set)| set.generic)
        .unwrap();
    let call = find_expr(
        fn_body(&hir, "main"),
        &|kind| matches!(kind, HirExprKind::CallStatic { fn_id, args, .. } if *fn_id == static_id && args.is_empty()),
    );
    assert!(call.is_some(), "static value call must not pass a receiver");
}

#[test]
fn inherited_parent_default_is_lowered_for_declared_trait() {
    let hir = lower_source(
        r#"
trait Parent {
    sound(self) String { return "parent"; }
}
trait Child Parent {}
struct Animal Child { name String }
fn main() {
    Animal { name = "Cat" }.sound();
}
"#,
    );
    assert!(
        hir.methods
            .keys()
            .any(|(_, name, _)| name.as_str() == "sound"),
        "the parent default should be copied to Animal"
    );
}
