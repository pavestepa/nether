use nether_ast::{BinaryOp, EnumVariant, Expr, ExprKind, Item, Pattern, SelfParam, Stmt, TypeDeclKind};
use nether_diagnostics::{Diagnostic, SourceMap};
use nether_parser::parse_module;

/// Parses `source` and panics (printing every diagnostic) if parsing
/// produced any — the common case for tests asserting a shape.
fn parse_ok(source: &str) -> nether_ast::Module {
    let mut map = SourceMap::new();
    let file = map.add_file("test.nr", source);
    let (module, diags) = parse_module(source, file);
    assert!(diags.is_empty(), "unexpected diagnostics: {}", render_all(&diags, &map));
    module
}

fn parse_with_diagnostics(source: &str) -> (nether_ast::Module, Vec<Diagnostic>) {
    let mut map = SourceMap::new();
    let file = map.add_file("test.nr", source);
    parse_module(source, file)
}

fn render_all(diags: &[Diagnostic], map: &SourceMap) -> String {
    diags.iter().map(|d| nether_diagnostics::render(d, map)).collect::<Vec<_>>().join("\n")
}

#[test]
fn canonical_spec_example_parses_cleanly() {
    // language-spec.md §15
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
    let module = parse_ok(source);
    // use, fn main, type Lang, impl Lang, impl Lang: Into<String>,
    // interface Sound, impl Lang: Sound
    assert_eq!(module.items.len(), 7);
    assert!(matches!(module.items[0], Item::Use(_)));
    assert!(matches!(module.items[1], Item::Fn(_)));
    assert!(matches!(module.items[2], Item::Type(_)));
    assert!(matches!(module.items[3], Item::Impl(_)));
    assert!(matches!(module.items[4], Item::Impl(_)));
    assert!(matches!(module.items[5], Item::Interface(_)));
    assert!(matches!(module.items[6], Item::Impl(_)));

    let Item::Impl(into_string_impl) = &module.items[4] else { unreachable!() };
    assert!(into_string_impl.interface.is_some());
}

#[test]
fn tuple_struct_and_unit_type() {
    let module = parse_ok("type Point(i32, i32);\ntype EmptyType;\n");
    assert_eq!(module.items.len(), 2);
    let Item::Type(point) = &module.items[0] else { panic!("expected TypeDecl") };
    assert!(matches!(point.kind, TypeDeclKind::TupleStruct(ref tys) if tys.len() == 2));
    let Item::Type(empty) = &module.items[1] else { panic!("expected TypeDecl") };
    assert!(matches!(empty.kind, TypeDeclKind::Unit));
}

#[test]
fn struct_with_private_field_via_underscore_and_keyword() {
    let module = parse_ok(
        r#"
type Config {
    name: String,
    _secret: String,
    private token: String
}
"#,
    );
    let Item::Type(decl) = &module.items[0] else { panic!("expected TypeDecl") };
    let TypeDeclKind::Struct(fields) = &decl.kind else { panic!("expected Struct") };
    assert!(!fields[0].private);
    assert!(fields[1].private);
    assert!(fields[2].private);
}

#[test]
fn enum_with_payload_and_generics() {
    let module = parse_ok(
        r#"
enum Color {
    Red,
    Custom(String),
}
enum Result<T, E> {
    Ok(T),
    Error(E),
}
"#,
    );
    let Item::Enum(color) = &module.items[0] else { panic!("expected EnumDecl") };
    assert_eq!(color.variants.len(), 2);
    let EnumVariant { payload, .. } = &color.variants[1];
    assert_eq!(payload.len(), 1);

    let Item::Enum(result) = &module.items[1] else { panic!("expected EnumDecl") };
    assert_eq!(result.generics.len(), 2);
    assert_eq!(result.generics[0].name.name.as_str(), "T");
    assert!(result.generics[0].bound.is_none());
}

#[test]
fn generic_bound_on_fn() {
    let module = parse_ok("fn f<T: Sound>(x: T) {\n    println(x);\n}\n");
    let Item::Fn(f) = &module.items[0] else { panic!("expected FnDecl") };
    assert_eq!(f.generics.len(), 1);
    let bound = f.generics[0].bound.as_ref().expect("expected a bound");
    let nether_ast::TypeExpr::Named { path, .. } = bound else { panic!("expected a Named bound") };
    assert_eq!(path.segments[0].name.as_str(), "Sound");
}

#[test]
fn generic_bound_can_itself_be_generic() {
    // `T: Into<String>` — the bound interface is itself parameterized.
    let module = parse_ok("fn f<T: Into<String>>(x: T) {\n    println(x);\n}\n");
    let Item::Fn(f) = &module.items[0] else { panic!("expected FnDecl") };
    let bound = f.generics[0].bound.as_ref().expect("expected a bound");
    let nether_ast::TypeExpr::Named { path, generics, .. } = bound else { panic!("expected a Named bound") };
    assert_eq!(path.segments[0].name.as_str(), "Into");
    assert_eq!(generics.len(), 1);
}

#[test]
fn interface_default_body_vs_required_method() {
    let module = parse_ok(
        r#"
interface Sound {
    sound(): String {
        "..."
    }
    required_method(self): i32;
}
"#,
    );
    let Item::Interface(decl) = &module.items[0] else { panic!("expected InterfaceDecl") };
    assert!(decl.methods[0].body.is_some());
    assert!(decl.methods[1].body.is_none());
    assert_eq!(decl.methods[1].self_param, Some(SelfParam::ByRef));
}

#[test]
fn match_with_dotted_variant_patterns() {
    let module = parse_ok(
        r#"
fn describe(color: Color) {
    match color {
        Color.Red => println("red"),
        Color.Custom(name) => println(name),
        _ => println("other"),
    }
}
"#,
    );
    let Item::Fn(f) = &module.items[0] else { panic!("expected FnDecl") };
    let body = f.body.as_ref().unwrap();
    // The match has no trailing `;` and is the last thing in the block, so
    // it is the block's tail expression, not a `Stmt` (language-spec §2.4).
    let match_expr = body.tail.as_ref().expect("expected a tail expression");
    let ExprKind::Match { arms, .. } = &match_expr.kind else { panic!("expected Match") };
    assert_eq!(arms.len(), 3);
    assert!(matches!(arms[0].pattern, Pattern::Variant { .. }));
    assert!(matches!(
        arms[1].pattern,
        Pattern::Variant { ref payload, .. } if payload.len() == 1
    ));
    assert!(matches!(arms[2].pattern, Pattern::Wildcard(_)));
}

#[test]
fn closure_and_mut_call_argument() {
    let module = parse_ok(
        r#"
fn increment(mut n: i32) {
    n = n + 1;
}

fn main() {
    let mut x = 4;
    increment(mut x);
    let add = (a: i32, b: i32) => { a + b };
}
"#,
    );
    let Item::Fn(increment) = &module.items[0] else { panic!("expected FnDecl") };
    assert!(increment.params[0].mutable);

    let Item::Fn(main_fn) = &module.items[1] else { panic!("expected FnDecl") };
    let body = main_fn.body.as_ref().unwrap();
    let Stmt::Expr(call) = &body.stmts[1] else { panic!("expected call statement") };
    let ExprKind::Call { args, .. } = &call.kind else { panic!("expected Call") };
    assert!(matches!(args[0].kind, ExprKind::MutArg(_)));

    let Stmt::Let(let_stmt) = &body.stmts[2] else { panic!("expected let statement") };
    assert!(matches!(let_stmt.value.kind, ExprKind::Closure { .. }));
}

#[test]
fn string_template_produces_literal_and_expr_parts() {
    let module = parse_ok(r#"fn main() { let s = `hi ${1 + 2}!`; }"#);
    let Item::Fn(f) = &module.items[0] else { panic!("expected FnDecl") };
    let body = f.body.as_ref().unwrap();
    let Stmt::Let(let_stmt) = &body.stmts[0] else { panic!("expected let statement") };
    let ExprKind::StringTemplate(parts) = &let_stmt.value.kind else { panic!("expected StringTemplate") };
    assert_eq!(parts.len(), 3);
    match &parts[1] {
        nether_ast::TemplatePart::Expr(e) => {
            assert!(matches!(e.kind, ExprKind::Binary { op: BinaryOp::Add, .. }));
        }
        other => panic!("expected an Expr part, got {other:?}"),
    }
}

#[test]
fn binary_operator_precedence() {
    // `1 + 2 * 3` must parse as `1 + (2 * 3)`.
    let module = parse_ok("fn main() { let x = 1 + 2 * 3; }");
    let Item::Fn(f) = &module.items[0] else { panic!("expected FnDecl") };
    let Stmt::Let(let_stmt) = &f.body.as_ref().unwrap().stmts[0] else { panic!("expected let") };
    let ExprKind::Binary { op: BinaryOp::Add, rhs, .. } = &let_stmt.value.kind else {
        panic!("expected a top-level Add");
    };
    assert!(matches!(rhs.kind, ExprKind::Binary { op: BinaryOp::Mul, .. }));
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
    let Item::Fn(f) = &module.items[0] else { panic!("expected FnDecl") };
    let body = f.body.as_ref().unwrap();
    // last thing in the block with no trailing `;` => tail, not a Stmt.
    let if_expr = body.tail.as_ref().expect("expected a tail expression");
    assert!(matches!(if_expr.kind, ExprKind::If { .. }));
}

#[test]
fn struct_literal_parses_outside_condition_position() {
    let module = parse_ok(r#"fn main() { let d = Dog { name }; }"#);
    let Item::Fn(f) = &module.items[0] else { panic!("expected FnDecl") };
    let Stmt::Let(let_stmt) = &f.body.as_ref().unwrap().stmts[0] else { panic!("expected let") };
    assert!(matches!(let_stmt.value.kind, ExprKind::StructLit { .. }));
}

#[test]
fn tuple_literal_and_index_access() {
    let module = parse_ok("fn main() { let t = (1, \"a\"); let x = t.0; }");
    let Item::Fn(f) = &module.items[0] else { panic!("expected FnDecl") };
    let stmts = &f.body.as_ref().unwrap().stmts;
    let Stmt::Let(first) = &stmts[0] else { panic!("expected let") };
    assert!(matches!(first.value.kind, ExprKind::Tuple(ref elems) if elems.len() == 2));
    let Stmt::Let(second) = &stmts[1] else { panic!("expected let") };
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
    let Item::Fn(f) = &module.items[0] else { panic!("expected FnDecl") };
    let body = f.body.as_ref().unwrap();
    assert!(matches!(body.stmts[1], Stmt::Expr(Expr { kind: ExprKind::While { .. }, .. })));
    assert!(matches!(body.stmts[2], Stmt::Expr(Expr { kind: ExprKind::ForIn { .. }, .. })));
    // `loop { break; }` is last with no trailing `;` => tail, not a Stmt.
    let tail = body.tail.as_ref().expect("expected a tail expression");
    assert!(matches!(tail.kind, ExprKind::Loop { .. }));
}

#[test]
fn weak_and_array_type_annotations() {
    let module = parse_ok(
        r#"
type Node {
    parent: weak Node,
    children: [Node]
}
"#,
    );
    let Item::Type(decl) = &module.items[0] else { panic!("expected TypeDecl") };
    let TypeDeclKind::Struct(fields) = &decl.kind else { panic!("expected Struct") };
    assert!(matches!(fields[0].ty, nether_ast::TypeExpr::Weak(_, _)));
    assert!(matches!(fields[1].ty, nether_ast::TypeExpr::Array(_, _)));
}

#[test]
fn malformed_item_reports_diagnostic_and_recovers() {
    let source = "type;\ntype Dog { name: String }\n";
    let (module, diags) = parse_with_diagnostics(source);
    assert!(!diags.is_empty());
    // recovery should still find the second, well-formed type declaration
    assert!(module.items.iter().any(|item| matches!(item, Item::Type(t) if t.name.name.as_str() == "Dog")));
}
