use nether_ast::{Block, Expr, ExprKind, FieldAccessor, Ident, LetStmt, Stmt, UnaryOp};
use nether_lexer::{Keyword, Punct, Token};

use crate::expr_helpers::{is_block_like, peek_binop};
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
                self.error(
                    span,
                    format!(
                        "expected `;` after this expression, found {:?}",
                        self.peek()
                    ),
                );
                stmts.push(Stmt::Expr(expr));
            }
        }
        let end = self.expect_punct(Punct::RBrace, "to close a block");
        Block {
            stmts,
            tail,
            span: start.to(end),
        }
    }

    /// `let a = 3;` (inferred), `let user User = ...;` (explicit ARC/inline
    /// type — no colon), `let user: User = ...;` (explicit owned type —
    /// colon; language-spec §7). `parse_type_expr` itself consumes the
    /// leading colon when present, so this only decides *whether* a type
    /// was written before the `=`.
    fn parse_let_stmt(&mut self) -> LetStmt {
        let start = self.expect_keyword(Keyword::Let);
        let id = self.next_id();
        let mutable = self.eat_keyword(Keyword::Mut);
        let name = self.expect_ident();
        let ty = if self.can_start_type_expr() {
            Some(self.parse_type_expr())
        } else {
            None
        };
        self.expect_punct(Punct::Eq, "in a `let` binding");
        let value = self.parse_assign_expr();
        let end = self.expect_punct(Punct::Semi, "after a `let` binding");
        LetStmt {
            id,
            mutable,
            name,
            ty,
            value,
            span: start.to(end),
        }
    }

    /// Parses either a bare expression or, if the next token is `{`, a
    /// block used as a value — for the specific positions where a block is
    /// a legitimate expression (closure bodies, match arm bodies) without
    /// reintroducing arbitrary standalone block expressions everywhere
    /// (language-spec §2.6: braces are only ever attached to a known
    /// construct, never a bare primary expression — see
    /// [`Parser::parse_primary`]).
    pub(crate) fn parse_expr_or_block(&mut self) -> Expr {
        if matches!(self.peek(), Token::Punct(Punct::LBrace)) {
            let block = self.parse_block();
            let span = block.span;
            let id = self.next_id();
            Expr {
                id,
                kind: ExprKind::Block(block),
                span,
            }
        } else {
            self.parse_assign_expr()
        }
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
            Expr {
                id,
                kind: ExprKind::Assign {
                    target: Box::new(lhs),
                    value: Box::new(value),
                },
                span,
            }
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
            lhs = Expr {
                id,
                kind: ExprKind::Binary {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
                span,
            };
        }
        lhs
    }

    fn parse_unary(&mut self) -> Expr {
        let start = self.peek_span();
        match self.peek() {
            Token::Keyword(Keyword::Await) => {
                self.bump();
                let expr = self.parse_unary();
                let span = start.to(expr.span);
                Expr {
                    id: self.next_id(),
                    kind: ExprKind::Await(Box::new(expr)),
                    span,
                }
            }
            Token::Punct(Punct::Minus) => {
                self.bump();
                let expr = self.parse_unary();
                let span = start.to(expr.span);
                let id = self.next_id();
                Expr {
                    id,
                    kind: ExprKind::Unary {
                        op: UnaryOp::Neg,
                        expr: Box::new(expr),
                    },
                    span,
                }
            }
            Token::Punct(Punct::Bang) => {
                self.bump();
                let expr = self.parse_unary();
                let span = start.to(expr.span);
                let id = self.next_id();
                Expr {
                    id,
                    kind: ExprKind::Unary {
                        op: UnaryOp::Not,
                        expr: Box::new(expr),
                    },
                    span,
                }
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
                    expr = Expr {
                        id,
                        kind: ExprKind::Call {
                            callee: Box::new(expr),
                            generic_args: Vec::new(),
                            args,
                        },
                        span,
                    };
                }
                Token::Punct(Punct::Lt) => {
                    let Some(generic_args) = self.try_parse_call_generic_args() else {
                        break;
                    };
                    let args = self.parse_call_args();
                    let span = expr.span.to(self.prev_span());
                    let id = self.next_id();
                    expr = Expr {
                        id,
                        kind: ExprKind::Call {
                            callee: Box::new(expr),
                            generic_args,
                            args,
                        },
                        span,
                    };
                }
                Token::Punct(Punct::LBracket) => {
                    self.bump();
                    let index = self.parse_assign_expr();
                    let end = self.expect_punct(Punct::RBracket, "to close an index expression");
                    let span = expr.span.to(end);
                    let id = self.next_id();
                    expr = Expr {
                        id,
                        kind: ExprKind::Index {
                            base: Box::new(expr),
                            index: Box::new(index),
                        },
                        span,
                    };
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
                                kind: ExprKind::Field {
                                    base: Box::new(expr),
                                    field: FieldAccessor::Index(n as u32, idx_span),
                                },
                                span,
                            };
                        }
                        Token::Ident(name) => {
                            let name_span = self.bump().span;
                            let generic_args = self.try_parse_call_generic_args();
                            if matches!(self.peek(), Token::Punct(Punct::LParen)) {
                                let args = self.parse_call_args();
                                let span = expr.span.to(self.prev_span());
                                let id = self.next_id();
                                expr = Expr {
                                    id,
                                    kind: ExprKind::MethodCall {
                                        receiver: Box::new(expr),
                                        method: Ident::new(name, name_span),
                                        generic_args: generic_args.unwrap_or_default(),
                                        args,
                                    },
                                    span,
                                };
                            } else {
                                let span = expr.span.to(name_span);
                                let id = self.next_id();
                                expr = Expr {
                                    id,
                                    kind: ExprKind::Field {
                                        base: Box::new(expr),
                                        field: FieldAccessor::Named(Ident::new(name, name_span)),
                                    },
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

    /// Tries to parse `<Type, ...>` as call-site generic arguments.
    ///
    /// `<` remains an ordinary comparison operator unless the complete
    /// type list is followed by `(`. The parser state, diagnostics, and
    /// NodeId generator are restored on that non-call path.
    fn try_parse_call_generic_args(&mut self) -> Option<Vec<nether_ast::TypeExpr>> {
        if !matches!(self.peek(), Token::Punct(Punct::Lt)) {
            return None;
        }
        let saved_pos = self.pos;
        let saved_ids = self.ids.clone();
        let saved_diagnostics = self.diagnostics.len();

        self.bump();
        let mut args = Vec::new();
        if matches!(self.peek(), Token::Punct(Punct::Gt)) {
            self.error(
                self.peek_span(),
                "an explicit generic argument list cannot be empty",
            );
        } else {
            loop {
                args.push(self.parse_type_expr());
                if !self.eat_punct(Punct::Comma) {
                    break;
                }
            }
        }
        self.expect_punct(Punct::Gt, "to close explicit call type arguments");
        if matches!(self.peek(), Token::Punct(Punct::LParen)) {
            return Some(args);
        }

        self.pos = saved_pos;
        self.ids = saved_ids;
        self.diagnostics.truncate(saved_diagnostics);
        None
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
                args.push(Expr {
                    id,
                    kind: ExprKind::MutArg(Box::new(inner)),
                    span,
                });
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
}
