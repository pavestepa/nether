use nether_frontend::{ast::*, parser::parse, source::SourceId};

fn module(text: &str) -> Module {
    let parsed = parse(SourceId(0), text);
    assert!(
        parsed.diagnostics.is_empty(),
        "{:#?}\n{text}",
        parsed.diagnostics
    );
    parsed.module.unwrap()
}
fn body(text: &str) -> Block {
    let mut module = module(text);
    let ItemKind::Function(function) = module.items.remove(0).kind else {
        panic!("expected function")
    };
    function.body.unwrap()
}
fn failure(text: &str) -> String {
    let parsed = parse(SourceId(0), text);
    assert!(parsed.module.is_none(), "accepted {text}");
    parsed.diagnostics[0].code.into()
}

#[test]
fn newlines_return_array_and_block_contexts_are_distinct() {
    for newline in ["\n", "\r\n", "\r"] {
        let text = "fn test(): () {\nlet empty: {i32; 0} = {}\nif true {}\nreturn\n(10)\n}"
            .replace('\n', newline);
        let body = body(&text);
        assert_eq!(body.statements.len(), 4);
        assert!(
            matches!(&body.statements[0].kind, StatementKind::Binding { value: Expr {kind: ExprKind::Array(v),..},..} if v.is_empty())
        );
        assert!(matches!(
            body.statements[1].kind,
            StatementKind::Expression(Expr {
                kind: ExprKind::If { .. },
                ..
            })
        ));
        assert!(matches!(
            body.statements[2].kind,
            StatementKind::Return(None)
        ));
        assert!(matches!(
            body.statements[3].kind,
            StatementKind::Expression(Expr {
                kind: ExprKind::Group(_),
                ..
            })
        ));
    }
}

#[test]
fn continuations_do_not_turn_new_statements_into_postfix_operations() {
    let body = body("fn f(): () {\nlet total = 1 +\n2\nservice\n.load(\n1,\n2,\n)\n(next)\n}");
    assert_eq!(body.statements.len(), 3);
    assert!(matches!(
        body.statements[1].kind,
        StatementKind::Expression(Expr {
            kind: ExprKind::Call { .. },
            ..
        })
    ));
    assert!(matches!(
        body.statements[2].kind,
        StatementKind::Expression(Expr {
            kind: ExprKind::Group(_),
            ..
        })
    ));
    failure("fn f(): () { let a=1 let b=2 }");
    failure("fn f(): () { call(1\n2) }");
}

#[test]
fn operators_have_correct_precedence_and_assignment_associativity() {
    let body = body("fn f(): bool { a = b = 1 + 2 * 3; return a == 7 && b == 7 }");
    let StatementKind::Expression(Expr {
        kind: ExprKind::Assign { value, .. },
        ..
    }) = &body.statements[0].kind
    else {
        panic!()
    };
    let ExprKind::Assign { value, .. } = &value.kind else {
        panic!()
    };
    let ExprKind::Binary {
        operator: "+",
        right,
        ..
    } = &value.kind
    else {
        panic!()
    };
    assert!(matches!(right.kind, ExprKind::Binary { operator: "*", .. }));
    failure("fn f(): bool { return 1 < 2 < 3 }");
    module("fn f(): bool { return (1 < 2) == true }");
}

#[test]
fn full_declaration_surface_and_nested_generic_closers() {
    let input = r#"
import {User, read} from "app/users"
export {User} from "app/users"
#[Copy]
struct Point<T: Copy = i32> {
    x: T
    fn get(this): T { return this.x }
    fn set(var this, value: T): () { this.x = value }
}
interface Read<T> { fn read(this): T }
class Box<T> : Read<T> {
    private value: T
    weak next: Box<T>
    constructor(value: T) { this.value = value }
    destructor { cleanup(this.value) }
    fn read(this): T { return value }
    fn create(value: T): Box<T> { return new Box<T>(value) }
}
enum Result<T, E> { Ok(T), Err { error: E }, Empty, }
const LENGTH: usize = 4
fn nested<const N: usize = 4>(value: Box<Box<i32>>): {i32; N} {
    let pointer: *var i32 = raw()
    unsafe { *(pointer + 1) = 2 }
    return {1,2,3,4}
}
#[link("c")]
extern "C" { fn puts(data: *u8): i32 }
extern "C" fn callback(value: i32): i32 { return value }
"#;
    assert_eq!(module(input).items.len(), 10);
}

#[test]
fn generic_calls_and_comparisons_are_disambiguated_without_losing_shifts() {
    let body = body("fn f(): () { let x = make<Box<i32>>()\nlet y = a < b >> 1\nlet z = Vec<i32>.create()\nlet p = Pair<i32> { first: 1, second: 2 } }");
    assert_eq!(body.statements.len(), 4);
    let StatementKind::Binding { value, .. } = &body.statements[1].kind else {
        panic!()
    };
    let ExprKind::Binary {
        operator: "<",
        right,
        ..
    } = &value.kind
    else {
        panic!()
    };
    assert!(matches!(
        right.kind,
        ExprKind::Binary { operator: ">>", .. }
    ));
}

#[test]
fn patterns_guards_and_branch_results_preserve_binding_modes() {
    let body = body("fn f(value: Result<i32,i32>): i32 { return match value { Result.Ok(ref var x) if valid(x) => { x; }, Result.Err {error: ref e} => e, Result.Empty => 0, } }");
    let StatementKind::Return(Some(Expr {
        kind: ExprKind::Match { arms, .. },
        ..
    })) = &body.statements[0].kind
    else {
        panic!()
    };
    assert_eq!(arms.len(), 3);
    assert!(arms[0].guard.is_some());
    let PatternKind::Variant {
        fields: PatternFields::Tuple(fields),
        ..
    } = &arms[0].pattern.kind
    else {
        panic!()
    };
    assert!(matches!(
        fields[0].kind,
        PatternKind::Binding {
            by_ref: true,
            mutable: true,
            ..
        }
    ));
    module("fn f(): () { let (var x,y) = (1,2); let {a,..} = {1,2,3} }");
    failure("fn f(): () { let {..,..} = {1,2,3} }");
}

#[test]
fn unsupported_syntax_is_rejected_at_source() {
    for (text, code) in [
        ("async fn f(): () {}", "E0100"),
        ("fn f(...args: i32): () {}", "E0101"),
        ("fn f(x: dyn Thing): () {}", "E0102"),
        ("fn f(): () { for x in values {} }", "E0103"),
        ("fn f(): () { let x = [1,2] }", "E0105"),
        ("fn f(): () { operation()? }", "E0107"),
        ("fn f(): () { if let x = value {} }", "E0107"),
    ] {
        assert_eq!(failure(text), code);
    }
    failure("fn f(this): () {}");
    failure("struct S { weak next: S }");
    failure("fn f<T = i32, U>(): () {}");
    failure("fn f<const N: i32>(): () {}");
}

#[test]
fn incomplete_and_deep_inputs_report_errors_without_panicking() {
    let sample = "fn test<T>(a: {T; 2}): T { return if true { a[0] } else { a[1] } }";
    for end in 0..sample.len() {
        let _ = parse(SourceId(0), &sample[..end]);
    }
    let nested = format!(
        "fn f(): i32 {{ return {}1{} }}",
        "(".repeat(300),
        ")".repeat(300)
    );
    assert_eq!(failure(&nested), "E0201");
    let chain = format!("fn f(): i32 {{ return 1{} }}", "+1".repeat(600));
    assert_eq!(failure(&chain), "E0201");
    for token in [
        "{", "}", "(", ")", "fn", "=>", "let", "this", "...", "<", ">>", "\n",
    ] {
        let _ = parse(SourceId(0), &token.repeat(100));
    }
}

#[test]
fn type_closers_split_adjacent_assignment_tokens() {
    module("fn f():() {let a:Option<i32>=get();let b:Outer<Inner<i32>>=get()} ");
    module("fn f(a:i32,b:i32,c:i32):bool {return a < b >> c}");
}
