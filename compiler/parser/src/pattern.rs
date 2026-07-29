use nether_ast::{Ident, Literal, Path, Pattern};
use nether_lexer::{Keyword, Punct, Token};

use crate::parser::Parser;

impl Parser {
    /// Parses a `match` pattern (language-spec §9). No guards in the MVP.
    pub(crate) fn parse_pattern(&mut self) -> Pattern {
        let start = self.peek_span();
        match self.peek().clone() {
            Token::Ident(name) if name == "_" => {
                self.bump();
                Pattern::Wildcard(start)
            }
            Token::Ident(name) => {
                self.bump();
                if matches!(self.peek(), Token::Punct(Punct::Dot)) && matches!(self.peek_at(1), Token::Ident(_)) {
                    let mut segments = vec![Ident::new(name, start)];
                    while matches!(self.peek(), Token::Punct(Punct::Dot)) && matches!(self.peek_at(1), Token::Ident(_))
                    {
                        self.bump();
                        segments.push(self.expect_ident());
                    }
                    let path_span = segments[0].span.to(segments.last().unwrap().span);
                    let path_id = self.next_id();
                    let path = Path { id: path_id, segments, span: path_span };
                    let (payload, end) = if matches!(self.peek(), Token::Punct(Punct::LParen)) {
                        self.parse_variant_payload_pattern()
                    } else {
                        (Vec::new(), path_span)
                    };
                    Pattern::Variant { path, payload, span: path_span.to(end) }
                } else if matches!(self.peek(), Token::Punct(Punct::LParen)) {
                    // A single-segment variant pattern, e.g. bare `Custom(x)`
                    // without a qualifying enum name.
                    let path_id = self.next_id();
                    let path = Path::single(path_id, Ident::new(name, start));
                    let (payload, end) = self.parse_variant_payload_pattern();
                    Pattern::Variant { path, payload, span: start.to(end) }
                } else {
                    let id = self.next_id();
                    Pattern::Binding(id, Ident::new(name, start))
                }
            }
            Token::Int(v) => {
                self.bump();
                Pattern::Literal(Literal::Int(v), start)
            }
            Token::Float(v) => {
                self.bump();
                Pattern::Literal(Literal::Float(v), start)
            }
            Token::Str(s) => {
                self.bump();
                Pattern::Literal(Literal::Str(s), start)
            }
            Token::Char(c) => {
                self.bump();
                Pattern::Literal(Literal::Char(c), start)
            }
            Token::Keyword(Keyword::True) => {
                self.bump();
                Pattern::Literal(Literal::Bool(true), start)
            }
            Token::Keyword(Keyword::False) => {
                self.bump();
                Pattern::Literal(Literal::Bool(false), start)
            }
            Token::Punct(Punct::LParen) => {
                self.bump();
                let mut elems = Vec::new();
                while !matches!(self.peek(), Token::Punct(Punct::RParen)) && !self.is_eof() {
                    elems.push(self.parse_pattern());
                    if !self.eat_punct(Punct::Comma) {
                        break;
                    }
                }
                let end = self.expect_punct(Punct::RParen, "to close a tuple pattern");
                Pattern::Tuple(elems, start.to(end))
            }
            other => {
                let span = self.peek_span();
                self.error(span, format!("expected a pattern, found {other:?}"));
                self.bump();
                Pattern::Wildcard(span)
            }
        }
    }

    fn parse_variant_payload_pattern(&mut self) -> (Vec<Pattern>, nether_diagnostics::Span) {
        self.bump(); // '('
        let mut payload = Vec::new();
        while !matches!(self.peek(), Token::Punct(Punct::RParen)) && !self.is_eof() {
            payload.push(self.parse_pattern());
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        let end = self.expect_punct(Punct::RParen, "to close a variant pattern's payload");
        (payload, end)
    }
}
