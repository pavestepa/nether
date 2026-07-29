use nether_ast::Symbol;
use nether_monomorphization::{monomorphize, MonoExpr, MonoExprKind, MonoFnId, MonoModule};

fn mono_source(source: &str) -> MonoModule {
    let mut map = nether_diagnostics::SourceMap::new();
    let file = map.add_file("test.nr", source);
    let (module, parse_diags) = nether_parser::parse_module(source, file);
    assert!(parse_diags.is_empty(), "unexpected parse diagnostics: {parse_diags:?}");
    let (resolved, resolve_diags) = nether_resolver::resolve(&module);
    assert!(resolve_diags.is_empty(), "unexpected resolve diagnostics: {resolve_diags:?}");
    let (tables, check_diags) = nether_typecheck::check(&module, &resolved);
    assert!(check_diags.is_empty(), "unexpected typecheck diagnostics: {check_diags:?}");
    let hir = nether_hir::lower(&module, &resolved, tables);
    let main_id = *hir.fn_by_name.get(&Symbol::new("main")).expect("no `main` in test source");
    monomorphize(&hir, main_id)
}

fn find_expr<'a>(expr: &'a MonoExpr, pred: &dyn Fn(&MonoExprKind) -> bool) -> Option<&'a MonoExpr> {
    if pred(&expr.kind) {
        return Some(expr);
    }
    match &expr.kind {
        MonoExprKind::Block(stmts, tail) => {
            for stmt in stmts {
                let inner = match &stmt.kind {
                    nether_monomorphization::MonoStmtKind::Let { value, .. } => value,
                    nether_monomorphization::MonoStmtKind::Expr(e) => e,
                };
                if let Some(found) = find_expr(inner, pred) {
                    return Some(found);
                }
            }
            tail.as_ref().and_then(|t| find_expr(t, pred))
        }
        MonoExprKind::If { cond, then_branch, else_branch } => {
            find_expr(cond, pred).or_else(|| find_expr(then_branch, pred)).or_else(|| else_branch.as_ref().and_then(|e| find_expr(e, pred)))
        }
        MonoExprKind::While { cond, body } => find_expr(cond, pred).or_else(|| find_expr(body, pred)),
        MonoExprKind::Match { scrutinee, arms } => find_expr(scrutinee, pred).or_else(|| arms.iter().find_map(|a| find_expr(&a.body, pred))),
        MonoExprKind::Call { callee, args } => find_expr(callee, pred).or_else(|| args.iter().find_map(|a| find_expr(a, pred))),
        MonoExprKind::CallStatic { args, .. } | MonoExprKind::CallBuiltin { args, .. } => args.iter().find_map(|a| find_expr(a, pred)),
        MonoExprKind::Binary { lhs, rhs, .. } => find_expr(lhs, pred).or_else(|| find_expr(rhs, pred)),
        MonoExprKind::Assign { target, value } => find_expr(target, pred).or_else(|| find_expr(value, pred)),
        _ => None,
    }
}

fn all_calls(module: &MonoModule, from: MonoFnId, out: &mut Vec<MonoFnId>) {
    let f = module.get(from);
    fn walk(expr: &MonoExpr, out: &mut Vec<MonoFnId>) {
        if let MonoExprKind::CallStatic { fn_id, args } = &expr.kind {
            out.push(*fn_id);
            for a in args {
                walk(a, out);
            }
            return;
        }
        match &expr.kind {
            MonoExprKind::Block(stmts, tail) => {
                for s in stmts {
                    match &s.kind {
                        nether_monomorphization::MonoStmtKind::Let { value, .. } => walk(value, out),
                        nether_monomorphization::MonoStmtKind::Expr(e) => walk(e, out),
                    }
                }
                if let Some(t) = tail {
                    walk(t, out);
                }
            }
            MonoExprKind::If { cond, then_branch, else_branch } => {
                walk(cond, out);
                walk(then_branch, out);
                if let Some(e) = else_branch {
                    walk(e, out);
                }
            }
            MonoExprKind::While { cond, body } => {
                walk(cond, out);
                walk(body, out);
            }
            MonoExprKind::Match { scrutinee, arms } => {
                walk(scrutinee, out);
                for a in arms {
                    walk(&a.body, out);
                }
            }
            MonoExprKind::CallBuiltin { args, .. } | MonoExprKind::Call { args, .. } => {
                for a in args {
                    walk(a, out);
                }
            }
            MonoExprKind::Binary { lhs, rhs, .. } => {
                walk(lhs, out);
                walk(rhs, out);
            }
            _ => {}
        }
    }
    walk(&f.body, out);
}

#[test]
fn canonical_spec_example_monomorphizes_from_main() {
    let mono = mono_source(
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
"#,
    );
    assert_eq!(mono.get(mono.entry).name.as_str(), "main");
    // new/set_name/into_string are all reachable from main.
    assert!(mono.functions.iter().any(|f| f.name.as_str() == "new"));
    assert!(mono.functions.iter().any(|f| f.name.as_str() == "set_name"));
    assert!(mono.functions.iter().any(|f| f.name.as_str() == "into_string"));
}

#[test]
fn unreachable_function_is_excluded_from_mono_module() {
    let mono = mono_source(
        r#"
fn used(): i32 { 1 }
fn unused(): i32 { 2 }
fn main() { let x = used(); }
"#,
    );
    assert!(mono.functions.iter().any(|f| f.name.as_str() == "used"));
    assert!(!mono.functions.iter().any(|f| f.name.as_str() == "unused"), "dead code should not be monomorphized");
}

#[test]
fn shared_non_generic_call_is_instantiated_only_once() {
    let mono = mono_source(
        r#"
fn helper(): i32 { 1 }
fn main() {
    let a = helper();
    let b = helper();
}
"#,
    );
    let helper_count = mono.functions.iter().filter(|f| f.name.as_str() == "helper").count();
    assert_eq!(helper_count, 1, "one shared non-generic function should collapse to a single MonoFunction");
}

#[test]
fn generic_function_gets_a_distinct_instantiation_per_concrete_type() {
    let mono = mono_source(
        r#"
fn identity<T>(x: T): T { x }
fn main() {
    let a = identity(1);
    let b = identity(true);
}
"#,
    );
    let identity_count = mono.functions.iter().filter(|f| f.name.as_str() == "identity").count();
    assert_eq!(identity_count, 2, "identity::<i32> and identity::<bool> should be two distinct MonoFunctions");
}

#[test]
fn generic_function_called_twice_with_same_type_is_instantiated_once() {
    let mono = mono_source(
        r#"
fn identity<T>(x: T): T { x }
fn main() {
    let a = identity(1);
    let b = identity(2);
}
"#,
    );
    let identity_count = mono.functions.iter().filter(|f| f.name.as_str() == "identity").count();
    assert_eq!(identity_count, 1, "two calls with the same concrete type argument should memoize to one instantiation");
}

#[test]
fn no_generic_types_remain_after_monomorphization() {
    let mono = mono_source(
        r#"
fn identity<T>(x: T): T { x }
fn main() {
    let a = identity(1);
}
"#,
    );
    for f in &mono.functions {
        assert!(!matches!(f.ret, nether_typecheck::Type::Generic(_)), "no MonoFunction's return type should remain generic");
        for p in &f.params {
            assert!(!matches!(p.ty, nether_typecheck::Type::Generic(_)), "no MonoFunction's param type should remain generic");
        }
    }
}

#[test]
fn generic_method_call_resolves_to_the_concrete_impls_call_static() {
    let mono = mono_source(
        r#"
interface Sound {
    sound(self): String {
        "..."
    }
}
type Dog { name: String }
type Cat { name: String }
impl Dog: Sound {
    sound(self): String { "Woof" }
}
impl Cat: Sound {
}
fn make_noise<T: Sound>(x: T): String {
    x.sound()
}
fn main() {
    let d = Dog { name: "Rex" };
    let c = Cat { name: "Tom" };
    let a = make_noise(d);
    let b = make_noise(c);
}
"#,
    );
    // Two distinct instantiations of make_noise, one per receiver type.
    let make_noise_ids: Vec<MonoFnId> = mono.functions.iter().filter(|f| f.name.as_str() == "make_noise").map(|f| f.id).collect();
    assert_eq!(make_noise_ids.len(), 2);

    // Each instantiation's body must contain a CallStatic to the correct
    // owner's `sound` — not a leftover CallGenericMethod (there is no such
    // variant in MonoExprKind at all, so this also proves elimination
    // structurally).
    let dog_sound_id = mono.functions.iter().find(|f| f.name.as_str() == "sound" && f.owner.is_some()).map(|f| f.id);
    assert!(dog_sound_id.is_some());

    let mut all_target_ids = Vec::new();
    for id in &make_noise_ids {
        let f = mono.get(*id);
        let found = find_expr(&f.body, &|k| matches!(k, MonoExprKind::CallStatic { .. }));
        assert!(found.is_some(), "expected make_noise's body to contain a resolved CallStatic");
        if let Some(MonoExpr { kind: MonoExprKind::CallStatic { fn_id, .. }, .. }) = found {
            all_target_ids.push(*fn_id);
        }
    }
    assert_eq!(all_target_ids.len(), 2);
    assert_ne!(all_target_ids[0], all_target_ids[1], "Dog's and Cat's `sound` are different concrete methods");

    let mut reached = Vec::new();
    all_calls(&mono, mono.entry, &mut reached);
    for id in &make_noise_ids {
        assert!(reached.contains(id), "main should reach both make_noise instantiations");
    }
}
