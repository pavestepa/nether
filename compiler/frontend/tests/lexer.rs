use nether_frontend::{
    lexer::{lex, TokenKind as K},
    source::{Source, SourceId},
};

#[test]
fn preserves_byte_spans_unicode_literals_and_all_line_endings() {
    let text = "\u{feff}let x = 'λ'\r\n/* a\r/* b */\n*/x";
    let result = lex(SourceId(4), text);
    assert!(result.diagnostics.is_empty());
    assert_eq!(
        result
            .tokens
            .iter()
            .filter(|t| t.kind == K::Newline)
            .count(),
        3
    );
    let char_token = result
        .tokens
        .iter()
        .find(|t| matches!(t.kind, K::Char(_)))
        .unwrap();
    assert_eq!(&text[char_token.span.start..char_token.span.end], "'λ'");
    assert_eq!(char_token.span.source, SourceId(4));
    for token in &result.tokens {
        assert!(text.get(token.span.start..token.span.end).is_some());
    }
    let source = Source::new("case.nr", text);
    assert_eq!(source.location(text.len() - 1), Some((4, 3)));
    assert_eq!(source.location(1), None);
}

#[test]
fn lexes_numeric_bases_exponents_and_longest_symbols() {
    let result = lex(
        SourceId(0),
        "0xFF 0o17 0b10 1_000 1.25e-3 7E2 1..2 >>= ... =>",
    );
    assert!(result.diagnostics.is_empty());
    let kinds: Vec<_> = result.tokens.into_iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![
            K::Integer("0xFF".into()),
            K::Integer("0o17".into()),
            K::Integer("0b10".into()),
            K::Integer("1_000".into()),
            K::Float("1.25e-3".into()),
            K::Float("7E2".into()),
            K::Integer("1".into()),
            K::Symbol(".."),
            K::Integer("2".into()),
            K::Symbol(">>="),
            K::Symbol("..."),
            K::Symbol("=>"),
            K::Eof
        ]
    );
}

#[test]
fn rejects_entire_malformed_numeric_literal_without_losing_following_tokens() {
    for literal in [
        "0x", "0b2", "1__2", "2_", "1e", "1e+", "3u8", "1.0abc", "1.2_e3",
    ] {
        let result = lex(SourceId(0), &format!("{literal};next"));
        assert!(!result.diagnostics.is_empty(), "{literal}");
        assert!(result
            .tokens
            .iter()
            .any(|t| t.kind == K::Word("next".into())));
    }
}

#[test]
fn decodes_escapes_and_validates_unicode_scalars() {
    let result = lex(SourceId(0), r#""a\n\x7f\u{1F600}" '\u{3bb}'"#);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert_eq!(result.tokens[0].kind, K::String("a\n\x7f😀".into()));
    assert_eq!(result.tokens[1].kind, K::Char('λ'));
    for bad in [
        r#"'ab'"#,
        r#"''"#,
        r#""\x80""#,
        r#"'\u{d800}'"#,
        r#"'\u{110000}'"#,
        r#"'\u{}'"#,
        r#"'\q'"#,
        "\"open\nnext",
        "/* open",
    ] {
        let result = lex(SourceId(0), bad);
        assert!(!result.diagnostics.is_empty(), "{bad}");
        assert_eq!(result.tokens.last().unwrap().kind, K::Eof);
    }
}

#[test]
fn arbitrary_short_utf8_inputs_terminate_with_valid_spans() {
    let alphabet = [
        'a', 'λ', '\0', '\r', '\n', '\'', '"', '\\', '/', '*', '0', '`', '😀',
    ];
    for a in alphabet {
        for b in alphabet {
            for c in alphabet {
                let text: String = [a, b, c].into_iter().collect();
                let result = lex(SourceId(0), &text);
                assert_eq!(result.tokens.last().unwrap().span.start, text.len());
                for span in result
                    .tokens
                    .iter()
                    .map(|t| t.span)
                    .chain(result.diagnostics.iter().map(|d| d.span))
                {
                    assert!(text.get(span.start..span.end).is_some());
                }
            }
        }
    }
}
