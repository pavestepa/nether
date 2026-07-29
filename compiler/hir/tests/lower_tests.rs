use nether_ast::Symbol;
use nether_diagnostics::SourceMap;
use nether_hir::{lower, HirExprKind, HirModule, HirStmtKind};

fn lower_source(source: &str) -> HirModule {
    let mut map = SourceMap::new();
    let file = map.add_file("test.nr", source);
    let (module, parse_diags) = nether_parser::parse_module(source, file);
    assert!(parse_diags.is_empty(), "unexpected parse diagnostics: {parse_diags:?}");
    let (resolved, resolve_diags) = nether_resolver::resolve(&module);
    assert!(resolve_diags.is_empty(), "unexpected resolve diagnostics: {resolve_diags:?}");
    let (tables, check_diags) = nether_typecheck::check(&module, &resolved);
    assert!(check_diags.is_empty(), "unexpected typecheck diagnostics: {check_diags:?}");
    lower(&module, &resolved, tables)
}

fn fn_body<'a>(hir: &'a HirModule, name: &str) -> &'a nether_hir::HirExpr {
    let id = *hir.fn_by_name.get(&Symbol::new(name)).unwrap_or_else(|| panic!("no standalone fn named {name}"));
    &hir.get(id).body
}

/// Digs into a `Block`'s statements/tail to find the first expression
/// matching `pred`, searching one level of nested blocks (enough for
/// these tests' small fixtures without needing a full generic visitor).
fn find_expr<'a>(expr: &'a nether_hir::HirExpr, pred: &dyn Fn(&nether_hir::HirExprKind) -> bool) -> Option<&'a nether_hir::HirExpr> {
    if pred(&expr.kind) {
        return Some(expr);
    }
    match &expr.kind {
        HirExprKind::Block(stmts, tail) => {
            for stmt in stmts {
                let inner = match &stmt.kind {
                    HirStmtKind::Let { value, .. } => value,
                    HirStmtKind::Expr(e) => e,
                };
                if let Some(found) = find_expr(inner, pred) {
                    return Some(found);
                }
            }
            tail.as_ref().and_then(|t| find_expr(t, pred))
        }
        HirExprKind::If { cond, then_branch, else_branch } => find_expr(cond, pred)
            .or_else(|| find_expr(then_branch, pred))
            .or_else(|| else_branch.as_ref().and_then(|e| find_expr(e, pred))),
        HirExprKind::While { cond, body } => find_expr(cond, pred).or_else(|| find_expr(body, pred)),
        HirExprKind::Match { scrutinee, arms } => {
            find_expr(scrutinee, pred).or_else(|| arms.iter().find_map(|a| find_expr(&a.body, pred)))
        }
        HirExprKind::Call { callee, args } => find_expr(callee, pred).or_else(|| args.iter().find_map(|a| find_expr(a, pred))),
        HirExprKind::CallStatic { args, .. } | HirExprKind::CallBuiltin { args, .. } => args.iter().find_map(|a| find_expr(a, pred)),
        HirExprKind::Binary { lhs, rhs, .. } => find_expr(lhs, pred).or_else(|| find_expr(rhs, pred)),
        HirExprKind::Assign { target, value } => find_expr(target, pred).or_else(|| find_expr(value, pred)),
        _ => None,
    }
}

#[test]
fn canonical_spec_example_lowers_without_a_forin_node_ever_existing() {
    // (There is no ForIn variant in HirExprKind at all — this test mainly
    // proves the whole canonical example lowers successfully end-to-end.)
    let hir = lower_source(
        r#"
use lang.Lang;

fn main() {
    let a = Lang.new("Bobby");
    a.set_name("Husky");
    println(a.into_string());
}

type Lang {
    name: String
}

impl Lang {
    new(name: String): Lang {
        Lang { name }
    }

    set_name(mut self, new_name: String) {
        self.name = new_name;
    }
}

impl Lang: Into<String> {
    into_string(self): String {
        `name: ${self.name}`
    }
}

interface Sound {
    sound(): String {
        "..."
    }
}

impl Lang: Sound {
    sound(): String {
        "Woof! Ruff!"
    }
}
"#,
    );
    assert!(hir.fn_by_name.contains_key(&Symbol::new("main")));
    assert!(!hir.fns.is_empty());
}

#[test]
fn standalone_function_call_becomes_call_static() {
    let hir = lower_source("fn helper(x: i32): i32 { x }\nfn main() { let y = helper(1); }");
    let helper_id = *hir.fn_by_name.get(&Symbol::new("helper")).unwrap();
    let main_body = fn_body(&hir, "main");
    let found = find_expr(main_body, &|k| matches!(k, HirExprKind::CallStatic { fn_id, .. } if *fn_id == helper_id));
    assert!(found.is_some(), "expected a CallStatic to `helper`");
}

#[test]
fn static_method_call_becomes_call_static_with_no_receiver() {
    let hir = lower_source(
        r#"
type Dog { name: String }
impl Dog {
    new(name: String): Dog { Dog { name } }
}
fn main() {
    let d = Dog.new("Rex");
}
"#,
    );
    let dog_id = hir.signatures.type_shapes.keys().next().copied().unwrap();
    let new_id = *hir.methods.get(&(dog_id, Symbol::new("new"))).unwrap();
    let main_body = fn_body(&hir, "main");
    let found = find_expr(main_body, &|k| matches!(k, HirExprKind::CallStatic { fn_id, args } if *fn_id == new_id && args.len() == 1));
    assert!(found.is_some(), "expected a 1-arg CallStatic to Dog.new (the string arg, no receiver)");
}

#[test]
fn instance_method_call_prepends_receiver_as_first_argument() {
    let hir = lower_source(
        r#"
type Dog { name: String }
impl Dog {
    greet(self, other: String): String { self.name }
}
fn main() {
    let d = Dog { name: "Rex" };
    let g = d.greet("hi");
}
"#,
    );
    let dog_id = hir.signatures.type_shapes.keys().next().copied().unwrap();
    let greet_id = *hir.methods.get(&(dog_id, Symbol::new("greet"))).unwrap();
    let main_body = fn_body(&hir, "main");
    // 2 args: the receiver `d` plus the explicit `"hi"` string argument.
    let found = find_expr(main_body, &|k| matches!(k, HirExprKind::CallStatic { fn_id, args } if *fn_id == greet_id && args.len() == 2));
    assert!(found.is_some(), "expected CallStatic with receiver prepended to args");
}

#[test]
fn struct_literal_and_tuple_struct_construction_unify_to_construct() {
    let hir = lower_source(
        r#"
type Dog { name: String, age: i32 }
type Point(i32, i32);
fn main() {
    let d = Dog { age: 3, name: "Rex" };
    let p = Point(1, 2);
}
"#,
    );
    let main_body = fn_body(&hir, "main");

    // Fields must be reordered into declaration order (name, age) even
    // though the source wrote `age` first.
    let dog_construct = find_expr(main_body, &|k| matches!(k, HirExprKind::Construct { fields, .. } if fields.len() == 2)).unwrap();
    if let HirExprKind::Construct { fields, .. } = &dog_construct.kind {
        assert!(matches!(fields[0].kind, HirExprKind::Literal(nether_ast::Literal::Str(_))), "field 0 should be `name`");
        assert!(matches!(fields[1].kind, HirExprKind::Literal(nether_ast::Literal::Int(3))), "field 1 should be `age`");
    }

    let point_construct = find_expr(main_body, &|k| {
        matches!(k, HirExprKind::Construct { fields, .. } if fields.len() == 2 && matches!(fields[0].kind, HirExprKind::Literal(nether_ast::Literal::Int(1))))
    });
    assert!(point_construct.is_some(), "expected Point(1, 2) to lower to a Construct node too");
}

#[test]
fn enum_variant_construction_becomes_construct_variant() {
    let hir = lower_source(
        r#"
enum Shape { Circle(i32), Empty }
fn main() {
    let s = Shape.Circle(4);
}
"#,
    );
    let main_body = fn_body(&hir, "main");
    let found = find_expr(main_body, &|k| matches!(k, HirExprKind::ConstructVariant { variant: 0, payload, .. } if payload.len() == 1));
    assert!(found.is_some());
}

#[test]
fn reading_a_weak_field_desugars_to_a_weak_upgrade_builtin_call() {
    let hir = lower_source(
        r#"
type Child { name: String }
type Parent { kid: weak Child }
fn describe(p: Parent): String {
    match p.kid {
        Some(c) => c.name,
        None => "none",
    }
}
fn main() {}
"#,
    );
    let describe = fn_body(&hir, "describe");
    let upgrade = find_expr(describe, &|k| matches!(k, HirExprKind::CallBuiltin { name, .. } if name.as_str() == "__weak_upgrade"));
    assert!(upgrade.is_some(), "expected the weak field read to desugar into a __weak_upgrade call");
    let HirExprKind::CallBuiltin { args, .. } = &upgrade.unwrap().kind else { unreachable!() };
    assert_eq!(args.len(), 1);
    assert!(matches!(args[0].kind, HirExprKind::Field { .. }), "the upgrade call's own argument should be the raw field read");
}

#[test]
fn assigning_into_a_weak_field_does_not_desugar_the_target() {
    let hir = lower_source(
        r#"
type Child { name: String }
type Parent { kid: weak Child }
fn set_kid(p: Parent, c: Child) {
    p.kid = c;
}
fn main() {}
"#,
    );
    let set_kid = fn_body(&hir, "set_kid");
    let assign = find_expr(set_kid, &|k| matches!(k, HirExprKind::Assign { .. })).expect("expected the field assignment");
    let HirExprKind::Assign { target, .. } = &assign.kind else { unreachable!() };
    assert!(matches!(target.kind, HirExprKind::Field { .. }), "the assignment target must stay a plain field place, not a __weak_upgrade call");
}

#[test]
fn string_template_desugars_to_concat_with_tostring_wrapping_non_strings() {
    let hir = lower_source(r#"fn main() { let s = `n = ${1 + 2}!`; }"#);
    let main_body = fn_body(&hir, "main");
    let concat = find_expr(main_body, &|k| matches!(k, HirExprKind::Concat(_))).unwrap();
    let HirExprKind::Concat(pieces) = &concat.kind else { unreachable!() };
    assert_eq!(pieces.len(), 3);
    assert!(matches!(pieces[0].kind, HirExprKind::Literal(nether_ast::Literal::Str(_))));
    assert!(matches!(pieces[1].kind, HirExprKind::ToString(_)), "the i32 expression should be wrapped in ToString");
    assert!(matches!(pieces[2].kind, HirExprKind::Literal(nether_ast::Literal::Str(_))));
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
            assert!(matches!(cond.kind, HirExprKind::Binary { op: nether_ast::BinaryOp::Lt, .. }));
        }
    }
}

#[test]
fn inherited_interface_default_gets_a_distinct_hir_fn_per_owner() {
    let hir = lower_source(
        r#"
interface Sound {
    sound(): String {
        "..."
    }
}
type Dog { name: String }
type Cat { name: String }
impl Dog: Sound {
}
impl Cat: Sound {
}
"#,
    );
    // Confirm two distinct HirFnIds exist for `sound`, one per owner.
    let sound_ids: Vec<_> = hir.methods.iter().filter(|((_, name), _)| name.as_str() == "sound").map(|(_, id)| *id).collect();
    assert_eq!(sound_ids.len(), 2, "Dog and Cat should each get their own lowered `sound` HirFunction");
    assert_ne!(sound_ids[0], sound_ids[1]);
}

#[test]
fn generic_bound_method_call_becomes_call_generic_method() {
    let hir = lower_source(
        r#"
interface Sound {
    sound(): String {
        "..."
    }
}
fn make_noise<T: Sound>(x: T) {
    println(x.sound());
}
"#,
    );
    let body = fn_body(&hir, "make_noise");
    let found = find_expr(body, &|k| matches!(k, HirExprKind::CallGenericMethod { method_name, .. } if method_name.as_str() == "sound"));
    assert!(found.is_some(), "expected a CallGenericMethod for `x.sound()`");
}
