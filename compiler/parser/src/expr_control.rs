use nether_ast::{Expr, ExprKind, MatchArm, TemplatePart};
use nether_lexer::{Keyword, Punct, Token};

use crate::parser::Parser;

impl Parser {
    pub(super) fn parse_if_expr(&mut self) -> Expr {
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
                Some(Box::new(Expr {
                    id,
                    kind: ExprKind::Block(block),
                    span,
                }))
            }
        } else {
            None
        };
        let end = else_branch
            .as_ref()
            .map(|e| e.span)
            .unwrap_or(then_branch.span);
        let span = start.to(end);
        let id = self.next_id();
        Expr {
            id,
            kind: ExprKind::If {
                cond: Box::new(cond),
                then_branch,
                else_branch,
            },
            span,
        }
    }

    pub(super) fn parse_match_expr(&mut self) -> Expr {
        let start = self.expect_keyword(Keyword::Match);
        let scrutinee = self.parse_no_struct_lit(|p| p.parse_assign_expr());
        self.expect_punct(Punct::LBrace, "to start a match body");
        let mut arms = Vec::new();
        while !matches!(self.peek(), Token::Punct(Punct::RBrace)) && !self.is_eof() {
            let pattern = self.parse_pattern();
            self.expect_punct(Punct::FatArrow, "after a match pattern");
            let body = self.parse_expr_or_block();
            let arm_span = pattern.span().to(body.span);
            arms.push(MatchArm {
                pattern,
                body,
                span: arm_span,
            });
            self.eat_punct(Punct::Comma);
        }
        let end = self.expect_punct(Punct::RBrace, "to close a match body");
        let span = start.to(end);
        let id = self.next_id();
        Expr {
            id,
            kind: ExprKind::Match {
                scrutinee: Box::new(scrutinee),
                arms,
            },
            span,
        }
    }

    pub(super) fn parse_while_expr(&mut self) -> Expr {
        let start = self.expect_keyword(Keyword::While);
        let cond = self.parse_no_struct_lit(|p| p.parse_assign_expr());
        let body = self.parse_block();
        let span = start.to(body.span);
        let id = self.next_id();
        Expr {
            id,
            kind: ExprKind::While {
                cond: Box::new(cond),
                body,
            },
            span,
        }
    }

    pub(super) fn parse_for_expr(&mut self) -> Expr {
        let start = self.expect_keyword(Keyword::For);
        let pattern = self.parse_pattern();
        self.expect_keyword(Keyword::In);
        let iter = self.parse_no_struct_lit(|p| p.parse_assign_expr());
        let body = self.parse_block();
        let span = start.to(body.span);
        let id = self.next_id();
        Expr {
            id,
            kind: ExprKind::ForIn {
                pattern,
                iter: Box::new(iter),
                body,
            },
            span,
        }
    }

    pub(super) fn parse_loop_expr(&mut self) -> Expr {
        let start = self.expect_keyword(Keyword::Loop);
        let body = self.parse_block();
        let span = start.to(body.span);
        let id = self.next_id();
        Expr {
            id,
            kind: ExprKind::Loop { body },
            span,
        }
    }

    pub(super) fn build_template_expr(
        &mut self,
        parts: Vec<nether_lexer::TemplatePartTok>,
        span: nether_diagnostics::Span,
    ) -> Expr {
        let mut ast_parts = Vec::new();
        for part in parts {
            match part {
                nether_lexer::TemplatePartTok::Literal(s) => {
                    ast_parts.push(TemplatePart::Literal(s))
                }
                nether_lexer::TemplatePartTok::Expr(text, part_span) => {
                    let expr = self.reparse_template_expr(&text, part_span);
                    ast_parts.push(TemplatePart::Expr(expr));
                }
            }
        }
        let id = self.next_id();
        Expr {
            id,
            kind: ExprKind::StringTemplate(ast_parts),
            span,
        }
    }
}
