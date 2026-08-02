use nether_ast::{FileId, Ident, NodeId, NodeIdGen, Path};
use nether_diagnostics::{Diagnostic, Span};
use nether_lexer::{Keyword, Punct, SpannedToken, Token};

/// Holds all mutable parsing state: the token buffer/cursor, the shared
/// [`NodeIdGen`], and accumulated diagnostics.
///
/// `tokens`/`pos` are temporarily swapped out by
/// [`crate::template::Parser::reparse_template_expr`] to re-parse a
/// string-template interpolation segment using this same `Parser` (same
/// `ids` counter, same `diagnostics` sink) without a separate struct or
/// lifetime gymnastics — see that method for why this is safe (sequential,
/// never concurrent).
pub(crate) struct Parser {
    pub(crate) tokens: Vec<SpannedToken>,
    pub(crate) pos: usize,
    pub(crate) file: FileId,
    pub(crate) ids: NodeIdGen,
    pub(crate) diagnostics: Vec<Diagnostic>,
    /// False while parsing an `if`/`while`/`for`/`match` condition or
    /// scrutinee, so a following `{` is parsed as the block/arms rather
    /// than a struct literal — the same ambiguity Rust resolves the same
    /// way. See `parse_no_struct_lit`.
    pub(crate) struct_lit_allowed: bool,
}

impl Parser {
    pub(crate) fn with_node_id_start(tokens: Vec<SpannedToken>, file: FileId, start: u32) -> Self {
        Parser {
            tokens,
            pos: 0,
            file,
            ids: NodeIdGen::with_start(start),
            diagnostics: Vec::new(),
            struct_lit_allowed: true,
        }
    }

    pub(crate) fn next_id(&mut self) -> NodeId {
        self.ids.next_id()
    }

    pub(crate) fn peek(&self) -> &Token {
        &self.tokens[self.pos].token
    }

    pub(crate) fn peek_span(&self) -> Span {
        self.tokens[self.pos].span
    }

    /// Looks ahead `ahead` tokens without consuming; clamps to the last
    /// token (always `Eof`) rather than panicking past the end.
    pub(crate) fn peek_at(&self, ahead: usize) -> &Token {
        let idx = (self.pos + ahead).min(self.tokens.len() - 1);
        &self.tokens[idx].token
    }

    pub(crate) fn is_eof(&self) -> bool {
        matches!(self.peek(), Token::Eof)
    }

    pub(crate) fn bump(&mut self) -> SpannedToken {
        let tok = self.tokens[self.pos].clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    /// The span of the most recently consumed token — used to close off a
    /// composite span after a `bump()` whose result wasn't kept.
    pub(crate) fn prev_span(&self) -> Span {
        self.tokens[self.pos.saturating_sub(1)].span
    }

    pub(crate) fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diagnostics
            .push(Diagnostic::error(message).with_label(span, "here"));
    }

    pub(crate) fn eat_punct(&mut self, p: Punct) -> bool {
        if matches!(self.peek(), Token::Punct(pp) if *pp == p) {
            self.bump();
            true
        } else {
            false
        }
    }

    pub(crate) fn expect_punct(&mut self, p: Punct, ctx: &str) -> Span {
        if matches!(self.peek(), Token::Punct(pp) if *pp == p) {
            return self.bump().span;
        }
        let span = self.peek_span();
        self.error(
            span,
            format!("expected `{}` {ctx}, found {:?}", punct_str(p), self.peek()),
        );
        span
    }

    pub(crate) fn eat_keyword(&mut self, k: nether_lexer::Keyword) -> bool {
        if matches!(self.peek(), Token::Keyword(kk) if *kk == k) {
            self.bump();
            true
        } else {
            false
        }
    }

    pub(crate) fn expect_keyword(&mut self, k: nether_lexer::Keyword) -> Span {
        if matches!(self.peek(), Token::Keyword(kk) if *kk == k) {
            return self.bump().span;
        }
        let span = self.peek_span();
        self.error(span, format!("expected a keyword, found {:?}", self.peek()));
        span
    }

    pub(crate) fn expect_ident(&mut self) -> Ident {
        if let Token::Ident(name) = self.peek().clone() {
            let span = self.bump().span;
            return Ident::new(name, span);
        }
        let span = self.peek_span();
        self.error(
            span,
            format!("expected an identifier, found {:?}", self.peek()),
        );
        Ident::new("<error>", span)
    }

    /// A token that could plausibly start an expression — used to decide
    /// whether `break`/`return` carry a trailing value.
    pub(crate) fn can_start_expr(&self) -> bool {
        !matches!(
            self.peek(),
            Token::Punct(Punct::Semi)
                | Token::Punct(Punct::RBrace)
                | Token::Punct(Punct::RParen)
                | Token::Punct(Punct::RBracket)
                | Token::Punct(Punct::Comma)
                | Token::Eof
        )
    }

    /// Runs `f` with struct-literal parsing suppressed, restoring the
    /// previous setting afterward even if `f` is itself nested inside
    /// another suppressed region.
    pub(crate) fn parse_no_struct_lit<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let prev = self.struct_lit_allowed;
        self.struct_lit_allowed = false;
        let result = f(self);
        self.struct_lit_allowed = prev;
        result
    }

    /// A dotted path of plain identifiers (language-spec §10): `foo`,
    /// `foo.bar`, `foo.bar.baz`. Used wherever the grammar calls for a
    /// name that is unambiguously a full path with no interleaved calls or
    /// indexing — `use` targets, interface names in `impl Type: Iface`,
    /// enum-variant patterns. General expression parsing does its own,
    /// separate leading-path accumulation (see `expr.rs`) because there a
    /// call can interrupt the chain (`Dog.new(x).y`).
    pub(crate) fn parse_path(&mut self) -> Path {
        let id = self.next_id();
        let first = self.expect_ident();
        let mut segments = vec![first];
        while matches!(self.peek(), Token::Punct(Punct::Dot))
            && matches!(self.peek_at(1), Token::Ident(_))
        {
            self.bump();
            segments.push(self.expect_ident());
        }
        let span = segments[0].span.to(segments.last().unwrap().span);
        Path { id, segments, span }
    }

    /// Import paths additionally allow the `self` keyword as their first
    /// segment (`use self.child.Name;`). `super` and `crate` are ordinary
    /// identifiers in Nether and therefore already work here.
    pub(crate) fn parse_use_path(&mut self) -> Path {
        let id = self.next_id();
        let first = if matches!(self.peek(), Token::Keyword(Keyword::SelfLower)) {
            let span = self.bump().span;
            Ident::new("self", span)
        } else {
            self.expect_ident()
        };
        let mut segments = vec![first];
        while matches!(self.peek(), Token::Punct(Punct::Dot))
            && (matches!(self.peek_at(1), Token::Ident(_))
                || matches!(self.peek_at(1), Token::Keyword(Keyword::SelfLower)))
        {
            self.bump();
            let segment = if matches!(self.peek(), Token::Keyword(Keyword::SelfLower)) {
                let span = self.bump().span;
                Ident::new("self", span)
            } else {
                self.expect_ident()
            };
            segments.push(segment);
        }
        let span = segments[0].span.to(segments.last().unwrap().span);
        Path { id, segments, span }
    }
}

fn punct_str(p: Punct) -> &'static str {
    match p {
        Punct::LBrace => "{",
        Punct::RBrace => "}",
        Punct::LParen => "(",
        Punct::RParen => ")",
        Punct::LBracket => "[",
        Punct::RBracket => "]",
        Punct::Comma => ",",
        Punct::Semi => ";",
        Punct::Colon => ":",
        Punct::Dot => ".",
        Punct::DotDotDot => "...",
        Punct::Plus => "+",
        Punct::Minus => "-",
        Punct::Star => "*",
        Punct::Slash => "/",
        Punct::Percent => "%",
        Punct::Eq => "=",
        Punct::EqEq => "==",
        Punct::Ne => "!=",
        Punct::Lt => "<",
        Punct::Le => "<=",
        Punct::Gt => ">",
        Punct::Ge => ">=",
        Punct::Bang => "!",
        Punct::AmpAmp => "&&",
        Punct::PipePipe => "||",
        Punct::FatArrow => "=>",
    }
}
