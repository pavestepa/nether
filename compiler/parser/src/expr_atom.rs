use nether_ast::{Expr, ExprKind, Ident, Literal, Path};
use nether_lexer::{Keyword, Punct, Token};

use crate::parser::Parser;

impl Parser {
    pub(super) fn parse_primary(&mut self) -> Expr {
        let start = self.peek_span();
        match self.peek().clone() {
            Token::Int(v) => {
                self.bump();
                self.literal_expr(Literal::Int(v), start)
            }
            Token::Float(v) => {
                self.bump();
                self.literal_expr(Literal::Float(v), start)
            }
            Token::Char(c) => {
                self.bump();
                self.literal_expr(Literal::Char(c), start)
            }
            Token::Str(s) => {
                self.bump();
                self.literal_expr(Literal::Str(s), start)
            }
            Token::Keyword(Keyword::True) => {
                self.bump();
                self.literal_expr(Literal::Bool(true), start)
            }
            Token::Keyword(Keyword::False) => {
                self.bump();
                self.literal_expr(Literal::Bool(false), start)
            }
            Token::TemplateStr(parts) => {
                self.bump();
                self.build_template_expr(parts, start)
            }
            Token::Keyword(Keyword::SelfLower) => {
                self.bump();
                self.path_expr_from_ident(Ident::new("self", start))
            }
            Token::Ident(name) => {
                self.bump();
                self.path_expr_from_ident(Ident::new(name, start))
            }
            Token::Punct(Punct::LParen) => self.parse_paren_or_closure(start),
            Token::Keyword(Keyword::Move) => {
                self.bump();
                if !matches!(self.peek(), Token::Punct(Punct::LParen)) {
                    self.error(self.peek_span(), "expected `(` after `move`");
                }
                self.parse_closure_with_mode(start, true)
            }
            Token::Punct(Punct::LBracket) => self.parse_array_expr(start),
            Token::Punct(Punct::Colon) => {
                self.bump();
                self.parse_owned_struct_lit(start)
            }
            Token::Keyword(Keyword::If) => self.parse_if_expr(),
            Token::Keyword(Keyword::Match) => self.parse_match_expr(),
            Token::Keyword(Keyword::While) => self.parse_while_expr(),
            Token::Keyword(Keyword::For) => self.parse_for_expr(),
            Token::Keyword(Keyword::Loop) => self.parse_loop_expr(),
            Token::Keyword(Keyword::Break) => {
                self.bump();
                let value = if self.can_start_expr() {
                    Some(Box::new(self.parse_assign_expr()))
                } else {
                    None
                };
                let span = value.as_ref().map(|v| start.to(v.span)).unwrap_or(start);
                let id = self.next_id();
                Expr {
                    id,
                    kind: ExprKind::Break(value),
                    span,
                }
            }
            Token::Keyword(Keyword::Continue) => {
                self.bump();
                let id = self.next_id();
                Expr {
                    id,
                    kind: ExprKind::Continue,
                    span: start,
                }
            }
            Token::Keyword(Keyword::Return) => {
                self.bump();
                let value = if self.can_start_expr() {
                    Some(Box::new(self.parse_assign_expr()))
                } else {
                    None
                };
                let span = value.as_ref().map(|v| start.to(v.span)).unwrap_or(start);
                let id = self.next_id();
                Expr {
                    id,
                    kind: ExprKind::Return(value),
                    span,
                }
            }
            other => {
                self.error(start, format!("expected an expression, found {other:?}"));
                self.bump();
                let id = self.next_id();
                Expr {
                    id,
                    kind: ExprKind::Literal(Literal::Int(0)),
                    span: start,
                }
            }
        }
    }

    pub(super) fn literal_expr(&mut self, lit: Literal, span: nether_diagnostics::Span) -> Expr {
        let id = self.next_id();
        Expr {
            id,
            kind: ExprKind::Literal(lit),
            span,
        }
    }

    /// Builds the primary expression for a leading identifier: greedily
    /// accumulates a dotted [`Path`] (`user.User`, `self.name`), then — if
    /// a struct literal is currently allowed and a `{` follows — parses
    /// `Path { fields }`. Otherwise the bare path becomes an
    /// [`ExprKind::Path`], which [`Parser::parse_postfix`] may go on to
    /// wrap in `Call`/`MethodCall`/`Field`/`Index`.
    pub(super) fn path_expr_from_ident(&mut self, first: Ident) -> Expr {
        let start = first.span;
        let mut segments = vec![first];
        while matches!(self.peek(), Token::Punct(Punct::Dot))
            && matches!(self.peek_at(1), Token::Ident(_))
        {
            self.bump();
            segments.push(self.expect_ident());
        }
        let path_span = start.to(segments.last().unwrap().span);
        let path_id = self.next_id();
        let path = Path {
            id: path_id,
            segments,
            span: path_span,
        };

        if self.struct_lit_allowed && matches!(self.peek(), Token::Punct(Punct::LBrace)) {
            self.parse_struct_lit(path, path_span, false)
        } else {
            let id = self.next_id();
            Expr {
                id,
                kind: ExprKind::Path(path),
                span: path_span,
            }
        }
    }

    /// `Dog { name = "Rex" }` / shorthand `Dog { name }` (language-spec
    /// §11). `owned` marks the `:Dog { ... }` form — the caller has
    /// already consumed the leading `:` and is passing its span folded
    /// into `path_span`'s start where relevant (see
    /// [`Parser::parse_owned_struct_lit`]).
    pub(super) fn parse_struct_lit(
        &mut self,
        path: Path,
        path_span: nether_diagnostics::Span,
        owned: bool,
    ) -> Expr {
        self.bump(); // '{'
        let mut fields = Vec::new();
        while !matches!(self.peek(), Token::Punct(Punct::RBrace)) && !self.is_eof() {
            let name = self.expect_ident();
            let value = if self.eat_punct(Punct::Eq) {
                self.parse_assign_expr()
            } else {
                // shorthand `Dog { name }` = `Dog { name = name }`
                let path_id = self.next_id();
                let expr_id = self.next_id();
                Expr {
                    id: expr_id,
                    kind: ExprKind::Path(Path::single(path_id, name.clone())),
                    span: name.span,
                }
            };
            fields.push((name, value));
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        let end = self.expect_punct(Punct::RBrace, "to close a struct literal");
        let id = self.next_id();
        Expr {
            id,
            kind: ExprKind::StructLit {
                path,
                fields,
                owned,
            },
            span: path_span.to(end),
        }
    }

    /// `:Dog { ... }` — an owned struct literal (language-spec §3, §11).
    /// The leading `:` has already been consumed by the caller; this is
    /// the only expression-position construct where the ownership sigil
    /// appears directly, since it qualifies the literal itself rather than
    /// a use site elsewhere (a `let`/parameter/return type is what
    /// ordinarily carries it — language-spec §3).
    pub(super) fn parse_owned_struct_lit(&mut self, colon_span: nether_diagnostics::Span) -> Expr {
        let first = self.expect_ident();
        let mut segments = vec![first];
        while matches!(self.peek(), Token::Punct(Punct::Dot))
            && matches!(self.peek_at(1), Token::Ident(_))
        {
            self.bump();
            segments.push(self.expect_ident());
        }
        let path_span = segments[0].span.to(segments.last().unwrap().span);
        let path_id = self.next_id();
        let path = Path {
            id: path_id,
            segments,
            span: path_span,
        };
        if matches!(self.peek(), Token::Punct(Punct::LBrace)) {
            let mut lit = self.parse_struct_lit(path, path_span, true);
            lit.span = colon_span.to(lit.span);
            lit
        } else {
            let span = self.peek_span();
            self.error(
                span,
                format!(
                    "expected `{{` to start an owned struct literal, found {:?}",
                    self.peek()
                ),
            );
            let id = self.next_id();
            Expr {
                id,
                kind: ExprKind::StructLit {
                    path,
                    fields: Vec::new(),
                    owned: true,
                },
                span: colon_span.to(path_span),
            }
        }
    }

    pub(super) fn parse_paren_or_closure(&mut self, start: nether_diagnostics::Span) -> Expr {
        if self.looks_like_closure_params() {
            return self.parse_closure(start);
        }
        self.bump(); // '('
        if matches!(self.peek(), Token::Punct(Punct::RParen)) {
            let end = self.bump().span;
            let id = self.next_id();
            return Expr {
                id,
                kind: ExprKind::Tuple(Vec::new()),
                span: start.to(end),
            };
        }
        let first = self.parse_assign_expr();
        if self.eat_punct(Punct::Comma) {
            let mut elems = vec![first];
            while !matches!(self.peek(), Token::Punct(Punct::RParen)) && !self.is_eof() {
                elems.push(self.parse_assign_expr());
                if !self.eat_punct(Punct::Comma) {
                    break;
                }
            }
            let end = self.expect_punct(Punct::RParen, "to close a tuple");
            let id = self.next_id();
            Expr {
                id,
                kind: ExprKind::Tuple(elems),
                span: start.to(end),
            }
        } else {
            let end = self.expect_punct(Punct::RParen, "to close a parenthesized expression");
            let mut inner = first;
            inner.span = start.to(end);
            inner
        }
    }

    /// Scans forward from the current `(` for its matching `)` (tracking
    /// nesting depth) and checks whether `=>` follows — the only
    /// lookahead needed to tell a closure's parameter list apart from a
    /// parenthesized/tuple expression, both of which start with `(`.
    pub(super) fn looks_like_closure_params(&self) -> bool {
        let mut depth = 0i32;
        let mut i = self.pos;
        loop {
            match &self.tokens[i].token {
                Token::Punct(Punct::LParen) => depth += 1,
                Token::Punct(Punct::RParen) => {
                    depth -= 1;
                    if depth == 0 {
                        let next = self.tokens.get(i + 1).map(|t| &t.token);
                        return matches!(next, Some(Token::Punct(Punct::FatArrow)));
                    }
                }
                Token::Eof => return false,
                _ => {}
            }
            i += 1;
        }
    }

    pub(super) fn parse_closure(&mut self, start: nether_diagnostics::Span) -> Expr {
        self.parse_closure_with_mode(start, false)
    }

    fn parse_closure_with_mode(
        &mut self,
        start: nether_diagnostics::Span,
        move_capture: bool,
    ) -> Expr {
        self.expect_punct(Punct::LParen, "to start a closure's parameters");
        let mut params = Vec::new();
        while !matches!(self.peek(), Token::Punct(Punct::RParen)) && !self.is_eof() {
            params.push(self.parse_param());
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        self.expect_punct(Punct::RParen, "to close a closure's parameters");
        self.expect_punct(Punct::FatArrow, "after a closure's parameters");
        let body = self.parse_expr_or_block();
        let span = start.to(body.span);
        let id = self.next_id();
        Expr {
            id,
            kind: ExprKind::Closure {
                move_capture,
                params,
                body: Box::new(body),
            },
            span,
        }
    }

    pub(super) fn parse_array_expr(&mut self, start: nether_diagnostics::Span) -> Expr {
        self.bump(); // '['
        let mut elems = Vec::new();
        while !matches!(self.peek(), Token::Punct(Punct::RBracket)) && !self.is_eof() {
            elems.push(self.parse_assign_expr());
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        let end = self.expect_punct(Punct::RBracket, "to close an array literal");
        let id = self.next_id();
        Expr {
            id,
            kind: ExprKind::Array(elems),
            span: start.to(end),
        }
    }
}
