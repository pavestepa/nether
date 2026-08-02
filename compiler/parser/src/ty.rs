use nether_ast::{Ident, Path, TypeExpr};
use nether_lexer::{Keyword, Punct, Token};

use crate::parser::Parser;

impl Parser {
    /// Parses a type as written (language-spec: `weak T`, `[T]`, `(A, B)`,
    /// `(A, B) => R`, `Named<Generics>`). See `nether_ast::TypeExpr` docs
    /// for why this is unresolved data, not a `type-system.md` `Type`.
    pub(crate) fn parse_type_expr(&mut self) -> TypeExpr {
        let start = self.peek_span();
        match self.peek() {
            Token::Keyword(Keyword::Weak) => {
                self.bump();
                let inner = self.parse_type_expr();
                let span = start.to(inner.span());
                TypeExpr::Weak(Box::new(inner), span)
            }
            Token::Punct(Punct::LBracket) => {
                self.bump();
                let inner = self.parse_type_expr();
                let end = self.expect_punct(Punct::RBracket, "to close array type");
                TypeExpr::Array(Box::new(inner), start.to(end))
            }
            Token::Punct(Punct::LParen) => {
                self.bump();
                let mut elems = Vec::new();
                while !matches!(self.peek(), Token::Punct(Punct::RParen)) && !self.is_eof() {
                    elems.push(self.parse_type_expr());
                    if !self.eat_punct(Punct::Comma) {
                        break;
                    }
                }
                self.expect_punct(
                    Punct::RParen,
                    "to close a tuple or function parameter type list",
                );
                if self.eat_punct(Punct::FatArrow) {
                    let ret = self.parse_type_expr();
                    let span = start.to(ret.span());
                    TypeExpr::Function {
                        params: elems,
                        ret: Box::new(ret),
                        span,
                    }
                } else {
                    let end = self.prev_span();
                    TypeExpr::Tuple(elems, start.to(end))
                }
            }
            Token::Ident(_) => {
                let path = self.parse_path();
                let generics = self.parse_optional_generic_args();
                let span = generics
                    .last()
                    .map(|g| path.span.to(g.span()))
                    .unwrap_or(path.span);
                TypeExpr::Named {
                    path,
                    generics,
                    span,
                }
            }
            _ => {
                let span = self.peek_span();
                self.error(span, format!("expected a type, found {:?}", self.peek()));
                let id = self.next_id();
                TypeExpr::Named {
                    path: Path::single(id, Ident::new("<error>", span)),
                    generics: Vec::new(),
                    span,
                }
            }
        }
    }

    /// Parses `<A, B>` as written after a type name or an `impl` target
    /// (e.g. `Boxed<T>`, `impl<T> Option<T>`) — an empty `Vec` when there is
    /// no `<`.
    pub(crate) fn parse_optional_generic_args(&mut self) -> Vec<TypeExpr> {
        if !self.eat_punct(Punct::Lt) {
            return Vec::new();
        }
        let mut args = Vec::new();
        while !matches!(self.peek(), Token::Punct(Punct::Gt)) && !self.is_eof() {
            args.push(self.parse_type_expr());
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        self.expect_punct(Punct::Gt, "to close a generic argument list");
        args
    }
}
