use nether_ast::{
    EnumDecl, EnumVariant, Field, FnDecl, GenericParam, ImplBlock, Item, ModDecl, Param, SelfParam,
    StructDecl, StructDeclKind, TraitDecl, TypeAliasDecl, UseDecl, Visibility,
};
use nether_diagnostics::Span;
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
        let start = self.peek_span();
        let allow_pascal_case = self.parse_item_attributes();
        let visibility = if self.eat_keyword(Keyword::Pub) {
            Visibility::Public
        } else {
            Visibility::Private
        };
        if allow_pascal_case && !matches!(self.peek(), Token::Keyword(Keyword::Type)) {
            self.error(
                start,
                "`allow_pascal_case` is only valid on a type alias declaration",
            );
        }
        match self.peek() {
            Token::Keyword(Keyword::Struct) => self
                .parse_struct_decl(doc, visibility, start)
                .map(Item::Struct),
            Token::Keyword(Keyword::Type) => self
                .parse_type_alias_decl(doc, visibility, start, allow_pascal_case)
                .map(Item::TypeAlias),
            Token::Keyword(Keyword::Impl) => self.parse_impl_block().map(Item::Impl),
            Token::Keyword(Keyword::Enum) => {
                self.parse_enum_decl(doc, visibility, start).map(Item::Enum)
            }
            Token::Keyword(Keyword::Trait) => self
                .parse_trait_decl(doc, visibility, start)
                .map(Item::Trait),
            Token::Keyword(Keyword::Fn) => self.parse_fn_decl(doc, visibility, start).map(Item::Fn),
            Token::Keyword(Keyword::Use) => self.parse_use_decl(visibility, start).map(Item::Use),
            Token::Keyword(Keyword::Mod) => self.parse_mod_decl(visibility, start).map(Item::Mod),
            other => {
                let span = self.peek_span();
                self.error(
                    span,
                    format!(
                        "expected an item (`struct`, `type`, `impl`, `enum`, `trait`, `fn`, `use`, `mod`), found {other:?}"
                    ),
                );
                None
            }
        }
    }

    /// Stage 3's first item attribute. Attributes are parsed centrally so
    /// unknown names and use on the wrong item receive a focused diagnostic
    /// rather than cascading into "expected an item" errors.
    fn parse_item_attributes(&mut self) -> bool {
        let mut allow_pascal_case = false;
        while self.eat_punct(Punct::Hash) {
            let attribute_start = self.prev_span();
            self.expect_punct(Punct::LBracket, "after `#` in an item attribute");
            let name = self.expect_ident();
            self.expect_punct(Punct::RBracket, "to close an item attribute");
            if name.name.as_str() == "allow_pascal_case" {
                if allow_pascal_case {
                    self.error(
                        attribute_start.to(name.span),
                        "duplicate `allow_pascal_case` attribute",
                    );
                }
                allow_pascal_case = true;
            } else {
                self.error(name.span, format!("unknown item attribute `{}`", name.name));
            }
        }
        allow_pascal_case
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
                Token::Keyword(Keyword::Struct)
                    | Token::Keyword(Keyword::Type)
                    | Token::Keyword(Keyword::Impl)
                    | Token::Keyword(Keyword::Enum)
                    | Token::Keyword(Keyword::Trait)
                    | Token::Keyword(Keyword::Fn)
                    | Token::Keyword(Keyword::Use)
                    | Token::Keyword(Keyword::Mod)
                    | Token::Keyword(Keyword::Pub)
                    | Token::Punct(Punct::Hash)
            ) {
                break;
            }
            self.bump();
        }
    }

    fn parse_struct_decl(
        &mut self,
        doc: Option<String>,
        visibility: Visibility,
        start: Span,
    ) -> Option<StructDecl> {
        self.expect_keyword(Keyword::Struct);
        let id = self.next_id();
        let name = self.expect_ident();
        let generics = self.parse_optional_generic_params();
        let traits = self.parse_trait_list();
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
                StructDeclKind::Struct(fields)
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
                StructDeclKind::TupleStruct(tys)
            }
            Token::Punct(Punct::Semi) => {
                self.bump();
                StructDeclKind::Unit
            }
            other => {
                let span = self.peek_span();
                self.error(
                    span,
                    format!("expected `{{`, `(`, or `;` after a struct name, found {other:?}"),
                );
                StructDeclKind::Unit
            }
        };
        let end = self.prev_span();
        Some(StructDecl {
            id,
            name,
            visibility,
            generics,
            traits,
            kind,
            doc,
            span: start.to(end),
        })
    }

    /// `type color = (u32, u32, u32);` (language-spec §4.3). The
    /// ownership-qualified form (`type: Name = ...`) is recognized just
    /// far enough to report that it isn't supported yet, then recovers by
    /// parsing the rest as an ordinary alias rather than cascading further
    /// diagnostics.
    fn parse_type_alias_decl(
        &mut self,
        doc: Option<String>,
        visibility: Visibility,
        start: Span,
        allow_pascal_case: bool,
    ) -> Option<TypeAliasDecl> {
        self.expect_keyword(Keyword::Type);
        if matches!(self.peek(), Token::Punct(Punct::Colon)) {
            let span = self.peek_span();
            self.error(
                span,
                "ownership-qualified alias declarations (`type: Name = ...`) are not yet supported",
            );
            self.bump();
        }
        let id = self.next_id();
        let name = self.expect_ident();
        self.expect_punct(Punct::Eq, "after a type alias name");
        let ty = self.parse_type_expr();
        let end = self.expect_punct(Punct::Semi, "after a type alias declaration");
        Some(TypeAliasDecl {
            id,
            name,
            visibility,
            allow_pascal_case,
            ty,
            doc,
            span: start.to(end),
        })
    }

    /// A struct field — `name Type` (language-spec §4.2; no colon, unlike
    /// the pre-rewrite MVP's `name: Type`).
    fn parse_field(&mut self) -> Field {
        let visibility = if self.eat_keyword(Keyword::Pub) {
            Visibility::Public
        } else {
            Visibility::Private
        };
        let name = self.expect_ident();
        let ty = self.parse_type_expr();
        Field {
            name,
            ty,
            visibility,
        }
    }

    fn parse_impl_block(&mut self) -> Option<ImplBlock> {
        let start = self.expect_keyword(Keyword::Impl);
        let id = self.next_id();
        let generics = self.parse_optional_generic_params();
        let target = self.expect_ident();
        let target_args = self.parse_optional_generic_args();
        let traits = self.parse_trait_list();
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
        Some(ImplBlock {
            id,
            generics,
            target,
            target_args,
            traits,
            methods,
            span: start.to(end),
        })
    }

    fn parse_enum_decl(
        &mut self,
        doc: Option<String>,
        visibility: Visibility,
        start: Span,
    ) -> Option<EnumDecl> {
        self.expect_keyword(Keyword::Enum);
        let id = self.next_id();
        let name = self.expect_ident();
        let generics = self.parse_optional_generic_params();
        let traits = self.parse_trait_list();
        self.expect_punct(Punct::LBrace, "to start an enum body");
        let mut variants = Vec::new();
        while !matches!(self.peek(), Token::Punct(Punct::RBrace)) && !self.is_eof() {
            variants.push(self.parse_enum_variant());
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        let end = self.expect_punct(Punct::RBrace, "to close an enum body");
        Some(EnumDecl {
            id,
            name,
            visibility,
            generics,
            traits,
            variants,
            doc,
            span: start.to(end),
        })
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
        let span = payload
            .last()
            .map(|t| name.span.to(t.span()))
            .unwrap_or(name.span);
        EnumVariant {
            name,
            payload,
            span,
        }
    }

    fn parse_trait_decl(
        &mut self,
        doc: Option<String>,
        visibility: Visibility,
        start: Span,
    ) -> Option<TraitDecl> {
        self.expect_keyword(Keyword::Trait);
        let id = self.next_id();
        let name = self.expect_ident();
        let generics = self.parse_optional_generic_params();
        let parents = self.parse_trait_list();
        self.expect_punct(Punct::LBrace, "to start a trait body");
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
        let end = self.expect_punct(Punct::RBrace, "to close a trait body");
        Some(TraitDecl {
            id,
            name,
            visibility,
            generics,
            parents,
            methods,
            doc,
            span: start.to(end),
        })
    }

    /// `struct Dog Sound, Clone { ... }` / `impl Dog Sound, Clone { ... }`
    /// / `trait Child Parent1, Parent2 { ... }` — a trait list has no
    /// leading colon (`:` is reserved for the unique-ownership domain,
    /// language-spec §3); it's simply zero or more comma-separated trait
    /// names directly after the declaration head. Unambiguous because a
    /// declaration body always starts with `{`/`(`/`;`, never a bare
    /// identifier — so a leading `Ident` here can only be a trait name.
    fn parse_trait_list(&mut self) -> Vec<nether_ast::TypeExpr> {
        if !matches!(self.peek(), Token::Ident(_)) {
            return Vec::new();
        }
        let mut traits = vec![self.parse_type_expr()];
        while self.eat_punct(Punct::Comma) {
            traits.push(self.parse_type_expr());
        }
        traits
    }

    fn parse_use_decl(&mut self, visibility: Visibility, start: Span) -> Option<UseDecl> {
        self.expect_keyword(Keyword::Use);
        let id = self.next_id();
        let path = self.parse_use_path();
        let end = self.expect_punct(Punct::Semi, "after a use declaration");
        Some(UseDecl {
            id,
            visibility,
            path,
            span: start.to(end),
        })
    }

    fn parse_mod_decl(&mut self, visibility: Visibility, start: Span) -> Option<ModDecl> {
        self.expect_keyword(Keyword::Mod);
        let id = self.next_id();
        let name = self.expect_ident();
        let end = self.expect_punct(Punct::Semi, "after a module declaration");
        Some(ModDecl {
            id,
            name,
            visibility,
            span: start.to(end),
        })
    }

    /// A standalone `fn` declaration — always has the `fn` keyword and
    /// never a `self` parameter (language-spec §6.1).
    fn parse_fn_decl(
        &mut self,
        doc: Option<String>,
        visibility: Visibility,
        start: Span,
    ) -> Option<FnDecl> {
        self.expect_keyword(Keyword::Fn);
        let id = self.next_id();
        let name = self.expect_ident();
        let generics = self.parse_optional_generic_params();
        self.expect_punct(Punct::LParen, "to start a parameter list");
        let params = self.parse_params_list();
        self.expect_punct(Punct::RParen, "to close a parameter list");
        let ret = self.parse_optional_return_type();
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
        Some(FnDecl {
            id,
            name,
            visibility,
            generics,
            self_param: None,
            params,
            ret,
            body,
            doc,
            span: start.to(end),
        })
    }

    /// A method inside `impl`/`trait` — no `fn` keyword (language-spec
    /// §6), may start with `self`/`mut self`, and may have no body only
    /// when it's a trait method with no default implementation
    /// (language-spec §7).
    pub(crate) fn parse_method_decl(&mut self, doc: Option<String>) -> Option<FnDecl> {
        let start = self.peek_span();
        let visibility = if self.eat_keyword(Keyword::Pub) {
            Visibility::Public
        } else {
            Visibility::Private
        };
        if !matches!(self.peek(), Token::Ident(_)) {
            let span = self.peek_span();
            self.error(
                span,
                format!("expected a method name, found {:?}", self.peek()),
            );
            return None;
        }
        let id = self.next_id();
        let name = self.expect_ident();
        let generics = self.parse_optional_generic_params();
        self.expect_punct(Punct::LParen, "to start a parameter list");
        let self_param = self.parse_optional_self_param();
        let params = self.parse_params_list();
        self.expect_punct(Punct::RParen, "to close a parameter list");
        let ret = self.parse_optional_return_type();
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
        Some(FnDecl {
            id,
            name,
            visibility,
            generics,
            self_param,
            params,
            ret,
            body,
            doc,
            span: start.to(end),
        })
    }

    /// A return type is either absent (next token starts the body or, for
    /// a bodiless trait method, `;`), an ARC/inline type with no
    /// leading colon (`fn f() Type`), or an owned type with one
    /// (`fn f(): Type`) — language-spec §8.2. `parse_type_expr` itself
    /// consumes that leading colon when present, so this only needs to
    /// decide *whether* a type follows at all.
    fn parse_optional_return_type(&mut self) -> Option<nether_ast::TypeExpr> {
        if self.can_start_type_expr() {
            Some(self.parse_type_expr())
        } else {
            None
        }
    }

    /// Whether the upcoming tokens can start a [`TypeExpr`](nether_ast::TypeExpr)
    /// — shared by return-type parsing here and `let`-binding type
    /// parsing (`expr.rs`), since both distinguish "no type written" from
    /// "a type follows" the same way (language-spec §7/§8.2).
    pub(crate) fn can_start_type_expr(&self) -> bool {
        matches!(
            self.peek(),
            Token::Punct(Punct::Colon)
                | Token::Keyword(Keyword::Weak)
                | Token::Punct(Punct::LBracket)
                | Token::Punct(Punct::LParen)
                | Token::Ident(_)
        )
    }

    /// A method receiver — `self`/`mut self` (ARC domain) or `: self`/
    /// `: &self`/`: &mut self` (unique-ownership domain; language-spec
    /// §8.4). The leading-colon forms are unambiguous against an ordinary
    /// first parameter: every plain parameter starts with an identifier
    /// name, never a bare `:`.
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
        } else if matches!(self.peek(), Token::Punct(Punct::Colon)) {
            self.bump();
            let result = if matches!(self.peek(), Token::Keyword(Keyword::SelfLower)) {
                self.bump();
                Some(SelfParam::Owned)
            } else if self.eat_punct(Punct::Amp) {
                let mutable = self.eat_keyword(Keyword::Mut);
                if matches!(self.peek(), Token::Keyword(Keyword::SelfLower)) {
                    self.bump();
                } else {
                    let span = self.peek_span();
                    self.error(span, format!("expected `self`, found {:?}", self.peek()));
                }
                Some(if mutable {
                    SelfParam::OwnedMutRef
                } else {
                    SelfParam::OwnedRef
                })
            } else {
                let span = self.peek_span();
                self.error(
                    span,
                    format!(
                        "expected `self`, `&self`, or `&mut self` after `:`, found {:?}",
                        self.peek()
                    ),
                );
                None
            };
            self.eat_punct(Punct::Comma);
            result
        } else {
            None
        }
    }

    fn parse_params_list(&mut self) -> Vec<Param> {
        let mut params = Vec::new();
        while !matches!(self.peek(), Token::Punct(Punct::RParen)) && !self.is_eof() {
            let param = self.parse_param();
            if param.variadic && !matches!(self.peek(), Token::Punct(Punct::RParen)) {
                self.error(
                    param.span,
                    "a variadic parameter (`...Type`) must be the last parameter",
                );
            }
            params.push(param);
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        params
    }

    /// One of the five parameter forms (language-spec §8.1):
    /// `a Animal` (ordinary ARC), `b mut Animal` (ARC + mutation
    /// permission — note `mut` comes *after* the name here, unlike
    /// `let mut`), `c: Animal` (owned), `d: &Animal` / `e: &mut Animal`
    /// (borrows), and `items ...Type` (variadic — no colon). `mutable`/
    /// `variadic` are recorded on `Param` directly; the owned/ref/mut-ref
    /// distinction lives entirely in `ty`'s shape
    /// (`Unique`/`Ref`/`MutRef`), since `parse_type_expr` already consumes
    /// a leading `:` itself.
    pub(crate) fn parse_param(&mut self) -> Param {
        let start = self.peek_span();
        let id = self.next_id();
        let name = self.expect_ident();
        let mutable = self.eat_keyword(Keyword::Mut);
        let variadic = self.eat_punct(Punct::DotDotDot);
        let ty = self.parse_type_expr();
        let span = start.to(ty.span());
        Param {
            id,
            name,
            mutable,
            variadic,
            ty,
            span,
        }
    }

    fn parse_optional_generic_params(&mut self) -> Vec<GenericParam> {
        if !self.eat_punct(Punct::Lt) {
            return Vec::new();
        }
        let mut params = Vec::new();
        while !matches!(self.peek(), Token::Punct(Punct::Gt)) && !self.is_eof() {
            let name = self.expect_ident();
            let bound = if self.eat_punct(Punct::Colon) {
                Some(self.parse_type_expr())
            } else {
                None
            };
            params.push(GenericParam { name, bound });
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        self.expect_punct(Punct::Gt, "to close a generic parameter list");
        params
    }
}
