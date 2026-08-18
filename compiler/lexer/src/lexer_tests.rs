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
