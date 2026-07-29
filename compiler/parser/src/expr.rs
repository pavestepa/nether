use nether_ast::{
    BinaryOp, Block, Expr, ExprKind, FieldAccessor, Ident, LetStmt, Literal, MatchArm, Path, Stmt,
    TemplatePart, UnaryOp,
};
use nether_lexer::{Keyword, Punct, Token};

use crate::parser::Parser;

impl Parser {
    /// Parses a block `{ stmts... tail? }` (language-spec §2.4: a block is
    /// semicolon-terminated statements followed by an optional
    /// non-semicolon tail expression).
    pub(crate) fn parse_block(&mut self) -> Block {
        let start = self.expect_punct(Punct::LBrace, "to start a block");
        let mut stmts = Vec::new();
        let mut tail = None;
        while !matches!(self.peek(), Token::Punct(Punct::RBrace)) && !self.is_eof() {
            if matches!(self.peek(), Token::Keyword(Keyword::Let)) {
                stmts.push(Stmt::Let(self.parse_let_stmt()));
                continue;
            }
            let expr = self.parse_assign_expr();
            if self.eat_punct(Punct::Semi) {
                stmts.push(Stmt::Expr(expr));
            } else if matches!(self.peek(), Token::Punct(Punct::RBrace)) {
                tail = Some(Box::new(expr));
                break;
            } else if is_block_like(&expr.kind) {
                // `if`/`match`/`while`/`for`/`loop`/`{}` may be used as a
                // statement with no trailing `;`, mirroring Rust.
                stmts.push(Stmt::Expr(expr));
            } else {
                let span = self.peek_span();
                self.error(span, format!("expected `;` after this expression, found {:?}", self.peek()));
                stmts.push(Stmt::Expr(expr));
            }
        }
        let end = self.expect_punct(Punct::RBrace, "to close a block");
        Block { stmts, tail, span: start.to(end) }
    }

    fn parse_let_stmt(&mut self) -> LetStmt {
        let start = self.expect_keyword(Keyword::Let);
        let id = self.next_id();
        let mutable = self.eat_keyword(Keyword::Mut);
        let name = self.expect_ident();
        let ty = if self.eat_punct(Punct::Colon) { Some(self.parse_type_expr()) } else { None };
        self.expect_punct(Punct::Eq, "in a `let` binding");
        let value = self.parse_assign_expr();
        let end = self.expect_punct(Punct::Semi, "after a `let` binding");
        LetStmt { id, mutable, name, ty, value, span: start.to(end) }
    }

    /// Entry point for "a full expression" wherever the grammar wants one
    /// (let-values, block tails/statements, call arguments, array/tuple
    /// elements). Handles `=` (right-associative, lowest precedence) on
    /// top of [`Parser::parse_expr`]'s operator-precedence climbing.
    pub(crate) fn parse_assign_expr(&mut self) -> Expr {
        let lhs = self.parse_expr(0);
        if matches!(self.peek(), Token::Punct(Punct::Eq)) {
            self.bump();
            let value = self.parse_assign_expr();
            let span = lhs.span.to(value.span);
            let id = self.next_id();
            Expr { id, kind: ExprKind::Assign { target: Box::new(lhs), value: Box::new(value) }, span }
        } else {
            lhs
        }
    }

    /// Binary-operator precedence climbing (Pratt parsing). Does not
    /// itself handle `=` — see [`Parser::parse_assign_expr`].
    fn parse_expr(&mut self, min_bp: u8) -> Expr {
        let mut lhs = self.parse_unary();
        while let Some((op, l_bp, r_bp)) = peek_binop(self.peek()) {
            if l_bp < min_bp {
                break;
            }
            self.bump();
            let rhs = self.parse_expr(r_bp);
            let span = lhs.span.to(rhs.span);
            let id = self.next_id();
            lhs = Expr { id, kind: ExprKind::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs) }, span };
        }
        lhs
    }

    fn parse_unary(&mut self) -> Expr {
        let start = self.peek_span();
        match self.peek() {
            Token::Punct(Punct::Minus) => {
                self.bump();
                let expr = self.parse_unary();
                let span = start.to(expr.span);
                let id = self.next_id();
                Expr { id, kind: ExprKind::Unary { op: UnaryOp::Neg, expr: Box::new(expr) }, span }
            }
            Token::Punct(Punct::Bang) => {
                self.bump();
                let expr = self.parse_unary();
                let span = start.to(expr.span);
                let id = self.next_id();
                Expr { id, kind: ExprKind::Unary { op: UnaryOp::Not, expr: Box::new(expr) }, span }
            }
            _ => {
                let primary = self.parse_primary();
                self.parse_postfix(primary)
            }
        }
    }

    fn parse_postfix(&mut self, mut expr: Expr) -> Expr {
        loop {
            match self.peek() {
                Token::Punct(Punct::LParen) => {
                    let args = self.parse_call_args();
                    let span = expr.span.to(self.prev_span());
                    let id = self.next_id();
                    expr = Expr { id, kind: ExprKind::Call { callee: Box::new(expr), args }, span };
                }
                Token::Punct(Punct::LBracket) => {
                    self.bump();
                    let index = self.parse_assign_expr();
                    let end = self.expect_punct(Punct::RBracket, "to close an index expression");
                    let span = expr.span.to(end);
                    let id = self.next_id();
                    expr = Expr { id, kind: ExprKind::Index { base: Box::new(expr), index: Box::new(index) }, span };
                }
                Token::Punct(Punct::Dot) => {
                    self.bump();
                    match self.peek().clone() {
                        Token::Int(n) => {
                            let idx_span = self.bump().span;
                            let span = expr.span.to(idx_span);
                            let id = self.next_id();
                            expr = Expr {
                                id,
                                kind: ExprKind::Field { base: Box::new(expr), field: FieldAccessor::Index(n as u32, idx_span) },
                                span,
                            };
                        }
                        Token::Ident(name) => {
                            let name_span = self.bump().span;
                            if matches!(self.peek(), Token::Punct(Punct::LParen)) {
                                let args = self.parse_call_args();
                                let span = expr.span.to(self.prev_span());
                                let id = self.next_id();
                                expr = Expr {
                                    id,
                                    kind: ExprKind::MethodCall {
                                        receiver: Box::new(expr),
                                        method: Ident::new(name, name_span),
                                        args,
                                    },
                                    span,
                                };
                            } else {
                                let span = expr.span.to(name_span);
                                let id = self.next_id();
                                expr = Expr {
                                    id,
                                    kind: ExprKind::Field { base: Box::new(expr), field: FieldAccessor::Named(Ident::new(name, name_span)) },
                                    span,
                                };
                            }
                        }
                        other => {
                            let span = self.peek_span();
                            self.error(span, format!("expected a field name or tuple index after `.`, found {other:?}"));
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
        expr
    }

    pub(crate) fn parse_call_args(&mut self) -> Vec<Expr> {
        self.expect_punct(Punct::LParen, "to start a call's arguments");
        let mut args = Vec::new();
        while !matches!(self.peek(), Token::Punct(Punct::RParen)) && !self.is_eof() {
            let arg_start = self.peek_span();
            if self.eat_keyword(Keyword::Mut) {
                let inner = self.parse_assign_expr();
                let span = arg_start.to(inner.span);
                let id = self.next_id();
                args.push(Expr { id, kind: ExprKind::MutArg(Box::new(inner)), span });
            } else {
                args.push(self.parse_assign_expr());
            }
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        self.expect_punct(Punct::RParen, "to close a call's arguments");
        args
    }

    fn parse_primary(&mut self) -> Expr {
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
            Token::Punct(Punct::LBracket) => self.parse_array_expr(start),
            Token::Punct(Punct::LBrace) => {
                let block = self.parse_block();
                let span = block.span;
                let id = self.next_id();
                Expr { id, kind: ExprKind::Block(block), span }
            }
            Token::Keyword(Keyword::If) => self.parse_if_expr(),
            Token::Keyword(Keyword::Match) => self.parse_match_expr(),
            Token::Keyword(Keyword::While) => self.parse_while_expr(),
            Token::Keyword(Keyword::For) => self.parse_for_expr(),
            Token::Keyword(Keyword::Loop) => self.parse_loop_expr(),
            Token::Keyword(Keyword::Break) => {
                self.bump();
                let value = if self.can_start_expr() { Some(Box::new(self.parse_assign_expr())) } else { None };
                let span = value.as_ref().map(|v| start.to(v.span)).unwrap_or(start);
                let id = self.next_id();
                Expr { id, kind: ExprKind::Break(value), span }
            }
            Token::Keyword(Keyword::Continue) => {
                self.bump();
                let id = self.next_id();
                Expr { id, kind: ExprKind::Continue, span: start }
            }
            Token::Keyword(Keyword::Return) => {
                self.bump();
                let value = if self.can_start_expr() { Some(Box::new(self.parse_assign_expr())) } else { None };
                let span = value.as_ref().map(|v| start.to(v.span)).unwrap_or(start);
                let id = self.next_id();
                Expr { id, kind: ExprKind::Return(value), span }
            }
            other => {
                self.error(start, format!("expected an expression, found {other:?}"));
                self.bump();
                let id = self.next_id();
                Expr { id, kind: ExprKind::Literal(Literal::Int(0)), span: start }
            }
        }
    }

    fn literal_expr(&mut self, lit: Literal, span: nether_diagnostics::Span) -> Expr {
        let id = self.next_id();
        Expr { id, kind: ExprKind::Literal(lit), span }
    }

    /// Builds the primary expression for a leading identifier: greedily
    /// accumulates a dotted [`Path`] (`user.User`, `self.name`), then — if
    /// a struct literal is currently allowed and a `{` follows — parses
    /// `Path { fields }`. Otherwise the bare path becomes an
    /// [`ExprKind::Path`], which [`Parser::parse_postfix`] may go on to
    /// wrap in `Call`/`MethodCall`/`Field`/`Index`.
    fn path_expr_from_ident(&mut self, first: Ident) -> Expr {
        let start = first.span;
        let mut segments = vec![first];
        while matches!(self.peek(), Token::Punct(Punct::Dot)) && matches!(self.peek_at(1), Token::Ident(_)) {
            self.bump();
            segments.push(self.expect_ident());
        }
        let path_span = start.to(segments.last().unwrap().span);
        let path_id = self.next_id();
        let path = Path { id: path_id, segments, span: path_span };

        if self.struct_lit_allowed && matches!(self.peek(), Token::Punct(Punct::LBrace)) {
            self.parse_struct_lit(path, path_span)
        } else {
            let id = self.next_id();
            Expr { id, kind: ExprKind::Path(path), span: path_span }
        }
    }

    fn parse_struct_lit(&mut self, path: Path, path_span: nether_diagnostics::Span) -> Expr {
        self.bump(); // '{'
        let mut fields = Vec::new();
        while !matches!(self.peek(), Token::Punct(Punct::RBrace)) && !self.is_eof() {
            let name = self.expect_ident();
            let value = if self.eat_punct(Punct::Colon) {
                self.parse_assign_expr()
            } else {
                // shorthand `Dog { name }` = `Dog { name: name }`
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
        Expr { id, kind: ExprKind::StructLit { path, fields }, span: path_span.to(end) }
    }

    fn parse_paren_or_closure(&mut self, start: nether_diagnostics::Span) -> Expr {
        if self.looks_like_closure_params() {
            return self.parse_closure(start);
        }
        self.bump(); // '('
        if matches!(self.peek(), Token::Punct(Punct::RParen)) {
            let end = self.bump().span;
            let id = self.next_id();
            return Expr { id, kind: ExprKind::Tuple(Vec::new()), span: start.to(end) };
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
            Expr { id, kind: ExprKind::Tuple(elems), span: start.to(end) }
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
    fn looks_like_closure_params(&self) -> bool {
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

    fn parse_closure(&mut self, start: nether_diagnostics::Span) -> Expr {
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
        let body = self.parse_assign_expr();
        let span = start.to(body.span);
        let id = self.next_id();
        Expr { id, kind: ExprKind::Closure { params, body: Box::new(body) }, span }
    }

    fn parse_array_expr(&mut self, start: nether_diagnostics::Span) -> Expr {
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
        Expr { id, kind: ExprKind::Array(elems), span: start.to(end) }
    }

    fn parse_if_expr(&mut self) -> Expr {
        let start = self.expect_keyword(Keyword::If);
        let cond = self.parse_no_struct_lit(|p| p.parse_assign_expr());
        let then_branch = self.parse_block();
        let else_branch = if self.eat_keyword(Keyword::Else) {
            if matches!(self.peek(), Token::Keyword(Keyword::If)) {
                Some(Box::new(self.parse_if_expr()))
            } else {
                let block = self.parse_block();
                let span = block.span;
                let id = self.next_id();
                Some(Box::new(Expr { id, kind: ExprKind::Block(block), span }))
            }
        } else {
            None
        };
        let end = else_branch.as_ref().map(|e| e.span).unwrap_or(then_branch.span);
        let span = start.to(end);
        let id = self.next_id();
        Expr { id, kind: ExprKind::If { cond: Box::new(cond), then_branch, else_branch }, span }
    }

    fn parse_match_expr(&mut self) -> Expr {
        let start = self.expect_keyword(Keyword::Match);
        let scrutinee = self.parse_no_struct_lit(|p| p.parse_assign_expr());
        self.expect_punct(Punct::LBrace, "to start a match body");
        let mut arms = Vec::new();
        while !matches!(self.peek(), Token::Punct(Punct::RBrace)) && !self.is_eof() {
            let pattern = self.parse_pattern();
            self.expect_punct(Punct::FatArrow, "after a match pattern");
            let body = self.parse_assign_expr();
            let arm_span = pattern.span().to(body.span);
            arms.push(MatchArm { pattern, body, span: arm_span });
            self.eat_punct(Punct::Comma);
        }
        let end = self.expect_punct(Punct::RBrace, "to close a match body");
        let span = start.to(end);
        let id = self.next_id();
        Expr { id, kind: ExprKind::Match { scrutinee: Box::new(scrutinee), arms }, span }
    }

    fn parse_while_expr(&mut self) -> Expr {
        let start = self.expect_keyword(Keyword::While);
        let cond = self.parse_no_struct_lit(|p| p.parse_assign_expr());
        let body = self.parse_block();
        let span = start.to(body.span);
        let id = self.next_id();
        Expr { id, kind: ExprKind::While { cond: Box::new(cond), body }, span }
    }

    fn parse_for_expr(&mut self) -> Expr {
        let start = self.expect_keyword(Keyword::For);
        let pattern = self.parse_pattern();
        self.expect_keyword(Keyword::In);
        let iter = self.parse_no_struct_lit(|p| p.parse_assign_expr());
        let body = self.parse_block();
        let span = start.to(body.span);
        let id = self.next_id();
        Expr { id, kind: ExprKind::ForIn { pattern, iter: Box::new(iter), body }, span }
    }

    fn parse_loop_expr(&mut self) -> Expr {
        let start = self.expect_keyword(Keyword::Loop);
        let body = self.parse_block();
        let span = start.to(body.span);
        let id = self.next_id();
        Expr { id, kind: ExprKind::Loop { body }, span }
    }

    fn build_template_expr(&mut self, parts: Vec<nether_lexer::TemplatePartTok>, span: nether_diagnostics::Span) -> Expr {
        let mut ast_parts = Vec::new();
        for part in parts {
            match part {
                nether_lexer::TemplatePartTok::Literal(s) => ast_parts.push(TemplatePart::Literal(s)),
                nether_lexer::TemplatePartTok::Expr(text, part_span) => {
                    let expr = self.reparse_template_expr(&text, part_span);
                    ast_parts.push(TemplatePart::Expr(expr));
                }
            }
        }
        let id = self.next_id();
        Expr { id, kind: ExprKind::StringTemplate(ast_parts), span }
    }
}

fn is_block_like(kind: &ExprKind) -> bool {
    matches!(
        kind,
        ExprKind::If { .. }
            | ExprKind::Match { .. }
            | ExprKind::While { .. }
            | ExprKind::ForIn { .. }
            | ExprKind::Loop { .. }
            | ExprKind::Block(_)
    )
}

fn peek_binop(token: &Token) -> Option<(BinaryOp, u8, u8)> {
    let op = match token {
        Token::Punct(Punct::PipePipe) => BinaryOp::Or,
        Token::Punct(Punct::AmpAmp) => BinaryOp::And,
        Token::Punct(Punct::EqEq) => BinaryOp::Eq,
        Token::Punct(Punct::Ne) => BinaryOp::Ne,
        Token::Punct(Punct::Lt) => BinaryOp::Lt,
        Token::Punct(Punct::Le) => BinaryOp::Le,
        Token::Punct(Punct::Gt) => BinaryOp::Gt,
        Token::Punct(Punct::Ge) => BinaryOp::Ge,
        Token::Punct(Punct::Plus) => BinaryOp::Add,
        Token::Punct(Punct::Minus) => BinaryOp::Sub,
        Token::Punct(Punct::Star) => BinaryOp::Mul,
        Token::Punct(Punct::Slash) => BinaryOp::Div,
        Token::Punct(Punct::Percent) => BinaryOp::Rem,
        _ => return None,
    };
    let bp = match op {
        BinaryOp::Or => 1,
        BinaryOp::And => 2,
        BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => 3,
        BinaryOp::Add | BinaryOp::Sub => 4,
        BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => 5,
    };
    Some((op, bp, bp + 1))
}
