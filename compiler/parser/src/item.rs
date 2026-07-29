use nether_ast::{
    EnumDecl, EnumVariant, Field, FnDecl, GenericParam, ImplBlock, InterfaceDecl, Item, Param,
    SelfParam, TypeDecl, TypeDeclKind, UseDecl,
};
use nether_lexer::{Keyword, Punct, Token};

use crate::parser::Parser;

impl Parser {
    pub(crate) fn parse_items_until_eof(&mut self) -> Vec<Item> {
        let mut items = Vec::new();
        while !self.is_eof() {
            match self.parse_item() {
                Some(item) => items.push(item),
                None => self.synchronize_item(),
            }
        }
        items
    }

    fn parse_item(&mut self) -> Option<Item> {
        let doc = self.take_doc_comments();
        match self.peek() {
            Token::Keyword(Keyword::Type) => self.parse_type_decl(doc).map(Item::Type),
            Token::Keyword(Keyword::Impl) => self.parse_impl_block().map(Item::Impl),
            Token::Keyword(Keyword::Enum) => self.parse_enum_decl(doc).map(Item::Enum),
            Token::Keyword(Keyword::Interface) => self.parse_interface_decl(doc).map(Item::Interface),
            Token::Keyword(Keyword::Fn) => self.parse_fn_decl(doc).map(Item::Fn),
            Token::Keyword(Keyword::Use) => self.parse_use_decl().map(Item::Use),
            other => {
                let span = self.peek_span();
                self.error(
                    span,
                    format!(
                        "expected an item (`type`, `impl`, `enum`, `interface`, `fn`, `use`), found {other:?}"
                    ),
                );
                None
            }
        }
    }

    /// Collects consecutive leading `///` doc-comment lines into one
    /// joined string, attached to whichever item follows.
    fn take_doc_comments(&mut self) -> Option<String> {
        let mut lines = Vec::new();
        while let Token::DocComment(text) = self.peek() {
            lines.push(text.clone());
            self.bump();
        }
        if lines.is_empty() {
            None
        } else {
            Some(lines.join("\n"))
        }
    }

    /// Error recovery: skip to the next token that plausibly starts an
    /// item, so one malformed item doesn't hide every diagnostic after it.
    fn synchronize_item(&mut self) {
        while !self.is_eof() {
            if matches!(
                self.peek(),
                Token::Keyword(Keyword::Type)
                    | Token::Keyword(Keyword::Impl)
                    | Token::Keyword(Keyword::Enum)
                    | Token::Keyword(Keyword::Interface)
                    | Token::Keyword(Keyword::Fn)
                    | Token::Keyword(Keyword::Use)
            ) {
                break;
            }
            self.bump();
        }
    }

    fn parse_type_decl(&mut self, doc: Option<String>) -> Option<TypeDecl> {
        let start = self.expect_keyword(Keyword::Type);
        let id = self.next_id();
        let name = self.expect_ident();
        let generics = self.parse_optional_generic_params();
        let kind = match self.peek() {
            Token::Punct(Punct::LBrace) => {
                self.bump();
                let mut fields = Vec::new();
                while !matches!(self.peek(), Token::Punct(Punct::RBrace)) && !self.is_eof() {
                    fields.push(self.parse_field());
                    if !self.eat_punct(Punct::Comma) {
                        break;
                    }
                }
                self.expect_punct(Punct::RBrace, "to close struct fields");
                TypeDeclKind::Struct(fields)
            }
            Token::Punct(Punct::LParen) => {
                self.bump();
                let mut tys = Vec::new();
                while !matches!(self.peek(), Token::Punct(Punct::RParen)) && !self.is_eof() {
                    tys.push(self.parse_type_expr());
                    if !self.eat_punct(Punct::Comma) {
                        break;
                    }
                }
                self.expect_punct(Punct::RParen, "to close tuple-struct fields");
                self.expect_punct(Punct::Semi, "after a tuple-struct declaration");
                TypeDeclKind::TupleStruct(tys)
            }
            Token::Punct(Punct::Semi) => {
                self.bump();
                TypeDeclKind::Unit
            }
            other => {
                let span = self.peek_span();
                self.error(span, format!("expected `{{`, `(`, or `;` after a type name, found {other:?}"));
                TypeDeclKind::Unit
            }
        };
        let end = self.prev_span();
        Some(TypeDecl { id, name, generics, kind, doc, span: start.to(end) })
    }

    fn parse_field(&mut self) -> Field {
        let is_private_kw = self.eat_keyword(Keyword::Private);
        let name = self.expect_ident();
        self.expect_punct(Punct::Colon, "after a field name");
        let ty = self.parse_type_expr();
        let private = is_private_kw || name.is_underscore_private();
        Field { name, ty, private }
    }

    fn parse_impl_block(&mut self) -> Option<ImplBlock> {
        let start = self.expect_keyword(Keyword::Impl);
        let id = self.next_id();
        let target = self.expect_ident();
        let interface = if self.eat_punct(Punct::Colon) { Some(self.parse_type_expr()) } else { None };
        self.expect_punct(Punct::LBrace, "to start an impl body");
        let mut methods = Vec::new();
        while !matches!(self.peek(), Token::Punct(Punct::RBrace)) && !self.is_eof() {
            let doc = self.take_doc_comments();
            match self.parse_method_decl(doc) {
                Some(method) => methods.push(method),
                None => {
                    self.bump();
                }
            }
        }
        let end = self.expect_punct(Punct::RBrace, "to close an impl body");
        Some(ImplBlock { id, target, interface, methods, span: start.to(end) })
    }

    fn parse_enum_decl(&mut self, doc: Option<String>) -> Option<EnumDecl> {
        let start = self.expect_keyword(Keyword::Enum);
        let id = self.next_id();
        let name = self.expect_ident();
        let generics = self.parse_optional_generic_params();
        self.expect_punct(Punct::LBrace, "to start an enum body");
        let mut variants = Vec::new();
        while !matches!(self.peek(), Token::Punct(Punct::RBrace)) && !self.is_eof() {
            variants.push(self.parse_enum_variant());
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        let end = self.expect_punct(Punct::RBrace, "to close an enum body");
        Some(EnumDecl { id, name, generics, variants, doc, span: start.to(end) })
    }

    fn parse_enum_variant(&mut self) -> EnumVariant {
        let name = self.expect_ident();
        let payload = if matches!(self.peek(), Token::Punct(Punct::LParen)) {
            self.bump();
            let mut tys = Vec::new();
            while !matches!(self.peek(), Token::Punct(Punct::RParen)) && !self.is_eof() {
                tys.push(self.parse_type_expr());
                if !self.eat_punct(Punct::Comma) {
                    break;
                }
            }
            self.expect_punct(Punct::RParen, "to close a variant's payload");
            tys
        } else {
            Vec::new()
        };
        let span = payload.last().map(|t| name.span.to(t.span())).unwrap_or(name.span);
        EnumVariant { name, payload, span }
    }

    fn parse_interface_decl(&mut self, doc: Option<String>) -> Option<InterfaceDecl> {
        let start = self.expect_keyword(Keyword::Interface);
        let id = self.next_id();
        let name = self.expect_ident();
        let generics = self.parse_optional_generic_params();
        self.expect_punct(Punct::LBrace, "to start an interface body");
        let mut methods = Vec::new();
        while !matches!(self.peek(), Token::Punct(Punct::RBrace)) && !self.is_eof() {
            let mdoc = self.take_doc_comments();
            match self.parse_method_decl(mdoc) {
                Some(m) => methods.push(m),
                None => {
                    self.bump();
                }
            }
        }
        let end = self.expect_punct(Punct::RBrace, "to close an interface body");
        Some(InterfaceDecl { id, name, generics, methods, doc, span: start.to(end) })
    }

    fn parse_use_decl(&mut self) -> Option<UseDecl> {
        let start = self.expect_keyword(Keyword::Use);
        let id = self.next_id();
        let path = self.parse_path();
        let end = self.expect_punct(Punct::Semi, "after a use declaration");
        Some(UseDecl { id, path, span: start.to(end) })
    }

    /// A standalone `fn` declaration — always has the `fn` keyword and
    /// never a `self` parameter (language-spec §6.1).
    fn parse_fn_decl(&mut self, doc: Option<String>) -> Option<FnDecl> {
        let start = self.expect_keyword(Keyword::Fn);
        let id = self.next_id();
        let name = self.expect_ident();
        let generics = self.parse_optional_generic_params();
        self.expect_punct(Punct::LParen, "to start a parameter list");
        let params = self.parse_params_list();
        self.expect_punct(Punct::RParen, "to close a parameter list");
        let ret = if self.eat_punct(Punct::Colon) { Some(self.parse_type_expr()) } else { None };
        let body = if matches!(self.peek(), Token::Punct(Punct::LBrace)) {
            Some(self.parse_block())
        } else {
            self.error(self.peek_span(), "a standalone `fn` must have a body");
            None
        };
        let end = body
            .as_ref()
            .map(|b| b.span)
            .or_else(|| ret.as_ref().map(|r| r.span()))
            .unwrap_or_else(|| self.prev_span());
        let private = name.is_underscore_private();
        Some(FnDecl { id, name, generics, self_param: None, params, ret, body, private, doc, span: start.to(end) })
    }

    /// A method inside `impl`/`interface` — no `fn` keyword (language-spec
    /// §6), may start with `self`/`mut self`, and may have no body only
    /// when it's an interface method with no default implementation
    /// (language-spec §7).
    pub(crate) fn parse_method_decl(&mut self, doc: Option<String>) -> Option<FnDecl> {
        let start = self.peek_span();
        let is_private_kw = self.eat_keyword(Keyword::Private);
        if !matches!(self.peek(), Token::Ident(_)) {
            let span = self.peek_span();
            self.error(span, format!("expected a method name, found {:?}", self.peek()));
            return None;
        }
        let id = self.next_id();
        let name = self.expect_ident();
        let generics = self.parse_optional_generic_params();
        self.expect_punct(Punct::LParen, "to start a parameter list");
        let self_param = self.parse_optional_self_param();
        let params = self.parse_params_list();
        self.expect_punct(Punct::RParen, "to close a parameter list");
        let ret = if self.eat_punct(Punct::Colon) { Some(self.parse_type_expr()) } else { None };
        let body = if matches!(self.peek(), Token::Punct(Punct::LBrace)) {
            Some(self.parse_block())
        } else {
            self.expect_punct(Punct::Semi, "after a method signature with no default body");
            None
        };
        let end = body
            .as_ref()
            .map(|b| b.span)
            .or_else(|| ret.as_ref().map(|r| r.span()))
            .unwrap_or_else(|| self.prev_span());
        let private = is_private_kw || name.is_underscore_private();
        Some(FnDecl { id, name, generics, self_param, params, ret, body, private, doc, span: start.to(end) })
    }

    fn parse_optional_self_param(&mut self) -> Option<SelfParam> {
        if matches!(self.peek(), Token::Keyword(Keyword::SelfLower)) {
            self.bump();
            self.eat_punct(Punct::Comma);
            Some(SelfParam::ByRef)
        } else if matches!(self.peek(), Token::Keyword(Keyword::Mut))
            && matches!(self.peek_at(1), Token::Keyword(Keyword::SelfLower))
        {
            self.bump();
            self.bump();
            self.eat_punct(Punct::Comma);
            Some(SelfParam::ByMutRef)
        } else {
            None
        }
    }

    fn parse_params_list(&mut self) -> Vec<Param> {
        let mut params = Vec::new();
        while !matches!(self.peek(), Token::Punct(Punct::RParen)) && !self.is_eof() {
            params.push(self.parse_param());
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        params
    }

    pub(crate) fn parse_param(&mut self) -> Param {
        let start = self.peek_span();
        let id = self.next_id();
        let mutable = self.eat_keyword(Keyword::Mut);
        let name = self.expect_ident();
        self.expect_punct(Punct::Colon, "after a parameter name");
        let ty = self.parse_type_expr();
        let span = start.to(ty.span());
        Param { id, name, mutable, ty, span }
    }

    fn parse_optional_generic_params(&mut self) -> Vec<GenericParam> {
        if !self.eat_punct(Punct::Lt) {
            return Vec::new();
        }
        let mut params = Vec::new();
        while !matches!(self.peek(), Token::Punct(Punct::Gt)) && !self.is_eof() {
            let name = self.expect_ident();
            let bound = if self.eat_punct(Punct::Colon) { Some(self.parse_type_expr()) } else { None };
            params.push(GenericParam { name, bound });
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        self.expect_punct(Punct::Gt, "to close a generic parameter list");
        params
    }
}
