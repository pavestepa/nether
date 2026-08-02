use nether_ast::{Expr, ExprKind, Item, Module, Path, Stmt};
use nether_diagnostics::{Diagnostic, SourceMap};
use nether_resolver::{resolve, DefKind, Resolution, ResolvedNames};

fn parse(source: &str) -> Module {
    let mut map = SourceMap::new();
    let file = map.add_file("test.nr", source);
    let (module, diags) = nether_parser::parse_module(source, file);
    assert!(diags.is_empty(), "unexpected parse diagnostics: {diags:?}");
    module
}

fn resolve_ok(source: &str) -> ResolvedNames {
    let module = parse(source);
    let (resolved, diags) = resolve(&module);
    assert!(
        diags.is_empty(),
        "unexpected resolve diagnostics: {}",
        messages(&diags)
    );
    resolved
}

fn resolve_with_diagnostics(source: &str) -> (Module, ResolvedNames, Vec<Diagnostic>) {
    let module = parse(source);
    let (resolved, diags) = resolve(&module);
    (module, resolved, diags)
}

fn messages(diags: &[Diagnostic]) -> String {
    diags
        .iter()
        .map(|d| d.message.clone())
        .collect::<Vec<_>>()
        .join("; ")
}

/// Finds the first `Path` expression node anywhere within `expr`'s tail
/// position by walking through the common wrapper shapes tests use here;
/// enough for these tests' fixtures without needing a full generic visitor.
fn first_path(expr: &Expr) -> &Path {
    match &expr.kind {
        ExprKind::Path(p) => p,
        ExprKind::MethodCall { receiver, .. } => first_path(receiver),
        ExprKind::Field { base, .. } => first_path(base),
        ExprKind::Call { callee, .. } => first_path(callee),
        other => panic!("expected an expression containing a Path, found {other:?}"),
    }
}

#[test]
fn canonical_spec_example_resolves_cleanly() {
    let source = r#"
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
"#;
    let resolved = resolve_ok(source);
    let lang_id = resolved
        .definitions
        .lookup(&"Lang".into())
        .expect("Lang should be defined");
    assert_eq!(resolved.definitions.get(lang_id).kind, DefKind::Type);
    // `new` and `set_name` (from the plain impl) plus `into_string` and
    // `sound` (from the interface impls) should all have been merged in.
    let methods = &resolved.definitions.get(lang_id).methods;
    for expected in ["new", "set_name", "into_string", "sound"] {
        assert!(
            methods.iter().any(|m| m.as_str() == expected),
            "missing method {expected}"
        );
    }
}

#[test]
fn let_binding_resolves_to_local_and_shadowing_works() {
    let module = parse(
        r#"
fn main() {
    let x = 1;
    println(x);
    let y = {
        let x = 2;
        x
    };
    println(x);
}
"#,
    );
    let (resolved, diags) = resolve(&module);
    assert!(
        diags.is_empty(),
        "unexpected diagnostics: {}",
        messages(&diags)
    );

    let Item::Fn(f) = &module.items[0] else {
        panic!("expected FnDecl")
    };
    let body = f.body.as_ref().unwrap();

    // stmts[0] = `let x = 1;` — its own local id.
    let Stmt::Let(outer_let) = &body.stmts[0] else {
        panic!("expected let")
    };
    let outer_local = *resolved
        .locals
        .get(&outer_let.id)
        .expect("outer x should be a binding site");

    // stmts[1] = `println(x);` — should resolve to the outer local.
    let Stmt::Expr(println_call) = &body.stmts[1] else {
        panic!("expected call stmt")
    };
    let ExprKind::Call { args, .. } = &println_call.kind else {
        panic!("expected Call")
    };
    let use_path = first_path(&args[0]);
    let res = resolved
        .path_res
        .get(&use_path.id)
        .expect("path should be resolved");
    assert_eq!(res.base, Resolution::Local(outer_local));

    // stmts[2] = `let y = { let x = 2; x };` — the inner `x` must resolve
    // to a *different* local than the outer one (shadowing).
    let Stmt::Let(y_let) = &body.stmts[2] else {
        panic!("expected let y")
    };
    let ExprKind::Block(inner_block) = &y_let.value.kind else {
        panic!("expected block")
    };
    let Stmt::Let(inner_let) = &inner_block.stmts[0] else {
        panic!("expected inner let")
    };
    let inner_local = *resolved
        .locals
        .get(&inner_let.id)
        .expect("inner x should be a binding site");
    assert_ne!(outer_local, inner_local);

    let inner_tail_path = first_path(inner_block.tail.as_ref().unwrap());
    let inner_res = resolved.path_res.get(&inner_tail_path.id).unwrap();
    assert_eq!(inner_res.base, Resolution::Local(inner_local));

    // stmts[3] = `println(x);` after the block — resolves back to the
    // *outer* local, since the inner one went out of scope.
    let Stmt::Expr(second_println) = &body.stmts[3] else {
        panic!("expected call stmt")
    };
    let ExprKind::Call { args, .. } = &second_println.kind else {
        panic!("expected Call")
    };
    let after_path = first_path(&args[0]);
    let after_res = resolved.path_res.get(&after_path.id).unwrap();
    assert_eq!(after_res.base, Resolution::Local(outer_local));
}

#[test]
fn self_resolves_to_a_local_bound_on_the_method() {
    let module = parse(
        r#"
type Dog { name: String }
impl Dog {
    set_name(mut self, new_name: String) {
        self.name = new_name;
    }
}
"#,
    );
    let (resolved, diags) = resolve(&module);
    assert!(
        diags.is_empty(),
        "unexpected diagnostics: {}",
        messages(&diags)
    );

    let Item::Impl(impl_block) = &module.items[1] else {
        panic!("expected impl")
    };
    let method = &impl_block.methods[0];
    let self_local = *resolved
        .locals
        .get(&method.id)
        .expect("self should be bound under the FnDecl's id");

    let Stmt::Expr(assign_stmt) = &method.body.as_ref().unwrap().stmts[0] else {
        panic!("expected assign stmt")
    };
    let ExprKind::Assign { target, .. } = &assign_stmt.kind else {
        panic!("expected assign")
    };
    let self_path = first_path(target);
    let res = resolved.path_res.get(&self_path.id).unwrap();
    assert_eq!(res.base, Resolution::Local(self_local));
    // `self.name` — only `self` is consumed by this resolution; `name` is
    // left for `typecheck` to resolve as a field access.
    assert_eq!(res.consumed, 1);
}

#[test]
fn static_method_call_resolves_to_static_member() {
    let module = parse(
        r#"
type Dog { name: String }
impl Dog {
    new(name: String): Dog {
        Dog { name }
    }
}
fn main() {
    let d = Dog.new("Rex");
}
"#,
    );
    let (resolved, diags) = resolve(&module);
    assert!(
        diags.is_empty(),
        "unexpected diagnostics: {}",
        messages(&diags)
    );

    let Item::Fn(main_fn) = &module.items[2] else {
        panic!("expected FnDecl")
    };
    let Stmt::Let(let_stmt) = &main_fn.body.as_ref().unwrap().stmts[0] else {
        panic!("expected let")
    };
    let ExprKind::Call { callee, .. } = &let_stmt.value.kind else {
        panic!("expected Call")
    };
    let ExprKind::Path(path) = &callee.kind else {
        panic!("expected Path callee")
    };
    let res = resolved.path_res.get(&path.id).unwrap();
    assert!(matches!(res.base, Resolution::StaticMember(_, _)));
    assert_eq!(res.consumed, 2);
}

#[test]
fn qualified_and_bare_enum_variant_patterns_resolve() {
    let module = parse(
        r#"
enum Color {
    Red,
    Custom(String),
}
fn describe(c: Color) {
    match c {
        Color.Red => println("red"),
        Custom(name) => println(name),
    }
}
"#,
    );
    let (resolved, diags) = resolve(&module);
    assert!(
        diags.is_empty(),
        "unexpected diagnostics: {}",
        messages(&diags)
    );

    let Item::Fn(f) = &module.items[1] else {
        panic!("expected FnDecl")
    };
    let tail = f
        .body
        .as_ref()
        .unwrap()
        .tail
        .as_ref()
        .expect("match is the tail expr");
    let ExprKind::Match { arms, .. } = &tail.kind else {
        panic!("expected Match")
    };

    let nether_ast::Pattern::Variant { path: red_path, .. } = &arms[0].pattern else {
        panic!("expected Variant")
    };
    let red_res = resolved.path_res.get(&red_path.id).unwrap();
    assert!(matches!(red_res.base, Resolution::EnumVariant(_, 0)));
    assert_eq!(red_res.consumed, 2);

    let nether_ast::Pattern::Variant {
        path: custom_path, ..
    } = &arms[1].pattern
    else {
        panic!("expected Variant")
    };
    let custom_res = resolved.path_res.get(&custom_path.id).unwrap();
    assert!(matches!(custom_res.base, Resolution::EnumVariant(_, 1)));
    assert_eq!(custom_res.consumed, 1);
}

#[test]
fn unresolved_name_reports_diagnostic() {
    let (_, _, diags) = resolve_with_diagnostics("fn main() { println(does_not_exist); }");
    assert_eq!(diags.len(), 1);
    assert!(diags[0].message.contains("does_not_exist"));
}

#[test]
fn unresolved_dotted_path_reports_the_unknown_base_and_suggests_a_local() {
    let (_, _, diags) = resolve_with_diagnostics(
        r#"
fn main() {
    let cat_kikki = 1;
    println(cat_kikky.into_i32());
}
"#,
    );
    assert_eq!(diags.len(), 1);
    assert!(
        diags[0].message.contains("cannot find `cat_kikky`"),
        "{}",
        messages(&diags)
    );
    let suggestion = diags[0]
        .suggestions
        .first()
        .expect("the nearby local should be suggested");
    assert_eq!(suggestion.replacement, "cat_kikki");
    assert!(suggestion.message.contains("did you mean `cat_kikki`"));
}

#[test]
fn duplicate_top_level_definition_reports_diagnostic() {
    let (_, _, diags) =
        resolve_with_diagnostics("type Dog { name: String }\ntype Dog { other: i32 }\n");
    assert_eq!(diags.len(), 1);
    assert!(diags[0].message.contains("defined more than once"));
}

#[test]
fn impl_for_unknown_type_reports_diagnostic() {
    let (_, _, diags) = resolve_with_diagnostics("impl Ghost { foo(self) {} }\n");
    assert_eq!(diags.len(), 1);
    assert!(diags[0].message.contains("cannot find type"));
}

#[test]
fn ambiguous_bare_variant_pattern_reports_diagnostic() {
    let (_, _, diags) = resolve_with_diagnostics(
        r#"
enum A { Same }
enum B { Same }
fn f(x: A) {
    match x {
        Same => 1,
    };
}
"#,
    );
    assert_eq!(diags.len(), 1);
    assert!(diags[0].message.contains("ambiguous"));
}

#[test]
fn generic_param_resolves_in_fn_signature() {
    let resolved = resolve_ok(
        r#"
interface Sound {
    sound(): String {
        "..."
    }
}
fn f<T: Sound>(x: T) {
    println(x);
}
"#,
    );
    let sound_id = resolved.definitions.lookup(&"Sound".into()).unwrap();
    assert_eq!(resolved.definitions.get(sound_id).kind, DefKind::Interface);
}

#[test]
fn explicit_impl_generics_bind_a_name_for_a_declaration_less_owner() {
    // The explicit `impl<T> Owner<T> { ... }` form exists specifically
    // for an owner with no local declaration to read a parameter name
    // from — `Option`/`Result` used to be the only real example (compiler
    // builtins with no `TypeDecl`/`EnumDecl`), but they're ordinary
    // prelude `enum`s now (`stdlib/option.nt`/`result.nt`), so nothing
    // reachable from a single self-contained module is truly
    // declaration-less anymore. This keeps the explicit-form mechanism
    // itself covered with a local stand-in.
    resolve_ok(
        r#"
enum MyOption<T> {
    MySome(T),
    MyNone,
}
impl<T> MyOption<T> {
    unwrap_or(self, fallback: T): T {
        match self {
            MySome(item) => item,
            MyNone => fallback,
        }
    }
}
"#,
    );
}

/// Builds a merged multi-file `Module` the way the driver would: `real.nr`
/// declares `helper`, `prelude.nr` re-exports it with `use real.helper;`,
/// and `consumer.nr` is the file under test. Returns the merged module and
/// `prelude.nr`'s own `FileId`, ready for `resolve_with_prelude`.
fn parse_prelude_fixture(consumer_source: &str) -> (Module, nether_diagnostics::FileId) {
    let mut map = SourceMap::new();

    let real_source = "fn helper(): i32 { 1 }\n";
    let real_file = map.add_file("real.nr", real_source);
    let (real_module, real_diags) = nether_parser::parse_module(real_source, real_file);
    assert!(real_diags.is_empty(), "{real_diags:?}");

    let prelude_source = "use real.helper;\n";
    let prelude_file = map.add_file("prelude.nr", prelude_source);
    let (prelude_module, prelude_diags) = nether_parser::parse_module(prelude_source, prelude_file);
    assert!(prelude_diags.is_empty(), "{prelude_diags:?}");
    let Item::Use(use_decl) = &prelude_module.items[0] else {
        panic!("expected a Use item")
    };
    let use_id = use_decl.id;

    let consumer_file = map.add_file("consumer.nr", consumer_source);
    let (consumer_module, consumer_diags) = nether_parser::parse_module(consumer_source, consumer_file);
    assert!(consumer_diags.is_empty(), "{consumer_diags:?}");

    let mut items = real_module.items;
    items.extend(prelude_module.items);
    items.extend(consumer_module.items);
    let mut imports = std::collections::HashMap::new();
    imports.insert(use_id, real_file);
    (
        Module {
            file: consumer_file,
            items,
            imports,
            variant_imports: std::collections::HashMap::new(),
        },
        prelude_file,
    )
}

#[test]
fn prelude_file_reexports_are_visible_everywhere_with_no_use() {
    let (module, prelude_file) = parse_prelude_fixture("fn use_it(): i32 { helper() }\n");
    let (_resolved, diags) = nether_resolver::resolve_with_prelude(&module, Some(prelude_file));
    assert!(diags.is_empty(), "unexpected diagnostics: {}", messages(&diags));
}

#[test]
fn prelude_names_are_invisible_without_an_explicit_prelude_file() {
    // The same fixture resolved through plain `resolve` (as if the driver
    // had found no bundled `stdlib/mod.nt`) must NOT see `helper` — proves
    // the visibility genuinely comes from the prelude promotion, not from
    // `helper` being reachable some other way.
    let (module, _prelude_file) = parse_prelude_fixture("fn use_it(): i32 { helper() }\n");
    let (_resolved, diags) = resolve(&module);
    assert!(!diags.is_empty());
}

/// Builds a merged multi-file `Module` the way the driver would for a
/// `use module.Enum.Variant;` path — `enums.nr` declares `enum Color {
/// Red, Green }`, `prelude.nr` re-exports `Red` with `use enums.Color.Red;`
/// (recorded in `variant_imports`, not `imports` — mirrors what
/// `nether_driver`'s use-loop does after its longer-prefix attempt fails),
/// and `consumer.nr` is the file under test.
fn parse_variant_prelude_fixture(consumer_source: &str) -> (Module, nether_diagnostics::FileId) {
    let mut map = SourceMap::new();

    let enums_source = "enum Color { Red, Green }\n";
    let enums_file = map.add_file("enums.nr", enums_source);
    let (enums_module, enums_diags) = nether_parser::parse_module(enums_source, enums_file);
    assert!(enums_diags.is_empty(), "{enums_diags:?}");

    let prelude_source = "use enums.Color.Red;\n";
    let prelude_file = map.add_file("prelude.nr", prelude_source);
    let (prelude_module, prelude_diags) = nether_parser::parse_module(prelude_source, prelude_file);
    assert!(prelude_diags.is_empty(), "{prelude_diags:?}");
    let Item::Use(use_decl) = &prelude_module.items[0] else {
        panic!("expected a Use item")
    };
    let use_id = use_decl.id;

    let consumer_file = map.add_file("consumer.nr", consumer_source);
    let (consumer_module, consumer_diags) = nether_parser::parse_module(consumer_source, consumer_file);
    assert!(consumer_diags.is_empty(), "{consumer_diags:?}");

    let mut items = enums_module.items;
    items.extend(prelude_module.items);
    items.extend(consumer_module.items);
    let mut variant_imports = std::collections::HashMap::new();
    variant_imports.insert(use_id, enums_file);
    (
        Module {
            file: consumer_file,
            items,
            imports: std::collections::HashMap::new(),
            variant_imports,
        },
        prelude_file,
    )
}

#[test]
fn bare_variant_name_from_use_module_enum_variant_resolves_to_enum_variant() {
    let (module, prelude_file) = parse_variant_prelude_fixture("fn use_it() { Red }\n");
    let (resolved, diags) = nether_resolver::resolve_with_prelude(&module, Some(prelude_file));
    assert!(diags.is_empty(), "unexpected diagnostics: {}", messages(&diags));
    let use_it = module
        .items
        .iter()
        .find_map(|item| match item {
            Item::Fn(f) if f.name.name.as_str() == "use_it" => Some(f),
            _ => None,
        })
        .expect("expected a fn named use_it");
    let tail = use_it
        .body
        .as_ref()
        .and_then(|b| b.tail.as_ref())
        .expect("expected a tail expression");
    let path = first_path(tail);
    let res = resolved
        .path_res
        .get(&path.id)
        .expect("expected a resolution for the bare `Red`");
    assert!(
        matches!(res.base, Resolution::EnumVariant(_, 0)),
        "expected EnumVariant(_, 0) for `Red`, got {:?}",
        res.base
    );
}

#[test]
fn a_local_declaration_shadows_a_prelude_reexport_without_conflict() {
    let (module, prelude_file) =
        parse_prelude_fixture("fn helper(): i32 { 2 }\nfn use_it(): i32 { helper() }\n");
    let (_resolved, diags) = nether_resolver::resolve_with_prelude(&module, Some(prelude_file));
    assert!(diags.is_empty(), "unexpected diagnostics: {}", messages(&diags));
}

#[test]
fn implicit_impl_form_on_a_builtin_owner_leaves_its_type_unbound() {
    // Without the explicit `<T>` form, `Option` has no declaration to read
    // a parameter name from, so `T` is an ordinary unresolved name.
    let (_, _, diags) = resolve_with_diagnostics(
        r#"
impl Option {
    unwrap_or(self, fallback: T): T {
        fallback
    }
}
"#,
    );
    assert!(!diags.is_empty());
}

#[test]
fn builtins_are_available_without_use() {
    // `Option`/`Result`/`Array` are deliberately not in this list — they're
    // ordinary `enum`/`type` declarations in the bundled prelude
    // (`stdlib/option.nt`/`result.nt`/`array.nt`), not builtins;
    // `resolve_ok` resolves a bare `Module` directly with no driver and no
    // prelude loading, so they are genuinely unavailable here (see
    // `prelude_file_reexports_are_visible_everywhere_with_no_use` for
    // prelude-specific coverage instead).
    let resolved = resolve_ok("fn main() { println(\"hi\"); }");
    let println_id = resolved
        .definitions
        .lookup(&"println".into())
        .expect("println should be builtin");
    assert_eq!(resolved.definitions.get(println_id).kind, DefKind::Fn);
    for name in ["i32", "bool", "String"] {
        assert!(
            resolved.definitions.lookup(&name.into()).is_some(),
            "missing builtin {name}"
        );
    }
}
