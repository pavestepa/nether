use nether_ast::{
    BinaryOp, EnumVariant, Expr, ExprKind, Item, Pattern, SelfParam, Stmt, StructDeclKind,
};
use nether_diagnostics::{Diagnostic, SourceMap};
use nether_parser::parse_module;

#[path = "parse_tests/async_syntax.rs"]
mod async_syntax;
#[path = "parse_tests/expressions.rs"]
mod expressions;
#[path = "parse_tests/ownership.rs"]
mod ownership;

/// Parses `source` and panics (printing every diagnostic) if parsing
/// produced any — the common case for tests asserting a shape.
fn parse_ok(source: &str) -> nether_ast::Module {
    let mut map = SourceMap::new();
    let file = map.add_file("test.nr", source);
    let (module, diags) = parse_module(source, file);
    assert!(
        diags.is_empty(),
        "unexpected diagnostics: {}",
        render_all(&diags, &map)
    );
    module
}

fn parse_with_diagnostics(source: &str) -> (nether_ast::Module, Vec<Diagnostic>) {
    let mut map = SourceMap::new();
    let file = map.add_file("test.nr", source);
    parse_module(source, file)
}

fn render_all(diags: &[Diagnostic], map: &SourceMap) -> String {
    diags
        .iter()
        .map(|d| nether_diagnostics::render(d, map))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn canonical_spec_example_parses_cleanly() {
    // language-spec.md §23
    let source = r#"
use lang.Lang;

fn main() {
    let a = Lang.new("Bobby");
    a.set_name("Husky");
    println(a.into_string());
}

struct Lang {
    name String
}

impl Lang {
    new(name String) Lang {
        Lang { name }
    }

    set_name(mut self, new_name String) {
        self.name = new_name;
    }
}

impl Lang Into<String> {
    into_string(self) String {
        `name: ${self.name}`
    }
}

trait Sound {
    sound() String {
        "..."
    }
}

impl Lang Sound {
    sound() String {
        "Woof! Ruff!"
    }
}
"#;
    let module = parse_ok(source);
    // use, fn main, struct Lang, impl Lang, impl Lang Into<String>,
    // trait Sound, impl Lang Sound
    assert_eq!(module.items.len(), 7);
    assert!(matches!(module.items[0], Item::Use(_)));
    assert!(matches!(module.items[1], Item::Fn(_)));
    assert!(matches!(module.items[2], Item::Struct(_)));
    assert!(matches!(module.items[3], Item::Impl(_)));
    assert!(matches!(module.items[4], Item::Impl(_)));
    assert!(matches!(module.items[5], Item::Trait(_)));
    assert!(matches!(module.items[6], Item::Impl(_)));

    let Item::Impl(into_string_impl) = &module.items[4] else {
        unreachable!()
    };
    assert_eq!(into_string_impl.traits.len(), 1);
}

#[test]
fn module_declarations_and_relative_use_roots_parse() {
    let module = parse_ok(
        r#"
mod child;
use self.child.make;
use super.Parent;
use crate.Root;
"#,
    );
    assert_eq!(module.items.len(), 4);
    let Item::Mod(child) = &module.items[0] else {
        panic!("expected ModDecl")
    };
    assert_eq!(child.name.name.as_str(), "child");
    for (item, root) in module.items[1..].iter().zip(["self", "super", "crate"]) {
        let Item::Use(use_decl) = item else {
            panic!("expected UseDecl")
        };
        assert_eq!(use_decl.path.segments[0].name.as_str(), root);
    }
}

#[test]
fn use_enum_variant_path_parses_with_three_segments() {
    // `use module.Enum.Variant;` — the parser places no cap on segment
    // count; disambiguating a genuinely nested module path from an
    // enum-variant reach-through is the driver's job, not the parser's.
    let module = parse_ok("use option.Option.Some;\n");
    let Item::Use(use_decl) = &module.items[0] else {
        panic!("expected UseDecl")
    };
    let names: Vec<&str> = use_decl
        .path
        .segments
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(names, ["option", "Option", "Some"]);
}

#[test]
fn module_visibility_is_private_by_default_and_pub_is_explicit() {
    let module = parse_ok("mod option;\npub mod result;\n");
    let Item::Mod(option) = &module.items[0] else {
        panic!("expected ModDecl")
    };
    assert_eq!(option.name.name.as_str(), "option");
    assert_eq!(option.visibility, nether_ast::Visibility::Private);
    let Item::Mod(result) = &module.items[1] else {
        panic!("expected ModDecl")
    };
    assert_eq!(result.visibility, nether_ast::Visibility::Public);
}

#[test]
fn pub_applies_to_every_api_item_and_method_shape() {
    let module = parse_ok(
        r#"
pub use api.Value;
pub struct Value { pub exposed i32, hidden i32 }
pub type count = i32;
pub enum Choice { One }
pub trait Named { pub name(self) String; }
pub fn make() Value { return Value { exposed = 1, hidden = 2 }; }
impl Value {
    pub new() Value { return make(); }
    pub get(self) i32 { return self.exposed; }
}
"#,
    );
    assert!(matches!(&module.items[0], Item::Use(item) if item.visibility.is_public()));
    assert!(matches!(&module.items[1], Item::Struct(item) if item.visibility.is_public()));
    assert!(matches!(&module.items[2], Item::TypeAlias(item) if item.visibility.is_public()));
    assert!(matches!(&module.items[3], Item::Enum(item) if item.visibility.is_public()));
    assert!(matches!(&module.items[4], Item::Trait(item) if item.visibility.is_public()));
    assert!(matches!(&module.items[5], Item::Fn(item) if item.visibility.is_public()));
    let Item::Impl(block) = &module.items[6] else {
        panic!("expected impl")
    };
    assert!(block
        .methods
        .iter()
        .all(|method| method.visibility.is_public()));
}

#[test]
fn tuple_struct_and_unit_type() {
    let module = parse_ok("struct Point(i32, i32);\nstruct EmptyType;\n");
    assert_eq!(module.items.len(), 2);
    let Item::Struct(point) = &module.items[0] else {
        panic!("expected StructDecl")
    };
    assert!(matches!(point.kind, StructDeclKind::TupleStruct(ref tys) if tys.len() == 2));
    let Item::Struct(empty) = &module.items[1] else {
        panic!("expected StructDecl")
    };
    assert!(matches!(empty.kind, StructDeclKind::Unit));
}

#[test]
fn struct_fields_are_private_by_default_and_pub_is_explicit() {
    let module = parse_ok(
        r#"
struct Config {
    pub name String,
    _secret String,
    token String
}
"#,
    );
    let Item::Struct(decl) = &module.items[0] else {
        panic!("expected StructDecl")
    };
    let StructDeclKind::Struct(fields) = &decl.kind else {
        panic!("expected Struct")
    };
    assert_eq!(fields[0].visibility, nether_ast::Visibility::Public);
    assert_eq!(fields[1].visibility, nether_ast::Visibility::Private);
    assert_eq!(fields[2].visibility, nether_ast::Visibility::Private);
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
    let Item::Enum(color) = &module.items[0] else {
        panic!("expected EnumDecl")
    };
    assert_eq!(color.variants.len(), 2);
    let EnumVariant { payload, .. } = &color.variants[1];
    assert_eq!(payload.len(), 1);

    let Item::Enum(result) = &module.items[1] else {
        panic!("expected EnumDecl")
    };
    assert_eq!(result.generics.len(), 2);
    assert_eq!(result.generics[0].name.name.as_str(), "T");
    assert!(result.generics[0].bounds.is_empty());
}

#[test]
fn variadic_parameter_parses_as_element_type_with_flag_set() {
    let module = parse_ok("fn println(args ...String) {\n}\n");
    let Item::Fn(f) = &module.items[0] else {
        panic!("expected FnDecl")
    };
    assert_eq!(f.params.len(), 1);
    assert!(f.params[0].variadic);
    let nether_ast::TypeExpr::Named { path, .. } = &f.params[0].ty else {
        panic!("expected a Named element type")
    };
    assert_eq!(path.segments[0].name.as_str(), "String");
}

#[test]
fn variadic_parameter_must_be_last() {
    let (_, diags) = parse_with_diagnostics("fn f(args ...String, x i32) {}\n");
    assert!(diags.iter().any(|d| d.message.contains("must be the last")));
}

#[test]
fn removed_private_keyword_reports_a_diagnostic_and_recovers() {
    let (module, diags) =
        parse_with_diagnostics("private struct printsys;\nfn after() i32 { return 1; }\n");
    assert!(diags.iter().any(|d| d.message.contains("expected an item")));
    // Recovery must still make it to the next real item.
    assert!(module.items.iter().any(|item| matches!(
        item,
        Item::Fn(f) if f.name.name.as_str() == "after"
    )));
}

#[test]
fn generic_bound_on_fn() {
    let module = parse_ok("fn f<T Sound>(x T) {\n    println(x);\n}\n");
    let Item::Fn(f) = &module.items[0] else {
        panic!("expected FnDecl")
    };
    assert_eq!(f.generics.len(), 1);
    let bound = f.generics[0].bounds.first().expect("expected a bound");
    let nether_ast::TypeExpr::Named { path, .. } = bound else {
        panic!("expected a Named bound")
    };
    assert_eq!(path.segments[0].name.as_str(), "Sound");
}

#[test]
fn generic_bound_can_itself_be_generic() {
    // `T: Into<String>` — the bound interface is itself parameterized.
    let module = parse_ok("fn f<T Into<String>>(x T) {\n    println(x);\n}\n");
    let Item::Fn(f) = &module.items[0] else {
        panic!("expected FnDecl")
    };
    let bound = f.generics[0].bounds.first().expect("expected a bound");
    let nether_ast::TypeExpr::Named { path, generics, .. } = bound else {
        panic!("expected a Named bound")
    };
    assert_eq!(path.segments[0].name.as_str(), "Into");
    assert_eq!(generics.len(), 1);
}

#[test]
fn generic_parameter_accepts_multiple_bounds() {
    let module = parse_ok("fn describe<T Sound + Named>(value T) {}\n");
    let Item::Fn(f) = &module.items[0] else {
        panic!("expected FnDecl")
    };
    assert_eq!(f.generics[0].bounds.len(), 2);
    let names: Vec<_> = f.generics[0]
        .bounds
        .iter()
        .map(|bound| match bound {
            nether_ast::TypeExpr::Named { path, .. } => path.segments[0].name.as_str(),
            _ => panic!("expected named bound"),
        })
        .collect();
    assert_eq!(names, ["Sound", "Named"]);

    let module = parse_ok("fn describe<T>(value T) where T Sound + Named {}\n");
    let Item::Fn(f) = &module.items[0] else {
        panic!("expected FnDecl")
    };
    assert_eq!(f.generics[0].bounds.len(), 2);

    let (_, diagnostics) = parse_with_diagnostics("fn invalid<T>(value T) where U Sound {}\n");
    assert!(diagnostics.iter().any(|diagnostic| diagnostic
        .message
        .contains("undeclared generic parameter `U`")));

    let (_, diagnostics) = parse_with_diagnostics("fn old<T: Sound>(value T) {}\n");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message.contains("write `<T Trait>`")));

    let (_, diagnostics) = parse_with_diagnostics("fn old<T>(value T) where T: Sound {}\n");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message.contains("write `where T Trait`")));

    parse_ok(
        r#"
struct Box<T> where T Sound { value T }
enum Maybe<T> where T Sound { Some(T), None }
trait Convert<T> where T Sound { convert<U>(value U) where U Named; }
impl<T> Box<T> where T Sound { get<U>(self, fallback U) T where U Named { return self.value; } }
"#,
    );
}

#[test]
fn explicit_generic_impl_block_parses_generics_and_target_args() {
    // `impl<T> Option<T> { ... }` — the Rust-like explicit form, needed to
    // bind a type parameter for a builtin owner (`Option`) with no local
    // declaration. The implicit `impl Boxed { ... }` form still parses
    // with both new fields empty.
    let module = parse_ok(
        r#"
default impl<T> Option<T> {
    is_some(self) bool {
        true
    }
}

struct Boxed<T> {
    value T
}

impl Boxed {
    get(self) T {
        self.value
    }
}
"#,
    );
    let Item::Impl(explicit) = &module.items[0] else {
        panic!("expected ImplBlock")
    };
    assert_eq!(explicit.target.name.as_str(), "Option");
    assert!(explicit.is_default);
    assert_eq!(explicit.generics.len(), 1);
    assert_eq!(explicit.generics[0].name.name.as_str(), "T");
    assert_eq!(explicit.target_args.len(), 1);
    let nether_ast::TypeExpr::Named { path, generics, .. } = &explicit.target_args[0] else {
        panic!("expected a Named target arg")
    };
    assert_eq!(path.segments[0].name.as_str(), "T");
    assert!(generics.is_empty());

    let Item::Impl(implicit) = &module.items[2] else {
        panic!("expected ImplBlock")
    };
    assert_eq!(implicit.target.name.as_str(), "Boxed");
    assert!(!implicit.is_default);
    assert!(implicit.generics.is_empty());
    assert!(implicit.target_args.is_empty());
}

#[test]
fn trait_default_body_vs_required_method() {
    let module = parse_ok(
        r#"
trait Sound {
    const CHANNELS i32;
    pub const DEFAULT_VOLUME i32 = 10;
    sound() String {
        "..."
    }
    required_method(self) i32;
}
struct Speaker;
impl Speaker Sound {
    const CHANNELS i32 = 2;
    const DEFAULT_VOLUME i32 = 8;
    sound() String { return "speaker"; }
    required_method(self) i32 { return 1; }
}
"#,
    );
    let Item::Trait(decl) = &module.items[0] else {
        panic!("expected TraitDecl")
    };
    assert!(decl.methods[0].body.is_some());
    assert!(decl.methods[1].body.is_none());
    assert_eq!(decl.methods[1].self_param, Some(SelfParam::ByRef));
    assert_eq!(decl.associated_consts.len(), 2);
    assert!(decl.associated_consts[0].value.is_none());
    assert!(decl.associated_consts[1].value.is_some());
    assert_eq!(
        decl.associated_consts[1].visibility,
        nether_ast::Visibility::Public
    );
    let Item::Impl(block) = &module.items[2] else {
        panic!("expected ImplBlock")
    };
    assert_eq!(block.associated_consts.len(), 2);
    assert!(block
        .associated_consts
        .iter()
        .all(|constant| constant.value.is_some()));
}

#[test]
fn match_with_dotted_variant_patterns() {
    let module = parse_ok(
        r#"
fn describe(color Color) {
    match color {
        Color.Red => println("red"),
        Color.Custom(name) => println(name),
        _ => println("other"),
    }
}
"#,
    );
    let Item::Fn(f) = &module.items[0] else {
        panic!("expected FnDecl")
    };
    let body = f.body.as_ref().unwrap();
    // The match has no trailing `;` and is the last thing in the block, so
    // it is the block's tail expression, not a `Stmt` (language-spec §2.4).
    let match_expr = body.tail.as_ref().expect("expected a tail expression");
    let ExprKind::Match { arms, .. } = &match_expr.kind else {
        panic!("expected Match")
    };
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
fn increment(n mut i32) {
    n = n + 1;
}

fn main() {
    let mut x = 4;
    increment(mut x);
    let add = (a i32, b i32) => { a + b };
}
"#,
    );
    let Item::Fn(increment) = &module.items[0] else {
        panic!("expected FnDecl")
    };
    assert!(increment.params[0].mutable);

    let Item::Fn(main_fn) = &module.items[1] else {
        panic!("expected FnDecl")
    };
    let body = main_fn.body.as_ref().unwrap();
    let Stmt::Expr(call) = &body.stmts[1] else {
        panic!("expected call statement")
    };
    let ExprKind::Call { args, .. } = &call.kind else {
        panic!("expected Call")
    };
    assert!(matches!(args[0].kind, ExprKind::MutArg(_)));

    let Stmt::Let(let_stmt) = &body.stmts[2] else {
        panic!("expected let statement")
    };
    assert!(matches!(let_stmt.value.kind, ExprKind::Closure { .. }));
}
