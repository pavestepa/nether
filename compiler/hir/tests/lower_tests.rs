use nether_ast::Symbol;
use nether_diagnostics::SourceMap;
use nether_hir::{lower, HirExprKind, HirModule, HirStmtKind};

#[path = "lower_tests/weak_and_interfaces.rs"]
mod weak_and_interfaces;

fn lower_source(source: &str) -> HirModule {
    let mut map = SourceMap::new();
    let file = map.add_file("test.nr", source);
    let (module, parse_diags) = nether_parser::parse_module(source, file);
    assert!(
        parse_diags.is_empty(),
        "unexpected parse diagnostics: {parse_diags:?}"
    );
    let (resolved, resolve_diags) = nether_resolver::resolve(&module);
    assert!(
        resolve_diags.is_empty(),
        "unexpected resolve diagnostics: {resolve_diags:?}"
    );
    let (tables, check_diags) = nether_typecheck::check(&module, &resolved);
    assert!(
        check_diags.is_empty(),
        "unexpected typecheck diagnostics: {check_diags:?}"
    );
    lower(&module, &resolved, tables)
}

fn fn_body<'a>(hir: &'a HirModule, name: &str) -> &'a nether_hir::HirExpr {
    let id = *hir
        .fn_by_name
        .get(&Symbol::new(name))
        .unwrap_or_else(|| panic!("no standalone fn named {name}"));
    &hir.get(id).body
}

/// Digs into a `Block`'s statements/tail to find the first expression
/// matching `pred`, searching one level of nested blocks (enough for
/// these tests' small fixtures without needing a full generic visitor).
fn find_expr<'a>(
    expr: &'a nether_hir::HirExpr,
    pred: &dyn Fn(&nether_hir::HirExprKind) -> bool,
) -> Option<&'a nether_hir::HirExpr> {
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
        HirExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => find_expr(cond, pred)
            .or_else(|| find_expr(then_branch, pred))
            .or_else(|| else_branch.as_ref().and_then(|e| find_expr(e, pred))),
        HirExprKind::While { cond, body } => {
            find_expr(cond, pred).or_else(|| find_expr(body, pred))
        }
        HirExprKind::Match { scrutinee, arms } => find_expr(scrutinee, pred)
            .or_else(|| arms.iter().find_map(|a| find_expr(&a.body, pred))),
        HirExprKind::Call { callee, args } => {
            find_expr(callee, pred).or_else(|| args.iter().find_map(|a| find_expr(a, pred)))
        }
        HirExprKind::CallStatic { args, .. } | HirExprKind::CallBuiltin { args, .. } => {
            args.iter().find_map(|a| find_expr(a, pred))
        }
        HirExprKind::Binary { lhs, rhs, .. } => {
            find_expr(lhs, pred).or_else(|| find_expr(rhs, pred))
        }
        HirExprKind::Assign { target, value } => {
            find_expr(target, pred).or_else(|| find_expr(value, pred))
        }
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
    let mut a = Lang.new("Bobby");
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
    let found = find_expr(
        main_body,
        &|k| matches!(k, HirExprKind::CallStatic { fn_id, .. } if *fn_id == helper_id),
    );
    assert!(found.is_some(), "expected a CallStatic to `helper`");
}

#[test]
fn explicit_generic_arguments_are_kept_on_hir_calls() {
    let hir = lower_source(
        r#"
fn opaque<T>(value: i32): i32 { value }
fn main() {
    opaque<String>(2);
}
"#,
    );
    let body = fn_body(&hir, "main");
    let found = find_expr(body, &|kind| {
        matches!(
            kind,
            HirExprKind::CallStatic { generic_args, .. }
                if generic_args == &[nether_typecheck::Type::String]
        )
    });
    assert!(
        found.is_some(),
        "expected String specialization on the call"
    );
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
    let new_id = hir
        .methods
        .get(&(dog_id, Symbol::new("new")))
        .unwrap()
        .generic
        .unwrap();
    let main_body = fn_body(&hir, "main");
    let found = find_expr(
        main_body,
        &|k| matches!(k, HirExprKind::CallStatic { fn_id, args, .. } if *fn_id == new_id && args.len() == 1),
    );
    assert!(
        found.is_some(),
        "expected a 1-arg CallStatic to Dog.new (the string arg, no receiver)"
    );
}

#[test]
fn instance_method_call_becomes_call_method_with_receiver_kept_separate() {
    // Instance method calls are resolved lazily by `monomorphization`
    // (`HirExprKind::CallMethod`), not baked to a fixed `HirFnId` here —
    // see that node's own docs for why (concrete specialization). The
    // receiver stays its own field rather than being prepended to `args`.
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
    assert!(hir.methods.contains_key(&(dog_id, Symbol::new("greet"))));
    let main_body = fn_body(&hir, "main");
    let found = find_expr(
        main_body,
        &|k| matches!(k, HirExprKind::CallMethod { method_name, args, .. } if method_name.as_str() == "greet" && args.len() == 1),
    );
    assert!(
        found.is_some(),
        "expected a CallMethod for `d.greet(\"hi\")` with the receiver kept out of `args`"
    );
}

#[test]
fn user_defined_array_method_becomes_call_method_but_builtins_stay_call_array_method() {
    // `Array<T>` is an ordinary generic owner now (`array_owner`, threaded
    // from `hir::lower` through to `monomorphization`) — a non-builtin
    // method name on an `Array` receiver goes through the same
    // `CallMethod` path any other type's instance method does, while
    // `len`/`push`/`pop` keep the dedicated `CallArrayMethod` fast path.
    let hir = lower_source(
        r#"
type Array<T>;
impl<T> Array<T> {
    first(self): T { self[0] }
}
fn main() {
    let a = [1, 2, 3];
    let f = a.first();
    let n = a.len();
}
"#,
    );
    let array_id = hir
        .array_owner
        .expect("Array is declared in this test's own source");
    assert!(hir.methods.contains_key(&(array_id, Symbol::new("first"))));
    let main_body = fn_body(&hir, "main");
    let found_call_method = find_expr(
        main_body,
        &|k| matches!(k, HirExprKind::CallMethod { method_name, args, .. } if method_name.as_str() == "first" && args.is_empty()),
    );
    assert!(
        found_call_method.is_some(),
        "expected a CallMethod for `a.first()`"
    );
    let found_call_array_method = find_expr(
        main_body,
        &|k| matches!(k, HirExprKind::CallArrayMethod { method, args, .. } if method.as_str() == "len" && args.is_empty()),
    );
    assert!(
        found_call_array_method.is_some(),
        "expected `a.len()` to keep lowering to the builtin CallArrayMethod"
    );
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
    let dog_construct = find_expr(
        main_body,
        &|k| matches!(k, HirExprKind::Construct { fields, .. } if fields.len() == 2),
    )
    .unwrap();
    if let HirExprKind::Construct { fields, .. } = &dog_construct.kind {
        assert!(
            matches!(
                fields[0].kind,
                HirExprKind::Literal(nether_ast::Literal::Str(_))
            ),
            "field 0 should be `name`"
        );
        assert!(
            matches!(
                fields[1].kind,
                HirExprKind::Literal(nether_ast::Literal::Int(3))
            ),
            "field 1 should be `age`"
        );
    }

    let point_construct = find_expr(
        main_body,
        &|k| matches!(k, HirExprKind::Construct { fields, .. } if fields.len() == 2 && matches!(fields[0].kind, HirExprKind::Literal(nether_ast::Literal::Int(1)))),
    );
    assert!(
        point_construct.is_some(),
        "expected Point(1, 2) to lower to a Construct node too"
    );
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
    let found = find_expr(
        main_body,
        &|k| matches!(k, HirExprKind::ConstructVariant { variant: 0, payload, .. } if payload.len() == 1),
    );
    assert!(found.is_some());
}

#[test]
fn reading_a_weak_field_desugars_to_a_weak_upgrade_builtin_call() {
    // `Option` is an ordinary prelude `enum` now (`stdlib/option.nt`), not
    // a compiler builtin — `lower_source` resolves a bare parsed `Module`
    // directly, no driver, no prelude loading, so this declares its own
    // stand-in with the same shape (`nether_hir` only ever sees a
    // resolved `DefId`, so this exercises the same desugaring as the real
    // bundled `Option`).
    let hir = lower_source(
        r#"
enum Option<T> {
    Some(T),
    None,
}
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
    let upgrade = find_expr(
        describe,
        &|k| matches!(k, HirExprKind::CallBuiltin { name, .. } if name.as_str() == "__weak_upgrade"),
    );
    assert!(
        upgrade.is_some(),
        "expected the weak field read to desugar into a __weak_upgrade call"
    );
    let HirExprKind::CallBuiltin { args, .. } = &upgrade.unwrap().kind else {
        unreachable!()
    };
    assert_eq!(args.len(), 1);
    assert!(
        matches!(args[0].kind, HirExprKind::Field { .. }),
        "the upgrade call's own argument should be the raw field read"
    );
}
