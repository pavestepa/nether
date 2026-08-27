#[cfg(test)]
mod tests {
    use crate::{tokenize, Keyword, Punct, TemplatePartTok, Token};
    use nether_diagnostics::SourceMap;

    fn tokens_of(source: &str) -> Vec<Token> {
        let mut map = SourceMap::new();
        let file = map.add_file("test.nr", source);
        let (tokens, diags) = tokenize(source, file);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        tokens.into_iter().map(|t| t.token).collect()
    }

    #[test]
    fn keywords_and_idents() {
        let toks = tokens_of("let mut Dog self mod");
        assert_eq!(
            toks,
            vec![
                Token::Keyword(Keyword::Let),
                Token::Keyword(Keyword::Mut),
                Token::Ident("Dog".into()),
                Token::Keyword(Keyword::SelfLower),
                Token::Keyword(Keyword::Mod),
                Token::Eof,
            ]
        );
    }

    #[test]
    fn primitive_type_names_are_plain_idents() {
        // i32/bool/... are NOT keywords (language-spec: casing rule and
        // primitive recognition live in resolver/typecheck, not the lexer).
        let toks = tokens_of("i32 bool usize");
        assert_eq!(
            toks,
            vec![
                Token::Ident("i32".into()),
                Token::Ident("bool".into()),
                Token::Ident("usize".into()),
                Token::Eof,
            ]
        );
    }

    #[test]
    fn integer_and_float_literals() {
        let toks = tokens_of("123 4.5 0");
        assert_eq!(
            toks,
            vec![
                Token::Int(123),
                Token::Float(4.5),
                Token::Int(0),
                Token::Eof
            ]
        );
    }

    #[test]
    fn dot_after_ident_is_not_consumed_by_number_lexing() {
        // `a.0` (tuple index access) must lex as Ident, Dot, Int — not as
        // some merged float-like token.
        let toks = tokens_of("a.0");
        assert_eq!(
            toks,
            vec![
                Token::Ident("a".into()),
                Token::Punct(Punct::Dot),
                Token::Int(0),
                Token::Eof
            ]
        );
    }

    #[test]
    fn plain_string_literal_with_escapes() {
        let toks = tokens_of(r#""hello\nworld""#);
        assert_eq!(
            toks,
            vec![Token::Str("hello\nworld".to_string()), Token::Eof]
        );
    }

    #[test]
    fn template_string_splits_literal_and_expr_segments() {
        let toks = tokens_of("`name: ${self.name}!`");
        match &toks[0] {
            Token::TemplateStr(parts) => {
                assert_eq!(parts.len(), 3);
                assert_eq!(parts[0], TemplatePartTok::Literal("name: ".to_string()));
                match &parts[1] {
                    TemplatePartTok::Expr(text, _) => assert_eq!(text, "self.name"),
                    other => panic!("expected Expr part, got {other:?}"),
                }
                assert_eq!(parts[2], TemplatePartTok::Literal("!".to_string()));
            }
            other => panic!("expected TemplateStr, got {other:?}"),
        }
    }

    #[test]
    fn template_string_trailing_literal_after_interpolation() {
        let toks = tokens_of("`a${x}b`");
        match &toks[0] {
            Token::TemplateStr(parts) => {
                assert_eq!(parts.len(), 3);
                assert_eq!(parts[0], TemplatePartTok::Literal("a".to_string()));
                assert!(matches!(&parts[1], TemplatePartTok::Expr(text, _) if text == "x"));
                assert_eq!(parts[2], TemplatePartTok::Literal("b".to_string()));
            }
            other => panic!("expected TemplateStr, got {other:?}"),
        }
    }

    #[test]
    fn char_literal() {
        let toks = tokens_of("'a'");
        assert_eq!(toks, vec![Token::Char('a'), Token::Eof]);
    }

    #[test]
    fn doc_comment_is_kept_plain_comment_is_dropped() {
        let toks = tokens_of("// plain\n/// documented\nlet a = 1;");
        assert_eq!(
            toks,
            vec![
                Token::DocComment("documented".to_string()),
                Token::Keyword(Keyword::Let),
                Token::Ident("a".into()),
                Token::Punct(Punct::Eq),
                Token::Int(1),
                Token::Punct(Punct::Semi),
                Token::Eof,
            ]
        );
    }

    #[test]
    fn block_comments_are_discarded_including_multiline_ones() {
        let toks = tokens_of("/* one line */let a/* mid */= /*\n multi\n line\n*/1;");
        assert_eq!(
            toks,
            vec![
                Token::Keyword(Keyword::Let),
                Token::Ident("a".into()),
                Token::Punct(Punct::Eq),
                Token::Int(1),
                Token::Punct(Punct::Semi),
                Token::Eof,
            ]
        );
    }

    #[test]
    fn block_comments_do_not_nest() {
        // The inner `/*` is just more discarded text — the comment ends
        // at the *first* `*/`, matching C/Go/JS/Kotlin, not Rust/Swift.
        let toks = tokens_of("/* outer /* inner */ let a = 1;");
        assert_eq!(
            toks,
            vec![
                Token::Keyword(Keyword::Let),
                Token::Ident("a".into()),
                Token::Punct(Punct::Eq),
                Token::Int(1),
                Token::Punct(Punct::Semi),
                Token::Eof,
            ]
        );
    }

    #[test]
    fn unterminated_block_comment_reports_diagnostic() {
        let mut map = SourceMap::new();
        let source = "let a = 1; /* never closed";
        let file = map.add_file("test.nr", source);
        let (_, diags) = tokenize(source, file);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("unterminated block comment"));
    }

    #[test]
    fn newline_after_a_trigger_token_inserts_a_semicolon() {
        let toks = tokens_of("let a = 1\nlet b = 2");
        assert_eq!(
            toks,
            vec![
                Token::Keyword(Keyword::Let),
                Token::Ident("a".into()),
                Token::Punct(Punct::Eq),
                Token::Int(1),
                Token::Punct(Punct::Semi),
                Token::Keyword(Keyword::Let),
                Token::Ident("b".into()),
                Token::Punct(Punct::Eq),
                Token::Int(2),
                Token::Eof,
            ]
        );
    }

    #[test]
    fn a_real_trailing_semicolon_is_not_duplicated() {
        let toks = tokens_of("let a = 1;\nlet b = 2;");
        assert_eq!(
            toks,
            vec![
                Token::Keyword(Keyword::Let),
                Token::Ident("a".into()),
                Token::Punct(Punct::Eq),
                Token::Int(1),
                Token::Punct(Punct::Semi),
                Token::Keyword(Keyword::Let),
                Token::Ident("b".into()),
                Token::Punct(Punct::Eq),
                Token::Int(2),
                Token::Punct(Punct::Semi),
                Token::Eof,
            ]
        );
    }

    #[test]
    fn blank_lines_after_a_statement_insert_only_one_semicolon() {
        let toks = tokens_of("let a = 1\n\n\nlet b = 2");
        let semi_count = toks
            .iter()
            .filter(|t| **t == Token::Punct(Punct::Semi))
            .count();
        assert_eq!(semi_count, 1);
    }

    #[test]
    fn newline_after_a_non_trigger_token_inserts_nothing() {
        // `+` can't end a statement -- the expression must continue.
        let toks = tokens_of("let a = 1 +\n2");
        assert_eq!(
            toks,
            vec![
                Token::Keyword(Keyword::Let),
                Token::Ident("a".into()),
                Token::Punct(Punct::Eq),
                Token::Int(1),
                Token::Punct(Punct::Plus),
                Token::Int(2),
                Token::Eof,
            ]
        );
    }

    #[test]
    fn newline_right_after_return_ends_the_statement_with_no_value() {
        // Mirrors Go's own resolution of JS's classic `return\n(expr)`
        // hazard: `return` is itself a trigger token, so the newline
        // ends the statement before `(expr)` is ever considered part of
        // it, rather than ambiguously extending onto the next line.
        let toks = tokens_of("return\n(1)");
        assert_eq!(
            toks,
            vec![
                Token::Keyword(Keyword::Return),
                Token::Punct(Punct::Semi),
                Token::Punct(Punct::LParen),
                Token::Int(1),
                Token::Punct(Punct::RParen),
                Token::Eof,
            ]
        );
    }

    #[test]
    fn no_semicolon_inserted_inside_a_multiline_parameter_list() {
        // The real hazard a naive "insert after any trigger token, at
        // any nesting depth" rule would hit: each parameter's own type
        // name (an identifier, a trigger) sits at the end of its line.
        let toks = tokens_of("fn f(\n    a i32,\n    b i32\n) i32 {\n    return a\n}");
        // Zero, not one: `return a` sits directly before the closing
        // `}` (the block's own tail position), so the "never insert
        // right before `}`" tail-expression-safety rule already leaves
        // it alone -- semantically fine either way, since `return`
        // unconditionally exits regardless of tail position.
        assert!(!toks.contains(&Token::Punct(Punct::Semi)), "{toks:?}");
        assert!(!toks
            .windows(2)
            .any(|w| w[0] == Token::Ident("i32".into()) && w[1] == Token::Punct(Punct::Semi)));
    }

    #[test]
    fn no_semicolon_inserted_inside_a_multiline_array_or_call_argument_list() {
        let toks = tokens_of("let a = [\n    1,\n    2\n]");
        assert!(!toks.contains(&Token::Punct(Punct::Semi)));
        let toks = tokens_of("foo(\n    1,\n    2\n)");
        assert!(!toks.contains(&Token::Punct(Punct::Semi)));
    }

    #[test]
    fn no_semicolon_inserted_inside_a_multiline_struct_declaration() {
        // Real bug #1: `struct Lang {\n    name String\n}` — `String`
        // is an identifier (a trigger), directly followed by the
        // closing `}`. Caught against language-spec.md's own canonical
        // example (`compiler/codegen/tests/codegen_tests.rs`'s
        // `canonical_spec_example_compiles_to_a_verified_module`).
        let toks = tokens_of("struct Lang {\n    name String\n}\nfn main() {}");
        assert!(!toks.contains(&Token::Punct(Punct::Semi)), "{toks:?}");
    }

    #[test]
    fn no_semicolon_inserted_before_a_blocks_own_closing_brace() {
        // Real bug #2: inserting a semicolon directly before a `}`
        // would silently change a block's own *value* — Nether is
        // expression-oriented, so a block's last construct with no
        // trailing `;` is that block's tail expression. Caught against
        // `compiler/codegen/tests/codegen_tests.rs`'s own
        // `closure_with_stack_and_heap_captures_compiles_to_an_indirect_call`.
        let toks = tokens_of("let f = (x i32) => {\n    x\n}");
        assert!(!toks.contains(&Token::Punct(Punct::Semi)), "{toks:?}");
    }

    #[test]
    fn no_semicolon_inserted_between_an_attribute_and_its_item() {
        // Real bug #3: `#[link(name = "m")]` ends in `]`, itself
        // ordinarily a trigger — but this specific `]` closes an
        // attribute, not an array/index expression, and sits directly
        // before the item it decorates. Caught against
        // `compiler/driver/tests/driver_tests/link_attribute.rs`'s own
        // end-to-end test. (The one real `Semi` in this source is the
        // extern fn signature's own explicit terminator, not ASI's.)
        let toks = tokens_of("#[link(name = \"m\")]\nextern \"C\" {\n    fn sqrt(x f64) f64;\n}");
        assert!(
            !toks
                .windows(2)
                .any(|w| w[0] == Token::Punct(Punct::RBracket) && w[1] == Token::Punct(Punct::Semi)),
            "{toks:?}"
        );
    }

    #[test]
    fn ordinary_array_literal_closing_bracket_still_triggers_normally() {
        // The attribute exception must stay narrow: an ordinary `]`
        // (not preceded by `#[`) keeps triggering ASI as normal.
        let toks = tokens_of("let a = [1, 2]\nlet b = 3");
        let semi_count = toks
            .iter()
            .filter(|t| **t == Token::Punct(Punct::Semi))
            .count();
        assert_eq!(semi_count, 1, "{toks:?}");
    }

    #[test]
    fn multi_char_punctuation() {
        let toks = tokens_of("== != <= >= => && ||");
        assert_eq!(
            toks,
            vec![
                Token::Punct(Punct::EqEq),
                Token::Punct(Punct::Ne),
                Token::Punct(Punct::Le),
                Token::Punct(Punct::Ge),
                Token::Punct(Punct::FatArrow),
                Token::Punct(Punct::AmpAmp),
                Token::Punct(Punct::PipePipe),
                Token::Eof,
            ]
        );
    }

    #[test]
    fn unterminated_string_reports_diagnostic() {
        let mut map = SourceMap::new();
        let source = "\"unterminated";
        let file = map.add_file("test.nr", source);
        let (_, diags) = tokenize(source, file);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("unterminated string"));
    }

    #[test]
    fn invalid_character_recovers_with_error_token() {
        let mut map = SourceMap::new();
        let source = "let a = 1 @ 2";
        let file = map.add_file("test.nr", source);
        let (tokens, diags) = tokenize(source, file);
        assert_eq!(diags.len(), 1);
        assert!(tokens.iter().any(|t| t.token == Token::Error));
        // lexing continues past the bad character:
        assert!(tokens.iter().any(|t| t.token == Token::Int(2)));
    }

    #[test]
    fn item_attribute_punctuation_is_tokenized() {
        assert_eq!(
            tokens_of("#[allow_pascal_case]"),
            vec![
                Token::Punct(Punct::Hash),
                Token::Punct(Punct::LBracket),
                Token::Ident("allow_pascal_case".into()),
                Token::Punct(Punct::RBracket),
                Token::Eof,
            ]
        );
    }
}
