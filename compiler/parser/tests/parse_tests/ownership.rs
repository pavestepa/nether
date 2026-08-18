//! Stage 1 grammar: the four value forms (`T`/`:T`/`t`/`:t`) and the
//! syntax changes that follow from them — language-spec §3, §4.2-4.3,
//! §7, §8, §11.

use super::*;
use nether_ast::{SelfParam, TypeExpr};

#[test]
fn allow_pascal_case_attribute_is_recorded_on_type_alias() {
    let module = parse_ok(
        r#"
#[allow_pascal_case]
pub type Coord = i32;
"#,
    );
    let Item::TypeAlias(alias) = &module.items[0] else {
        panic!("expected TypeAliasDecl")
    };
    assert!(alias.allow_pascal_case);
}

#[test]
fn item_attributes_reject_unknown_names_and_wrong_targets() {
    let (_, unknown) = parse_with_diagnostics("#[mystery]\ntype Alias = i32;");
    assert!(unknown.iter().any(|diagnostic| diagnostic
        .message
        .contains("unknown item attribute `mystery`")));

    let (_, wrong_target) = parse_with_diagnostics("#[allow_pascal_case]\nstruct Point { x i32 }");
    assert!(wrong_target.iter().any(|diagnostic| diagnostic
        .message
        .contains("only valid on a type alias declaration")));
}

#[test]
fn let_binding_forms_distinguish_arc_owned_and_inline() {
    let module = parse_ok(
        r#"
fn main() {
    let a = 3;
    let mut b = 3;
    let user User = c;
    let owned_user: User = d;
    let count i32 = 3;
    let owned_count: i32 = 3;
}
"#,
    );
    let Item::Fn(f) = &module.items[0] else {
        panic!("expected FnDecl")
    };
    let stmts = &f.body.as_ref().unwrap().stmts;

    let Stmt::Let(inferred) = &stmts[0] else {
        panic!("expected let")
    };
    assert!(inferred.ty.is_none());

    let Stmt::Let(user) = &stmts[2] else {
        panic!("expected let")
    };
    assert!(matches!(user.ty, Some(TypeExpr::Named { .. })));

    let Stmt::Let(owned_user) = &stmts[3] else {
        panic!("expected let")
    };
    assert!(matches!(owned_user.ty, Some(TypeExpr::Unique(_, _))));

    let Stmt::Let(count) = &stmts[4] else {
        panic!("expected let")
    };
    assert!(matches!(count.ty, Some(TypeExpr::Named { .. })));

    let Stmt::Let(owned_count) = &stmts[5] else {
        panic!("expected let")
    };
    assert!(matches!(owned_count.ty, Some(TypeExpr::Unique(_, _))));
}

#[test]
fn param_forms_cover_arc_mut_owned_and_borrows() {
    let module = parse_ok(
        r#"
fn foo(a Animal, b mut Animal, c: Animal, d: &Animal, e: &mut Animal) {
}
"#,
    );
    let Item::Fn(f) = &module.items[0] else {
        panic!("expected FnDecl")
    };
    assert_eq!(f.params.len(), 5);

    let a = &f.params[0];
    assert!(!a.mutable);
    assert!(matches!(a.ty, TypeExpr::Named { .. }));

    let b = &f.params[1];
    assert!(b.mutable);
    assert!(matches!(b.ty, TypeExpr::Named { .. }));

    let c = &f.params[2];
    assert!(!c.mutable);
    assert!(matches!(c.ty, TypeExpr::Unique(_, _)));

    let d = &f.params[3];
    assert!(matches!(d.ty, TypeExpr::Ref(_, _)));

    let e = &f.params[4];
    assert!(matches!(e.ty, TypeExpr::MutRef(_, _)));
}

#[test]
fn return_type_forms_distinguish_arc_owned_and_borrowed() {
    let module = parse_ok(
        r#"
fn create_user() User { }
fn create_user_owned(): User { }
fn count() i32 { }
fn count_owned(): i32 { }
fn get_name(user: &User): &String { }
"#,
    );
    let Item::Fn(arc_ret) = &module.items[0] else {
        panic!("expected FnDecl")
    };
    assert!(matches!(arc_ret.ret, Some(TypeExpr::Named { .. })));

    let Item::Fn(owned_ret) = &module.items[1] else {
        panic!("expected FnDecl")
    };
    assert!(matches!(owned_ret.ret, Some(TypeExpr::Unique(_, _))));

    let Item::Fn(inline_ret) = &module.items[2] else {
        panic!("expected FnDecl")
    };
    assert!(matches!(inline_ret.ret, Some(TypeExpr::Named { .. })));

    let Item::Fn(owned_inline_ret) = &module.items[3] else {
        panic!("expected FnDecl")
    };
    assert!(matches!(owned_inline_ret.ret, Some(TypeExpr::Unique(_, _))));

    let Item::Fn(borrowed_ret) = &module.items[4] else {
        panic!("expected FnDecl")
    };
    assert!(matches!(borrowed_ret.ret, Some(TypeExpr::Ref(_, _))));
    assert!(matches!(borrowed_ret.params[0].ty, TypeExpr::Ref(_, _)));
}

#[test]
fn self_receiver_forms_cover_arc_and_owned_domains() {
    let module = parse_ok(
        r#"
impl Animal {
    get_name(self) String { }
    set_name(mut self, name String) { }
    destroy(: self) { }
    get_name_ref(: &self): &String { }
    set_name_ref(: &mut self, name String) { }
}
"#,
    );
    let Item::Impl(block) = &module.items[0] else {
        panic!("expected ImplBlock")
    };
    assert_eq!(block.methods[0].self_param, Some(SelfParam::ByRef));
    assert_eq!(block.methods[1].self_param, Some(SelfParam::ByMutRef));
    assert_eq!(block.methods[2].self_param, Some(SelfParam::Owned));
    assert_eq!(block.methods[3].self_param, Some(SelfParam::OwnedRef));
    assert_eq!(block.methods[4].self_param, Some(SelfParam::OwnedMutRef));
}

#[test]
fn struct_literal_uses_eq_not_colon_for_fields() {
    let module = parse_ok(r#"fn main() { let d = Dog { name = "Rex", age }; }"#);
    let Item::Fn(f) = &module.items[0] else {
        panic!("expected FnDecl")
    };
    let Stmt::Let(let_stmt) = &f.body.as_ref().unwrap().stmts[0] else {
        panic!("expected let")
    };
    let ExprKind::StructLit { fields, owned, .. } = &let_stmt.value.kind else {
        panic!("expected StructLit")
    };
    assert!(!owned);
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].0.name.as_str(), "name");
    assert_eq!(fields[1].0.name.as_str(), "age");
}

#[test]
fn owned_struct_literal_parses_with_leading_colon() {
    let module = parse_ok(r#"fn main() { let d: Dog = :Dog { name = "Rex" }; }"#);
    let Item::Fn(f) = &module.items[0] else {
        panic!("expected FnDecl")
    };
    let Stmt::Let(let_stmt) = &f.body.as_ref().unwrap().stmts[0] else {
        panic!("expected let")
    };
    assert!(matches!(let_stmt.ty, Some(TypeExpr::Unique(_, _))));
    let ExprKind::StructLit { owned, .. } = &let_stmt.value.kind else {
        panic!("expected StructLit")
    };
    assert!(owned);
}

#[test]
fn struct_keyword_declares_a_struct_and_type_keyword_declares_an_alias() {
    let module = parse_ok(
        r#"
struct Dog {
    name String
}
type color = (u32, u32, u32);
"#,
    );
    assert!(matches!(module.items[0], Item::Struct(_)));
    let Item::TypeAlias(alias) = &module.items[1] else {
        panic!("expected TypeAliasDecl")
    };
    assert_eq!(alias.name.name.as_str(), "color");
    assert!(matches!(alias.ty, TypeExpr::Tuple(_, _)));
}

#[test]
fn owned_type_alias_is_reported_as_not_yet_supported() {
    let (_, diags) = parse_with_diagnostics("type: color = :(u32, u32, u32);\n");
    assert!(diags
        .iter()
        .any(|d| d.message.contains("not yet supported")));
}

#[test]
fn parses_explicit_move_closure() {
    let (module, diags) = parse_with_diagnostics("fn main() { let f = move () => { 1 }; }\n");
    assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    let Item::Fn(main) = &module.items[0] else {
        panic!("expected function");
    };
    let Stmt::Let(binding) = &main.body.as_ref().unwrap().stmts[0] else {
        panic!("expected let binding");
    };
    assert!(matches!(
        binding.value.kind,
        ExprKind::Closure {
            move_capture: true,
            ..
        }
    ));
}

#[test]
fn bare_block_is_not_an_expression() {
    let (_, diags) = parse_with_diagnostics(
        r#"
fn main() {
    let x = {
        let y = 1;
        y
    };
}
"#,
    );
    assert!(!diags.is_empty());
}

#[test]
fn closure_and_match_arm_bodies_still_accept_brace_blocks() {
    // These are the specific known-block positions that keep working even
    // though a bare `{ ... }` is no longer a general expression
    // (language-spec §2.6).
    let module = parse_ok(
        r#"
fn main() {
    let add = (a i32, b i32) => {
        a + b
    };
    let described = match add {
        _ => {
            1
        }
    };
}
"#,
    );
    let Item::Fn(f) = &module.items[0] else {
        panic!("expected FnDecl")
    };
    let stmts = &f.body.as_ref().unwrap().stmts;
    let Stmt::Let(add) = &stmts[0] else {
        panic!("expected let")
    };
    let ExprKind::Closure { body, .. } = &add.value.kind else {
        panic!("expected Closure")
    };
    assert!(matches!(body.kind, ExprKind::Block(_)));

    let Stmt::Let(described) = &stmts[1] else {
        panic!("expected let")
    };
    let ExprKind::Match { arms, .. } = &described.value.kind else {
        panic!("expected Match")
    };
    assert!(matches!(arms[0].body.kind, ExprKind::Block(_)));
}

#[test]
fn deep_reference_chain_nests_structurally() {
    let module = parse_ok("fn f(x: &&mut Animal) { }\n");
    let Item::Fn(f) = &module.items[0] else {
        panic!("expected FnDecl")
    };
    let TypeExpr::Ref(inner, _) = &f.params[0].ty else {
        panic!("expected an outer Ref")
    };
    assert!(matches!(**inner, TypeExpr::MutRef(_, _)));
}

#[test]
fn variadic_parameter_drops_the_colon() {
    let module = parse_ok("fn show(args ...String) {\n}\n");
    let Item::Fn(f) = &module.items[0] else {
        panic!("expected FnDecl")
    };
    assert!(f.params[0].variadic);
    assert!(matches!(f.params[0].ty, TypeExpr::Named { .. }));
}
