use nether_diagnostics::{Diagnostic, FileId, Span};

use crate::token::{keyword_from_str, Punct, SpannedToken, TemplatePartTok, Token};

/// Tokenizes one source file. See this crate's module docs for the overall
/// contract; never fails outright — invalid input produces a
/// [`Token::Error`] plus a diagnostic and lexing continues.
pub fn tokenize(source: &str, file: FileId) -> (Vec<SpannedToken>, Vec<Diagnostic>) {
    let mut lexer = Lexer {
        source,
        file,
        pos: 0,
        tokens: Vec::new(),
        diagnostics: Vec::new(),
    };
    lexer.run();
    (lexer.tokens, lexer.diagnostics)
}

struct Lexer<'a> {
    source: &'a str,
    file: FileId,
    pos: u32,
    tokens: Vec<SpannedToken>,
    diagnostics: Vec<Diagnostic>,
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}

fn is_ident_continue(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

impl Lexer<'_> {
    fn peek(&self) -> Option<char> {
        self.source[self.pos as usize..].chars().next()
    }

    fn peek_at(&self, ahead: usize) -> Option<char> {
        self.source[self.pos as usize..].chars().nth(ahead)
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8() as u32;
        Some(c)
    }

    fn span_from(&self, start: u32) -> Span {
        Span::new(self.file, start, self.pos)
    }

    fn run(&mut self) {
        loop {
            self.skip_whitespace();
            let start = self.pos;
            let Some(c) = self.peek() else {
                self.tokens.push(SpannedToken {
                    token: Token::Eof,
                    span: Span::new(self.file, start, start),
                });
                break;
            };
            if c == '/' && self.peek_at(1) == Some('/') {
                self.lex_comment();
                continue;
            }
            if is_ident_start(c) {
                self.lex_ident_or_keyword(start);
                continue;
            }
            if c.is_ascii_digit() {
                self.lex_number(start);
                continue;
            }
            if c == '"' {
                self.lex_plain_string(start);
                continue;
            }
            if c == '`' {
                self.lex_template_string(start);
                continue;
            }
            if c == '\'' {
                self.lex_char(start);
                continue;
            }
            self.lex_punct(start);
        }
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_whitespace()) {
            self.bump();
        }
    }

    fn lex_comment(&mut self) {
        self.bump();
        self.bump(); // consume "//"
        let is_doc = self.peek() == Some('/');
        if is_doc {
            self.bump();
        }
        let text_start = self.pos;
        while let Some(c) = self.peek() {
            if c == '\n' {
                break;
            }
            self.bump();
        }
        if is_doc {
            let text = self.source[text_start as usize..self.pos as usize]
                .trim()
                .to_string();
            let span = self.span_from(text_start);
            self.tokens.push(SpannedToken {
                token: Token::DocComment(text),
                span,
            });
        }
    }

    fn lex_ident_or_keyword(&mut self, start: u32) {
        while matches!(self.peek(), Some(c) if is_ident_continue(c)) {
            self.bump();
        }
        let text = &self.source[start as usize..self.pos as usize];
        let token = match keyword_from_str(text) {
            Some(kw) => Token::Keyword(kw),
            None => Token::Ident(text.to_string()),
        };
        let span = self.span_from(start);
        self.tokens.push(SpannedToken { token, span });
    }

    fn lex_number(&mut self, start: u32) {
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.bump();
        }
        let mut is_float = false;
        if self.peek() == Some('.') && matches!(self.peek_at(1), Some(c) if c.is_ascii_digit()) {
            is_float = true;
            self.bump();
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.bump();
            }
        }
        let text = &self.source[start as usize..self.pos as usize];
        let span = self.span_from(start);
        let token = if is_float {
            match text.parse::<f64>() {
                Ok(v) => Token::Float(v),
                Err(_) => {
                    self.diagnostics.push(
                        Diagnostic::error(format!("invalid float literal `{text}`"))
                            .with_label(span, "here"),
                    );
                    Token::Float(0.0)
                }
            }
        } else {
            match text.parse::<u128>() {
                Ok(v) => Token::Int(v),
                Err(_) => {
                    self.diagnostics.push(
                        Diagnostic::error(format!("integer literal `{text}` out of range"))
                            .with_label(span, "here"),
                    );
                    Token::Int(0)
                }
            }
        };
        self.tokens.push(SpannedToken { token, span });
    }

    /// Shared escape-sequence handling for both plain strings and char
    /// literals: `\n \t \\ \" \'`. An unrecognized escape is reported but
    /// recovers by taking the character literally, so one bad escape
    /// doesn't cascade into "unterminated literal" for the rest of the file.
    fn read_escape(&mut self, opening_span: Span) -> Option<char> {
        match self.bump() {
            Some('n') => Some('\n'),
            Some('t') => Some('\t'),
            Some('\\') => Some('\\'),
            Some('"') => Some('"'),
            Some('\'') => Some('\''),
            Some('`') => Some('`'),
            Some(other) => {
                self.diagnostics.push(
                    Diagnostic::error(format!("unknown escape sequence `\\{other}`"))
                        .with_label(opening_span, "in this literal"),
                );
                Some(other)
            }
            None => None,
        }
    }

    fn lex_plain_string(&mut self, start: u32) {
        self.bump(); // opening quote
        let mut value = String::new();
        loop {
            match self.peek() {
                None => {
                    self.diagnostics.push(
                        Diagnostic::error("unterminated string literal")
                            .with_label(self.span_from(start), "string starts here"),
                    );
                    break;
                }
                Some('"') => {
                    self.bump();
                    break;
                }
                Some('\\') => {
                    self.bump();
                    if let Some(c) = self.read_escape(self.span_from(start)) {
                        value.push(c);
                    }
                }
                Some(c) => {
                    self.bump();
                    value.push(c);
                }
            }
        }
        let span = self.span_from(start);
        self.tokens.push(SpannedToken {
            token: Token::Str(value),
            span,
        });
    }

    fn lex_char(&mut self, start: u32) {
        self.bump(); // opening quote
        let value = match self.peek() {
            Some('\\') => {
                self.bump();
                self.read_escape(self.span_from(start)).unwrap_or('\0')
            }
            Some(c) => {
                self.bump();
                c
            }
            None => {
                self.diagnostics.push(
                    Diagnostic::error("unterminated char literal")
                        .with_label(self.span_from(start), "here"),
                );
                '\0'
            }
        };
        if self.peek() == Some('\'') {
            self.bump();
        } else {
            self.diagnostics.push(
                Diagnostic::error("char literal must contain exactly one character")
                    .with_label(self.span_from(start), "expected closing `'` here"),
            );
        }
        let span = self.span_from(start);
        self.tokens.push(SpannedToken {
            token: Token::Char(value),
            span,
        });
    }

    fn lex_template_string(&mut self, start: u32) {
        self.bump(); // opening backtick
        let mut parts = Vec::new();
        let mut literal = String::new();
        loop {
            match self.peek() {
                None => {
                    self.diagnostics.push(
                        Diagnostic::error("unterminated template string")
                            .with_label(self.span_from(start), "template string starts here"),
                    );
                    break;
                }
                Some('`') => {
                    self.bump();
                    break;
                }
                Some('$') if self.peek_at(1) == Some('{') => {
                    if !literal.is_empty() {
                        parts.push(TemplatePartTok::Literal(std::mem::take(&mut literal)));
                    }
                    self.bump();
                    self.bump(); // consume "${"
                    let expr_start = self.pos;
                    let mut depth = 1u32;
                    while let Some(c) = self.peek() {
                        if c == '{' {
                            depth += 1;
                        } else if c == '}' {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        self.bump();
                    }
                    let expr_span = Span::new(self.file, expr_start, self.pos);
                    let expr_text = self.source[expr_start as usize..self.pos as usize].to_string();
                    if self.peek() == Some('}') {
                        self.bump();
                    } else {
                        self.diagnostics.push(
                            Diagnostic::error("unterminated interpolation")
                                .with_label(expr_span, "expected `}` to close this `${`"),
                        );
                    }
                    parts.push(TemplatePartTok::Expr(expr_text, expr_span));
                }
                Some('\\') => {
                    self.bump();
                    if let Some(c) = self.read_escape(self.span_from(start)) {
                        literal.push(c);
                    }
                }
                Some(c) => {
                    self.bump();
                    literal.push(c);
                }
            }
        }
        if !literal.is_empty() {
            parts.push(TemplatePartTok::Literal(literal));
        }
        let span = self.span_from(start);
        self.tokens.push(SpannedToken {
            token: Token::TemplateStr(parts),
            span,
        });
    }

    fn lex_punct(&mut self, start: u32) {
        let c = self.bump().unwrap();
        let punct = match c {
            '{' => Some(Punct::LBrace),
            '}' => Some(Punct::RBrace),
            '(' => Some(Punct::LParen),
            ')' => Some(Punct::RParen),
            '[' => Some(Punct::LBracket),
            ']' => Some(Punct::RBracket),
            ',' => Some(Punct::Comma),
            ';' => Some(Punct::Semi),
            ':' => Some(Punct::Colon),
            '.' if self.peek() == Some('.') && self.peek_at(1) == Some('.') => {
                self.bump();
                self.bump();
                Some(Punct::DotDotDot)
            }
            '.' => Some(Punct::Dot),
            '+' => Some(Punct::Plus),
            '-' => Some(Punct::Minus),
            '*' => Some(Punct::Star),
            '/' => Some(Punct::Slash),
            '%' => Some(Punct::Percent),
            '=' if self.peek() == Some('=') => {
                self.bump();
                Some(Punct::EqEq)
            }
            '=' if self.peek() == Some('>') => {
                self.bump();
                Some(Punct::FatArrow)
            }
            '=' => Some(Punct::Eq),
            '!' if self.peek() == Some('=') => {
                self.bump();
                Some(Punct::Ne)
            }
            '!' => Some(Punct::Bang),
            '<' if self.peek() == Some('=') => {
                self.bump();
                Some(Punct::Le)
            }
            '<' => Some(Punct::Lt),
            '>' if self.peek() == Some('=') => {
                self.bump();
                Some(Punct::Ge)
            }
            '>' => Some(Punct::Gt),
            '&' if self.peek() == Some('&') => {
                self.bump();
                Some(Punct::AmpAmp)
            }
            '|' if self.peek() == Some('|') => {
                self.bump();
                Some(Punct::PipePipe)
            }
            _ => None,
        };
        let span = self.span_from(start);
        match punct {
            Some(p) => self.tokens.push(SpannedToken {
                token: Token::Punct(p),
                span,
            }),
            None => {
                self.diagnostics.push(
                    Diagnostic::error(format!("unexpected character {c:?}"))
                        .with_label(span, "not valid here"),
                );
                self.tokens.push(SpannedToken {
                    token: Token::Error,
                    span,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::Keyword;
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
        let source = "let a = 1 # 2";
        let file = map.add_file("test.nr", source);
        let (tokens, diags) = tokenize(source, file);
        assert_eq!(diags.len(), 1);
        assert!(tokens.iter().any(|t| t.token == Token::Error));
        // lexing continues past the bad character:
        assert!(tokens.iter().any(|t| t.token == Token::Int(2)));
    }
}
