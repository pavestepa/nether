use nether_diagnostics::{Diagnostic, FileId, Span};

use crate::token::{keyword_from_str, Keyword, Punct, SpannedToken, TemplatePartTok, Token};

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
        paren_bracket_depth: 0,
        pending_field_list: false,
        brace_suppresses_asi: Vec::new(),
        bracket_is_attribute: Vec::new(),
        last_rbracket_was_attribute: false,
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
    /// Net nesting depth of unclosed `(`/`[` — *not* `{`/`}` in general
    /// (see `brace_suppresses_asi`). `skip_whitespace`'s ASI rule only
    /// fires at depth 0: a multi-line parameter list, call argument
    /// list, array/tuple literal, or index expression must never get a
    /// semicolon inserted mid-list just because one line happens to end
    /// in a trigger token.
    paren_bracket_depth: i32,
    /// Set the moment `struct`/`enum` is lexed; consumed by the very
    /// next `{` (pushing `true` onto `brace_suppresses_asi`, marking
    /// that field list) or by a `;`/`(` reached with no `{` in between
    /// (a unit/tuple struct — nothing to consume, cleared without
    /// pushing anything).
    pending_field_list: bool,
    /// One entry per currently-open `{`: whether ASI is suppressed
    /// inside it. `true` for a struct/enum declaration's own field list
    /// (`pending_field_list` consumed here) or anything lexically nested
    /// inside one (inherited from the enclosing level — a field's own
    /// `{T, N}` fixed-array *type*, say); `false` for an ordinary block
    /// (`fn`/`if`/`while`/`for`/`loop`/`unsafe`/closure body) — including
    /// nested ones, which is where ASI's actual value is.
    ///
    /// Deliberately does **not** cover struct-literal (`Dog { ... }`) or
    /// fixed-array-literal (`{1, 2, 3}`) *expression* bodies — nothing
    /// keyword-shaped precedes either one for the lexer to key off of
    /// (an ordinary block's own `{` is lexically identical: consider
    /// `fn foo() RetType { ... }` vs. `RetType { ... }`, an ordinary
    /// struct literal — both are "identifier, then `{`", genuinely
    /// ambiguous without real parser lookahead). A multi-line struct or
    /// fixed-array literal therefore keeps needing a trailing comma
    /// after its last field/element to stay safe (language-spec §2.5) —
    /// `parse_struct_lit`/`parse_array_expr`/`parse_fixed_array_expr`
    /// already tolerate one either way.
    brace_suppresses_asi: Vec<bool>,
    /// One entry per currently-open `[`: was it opened as `#[`, an
    /// attribute (`#[link(name = "m")]`)? Its closing `]` sits between
    /// the attribute and the item it decorates
    /// (`nether_parser::item::parse_item_attributes` expects the item
    /// immediately after), so — unlike an ordinary array-literal/index
    /// `]`, which should — it must never trigger ASI.
    bracket_is_attribute: Vec<bool>,
    /// Whether the token most recently pushed is a `]` that closed an
    /// attribute — consulted by `last_token_can_end_a_statement` instead
    /// of trusting `Token::Punct(RBracket)`'s raw kind alone. Always
    /// reflects the *current* last token: only ever read (never
    /// separately reset) right after checking that the last token really
    /// is `RBracket`, so a stale value from an earlier `]` can't leak in.
    last_rbracket_was_attribute: bool,
}

/// The exact token-kind set [`Lexer::skip_whitespace`]'s ASI rule treats
/// as "this could plausibly be the last token of a statement" — mostly
/// Go's own list (identifiers, literals, `return`/`break`/`continue`,
/// closing `)`/`]`) adapted to Nether's token set: `self` (a bare
/// identifier-shaped keyword) and `true`/`false` (literal-shaped
/// keywords) join the list for the same reason ordinary identifiers and
/// literals are on it.
///
/// Deliberately **not** Go's own full list: Go also triggers after a
/// closing `}`, but unlike Go's grammar, Nether's parser has nowhere
/// that tolerates a stray/empty statement — a `}` closing an ordinary
/// block/function/`impl` body is syntactically identical, at this
/// lexer-only level, to one closing a struct-literal or fixed-array
/// literal *value*, and inserting a semicolon after the former breaks
/// real code (confirmed against `stdlib/array.nr`'s own `impl` block: a
/// synthesized semicolon right after a method body's closing `}` reads,
/// to the item parser, as a stray token between declarations). Omitting
/// `}` entirely is the safe direction to err in — a struct/fixed-array
/// literal ending a `let` statement on its own line still needs an
/// explicit `;`, a smaller, documented gap (language-spec §2.5) rather
/// than a real parse break.
///
/// `]` is listed here unconditionally, but [`Lexer::last_token_can_end_a_statement`]
/// (this function's only caller) special-cases one narrower exception
/// first — a `]` that closed a `#[...]` attribute never triggers,
/// regardless of what this function alone would say — so this list
/// alone doesn't tell the whole story for that one token kind.
fn token_can_end_a_statement(token: &Token) -> bool {
    matches!(
        token,
        Token::Ident(_)
            | Token::Int(_)
            | Token::Float(_)
            | Token::Char(_)
            | Token::Str(_)
            | Token::TemplateStr(_)
            | Token::Keyword(Keyword::True)
            | Token::Keyword(Keyword::False)
            | Token::Keyword(Keyword::SelfLower)
            | Token::Keyword(Keyword::Return)
            | Token::Keyword(Keyword::Break)
            | Token::Keyword(Keyword::Continue)
            | Token::Punct(Punct::RParen)
            | Token::Punct(Punct::RBracket)
    )
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
            if c == '/' && self.peek_at(1) == Some('*') {
                self.lex_block_comment(start);
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

    /// Newline-aware optional semicolons (language-spec §2.5, Stage 7),
    /// Go's own ASI rule ported directly: if the *last emitted* token is
    /// one [`token_can_end_a_statement`] says can plausibly end a
    /// statement, the first `\n` in this whitespace run gets a real,
    /// synthesized `Punct::Semi` token pushed in place — at most one per
    /// run, so a blank line (or several) after a statement doesn't
    /// insert more than one. Every later stage (parser onward) sees this
    /// exactly like a semicolon the user actually typed; nothing past
    /// the lexer needs to know it was implicit.
    ///
    /// Deliberately the *simpler* of the two well-known ASI families
    /// (Go's "insert after specific token kinds," vs. Kotlin/Swift's
    /// more context-sensitive "does the next line look like a valid
    /// continuation" lookahead): it needs no parser changes at all, and
    /// sidesteps JS's classic `return\n(expr)` hazard entirely, since
    /// `return`/`break`/`continue` are themselves trigger tokens — a
    /// newline right after one of them always ends the statement, never
    /// waits to see what follows. The trade-off, also Go's own: a method
    /// chain continued on the next line must *not* start that line with
    /// a leading `.` (`foo\n    .bar()` becomes two statements, the
    /// second an error) — keep `.bar()` on the same line as `foo`, or
    /// terminate explicitly.
    ///
    /// One more suppression, beyond `brace_suppresses_asi`/
    /// `paren_bracket_depth`, is load-bearing here: never insert right
    /// before a closing `}` ([`Self::next_non_whitespace_is_rbrace`]).
    /// Nether is expression-oriented — *any* block's last construct with
    /// no trailing `;` is that block's own value (its "tail expression"),
    /// exactly like Rust's own block-value rule — so inserting a
    /// semicolon there wouldn't just risk a parse error, it would
    /// silently change a program's behavior: `{ `${x}` }` (the template
    /// string is the block's value) and `{ `${x}`; }` (the block's value
    /// is `()`, the string is discarded) are different programs. This
    /// check is what makes it safe to keep `}` out of
    /// `token_can_end_a_statement` *and* still correctly leave a real
    /// statement's own trailing newline alone when a `}` follows it.
    fn skip_whitespace(&mut self) {
        let mut inserted_semi = false;
        while let Some(c) = self.peek() {
            if !c.is_whitespace() {
                break;
            }
            if c == '\n'
                && !inserted_semi
                && self.paren_bracket_depth <= 0
                && !self.brace_suppresses_asi.last().copied().unwrap_or(false)
                && !self.next_non_whitespace_is_rbrace()
                && self.last_token_can_end_a_statement()
            {
                self.tokens.push(SpannedToken {
                    token: Token::Punct(Punct::Semi),
                    span: Span::new(self.file, self.pos, self.pos),
                });
                inserted_semi = true;
            }
            self.bump();
        }
    }

    fn last_token_can_end_a_statement(&self) -> bool {
        match self.tokens.last().map(|t| &t.token) {
            Some(Token::Punct(Punct::RBracket)) => !self.last_rbracket_was_attribute,
            Some(token) => token_can_end_a_statement(token),
            None => false,
        }
    }

    /// Scans forward from the current position through plain whitespace
    /// only (never comments — a trailing `// ...`/`/* ... */` right
    /// before a closing `}` is a narrow, accepted residual gap) to see
    /// whether the next real character is `}`. See [`Self::skip_whitespace`]'s
    /// own docs for why this must suppress ASI regardless of every other
    /// condition.
    fn next_non_whitespace_is_rbrace(&self) -> bool {
        self.source[self.pos as usize..]
            .chars()
            .find(|c| !c.is_whitespace())
            == Some('}')
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

    /// `/* ... */` (language-spec §2.2, Stage 7) — discarded entirely,
    /// exactly like a plain `//` comment; there is no block-comment
    /// equivalent of `///` doc comments. Deliberately **not** nestable
    /// (`/* /* */ */`'s inner `/*` is just more discarded text) — the
    /// simpler C/Go/JS/Kotlin convention, chosen over Rust/Swift's
    /// nesting for this stage since nothing in the existing grammar or
    /// spec calls for nesting and it avoids a second, easily-forgotten
    /// depth-counter code path.
    fn lex_block_comment(&mut self, start: u32) {
        self.bump();
        self.bump(); // consume "/*"
        loop {
            match self.peek() {
                None => {
                    self.diagnostics.push(
                        Diagnostic::error("unterminated block comment")
                            .with_label(self.span_from(start), "comment starts here")
                            .with_hint("add a closing `*/`"),
                    );
                    break;
                }
                Some('*') if self.peek_at(1) == Some('/') => {
                    self.bump();
                    self.bump();
                    break;
                }
                Some(_) => {
                    self.bump();
                }
            }
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
        // `struct`/`enum`'s *next* `{` opens a comma-separated field
        // list, never a statement block — see `brace_suppresses_asi`'s
        // own docs. A unit/tuple struct with no `{}` body at all clears
        // this again on `;`/`(`, in `lex_punct`.
        if matches!(token, Token::Keyword(Keyword::Struct) | Token::Keyword(Keyword::Enum)) {
            self.pending_field_list = true;
        }
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
            '#' => Some(Punct::Hash),
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
            '&' => Some(Punct::Amp),
            '|' if self.peek() == Some('|') => {
                self.bump();
                Some(Punct::PipePipe)
            }
            _ => None,
        };
        let span = self.span_from(start);
        match punct {
            Some(p) => {
                match p {
                    Punct::LParen => {
                        self.paren_bracket_depth += 1;
                        self.pending_field_list = false;
                    }
                    Punct::LBracket => {
                        let is_attribute =
                            matches!(self.tokens.last().map(|t| &t.token), Some(Token::Punct(Punct::Hash)));
                        self.bracket_is_attribute.push(is_attribute);
                        self.paren_bracket_depth += 1;
                        self.pending_field_list = false;
                    }
                    Punct::RParen => self.paren_bracket_depth -= 1,
                    Punct::RBracket => {
                        self.paren_bracket_depth -= 1;
                        self.last_rbracket_was_attribute =
                            self.bracket_is_attribute.pop().unwrap_or(false);
                    }
                    Punct::LBrace => {
                        let suppress = self.pending_field_list
                            || self.brace_suppresses_asi.last().copied().unwrap_or(false);
                        self.brace_suppresses_asi.push(suppress);
                        self.pending_field_list = false;
                    }
                    Punct::RBrace => {
                        self.brace_suppresses_asi.pop();
                    }
                    Punct::Semi => self.pending_field_list = false,
                    _ => {}
                }
                self.tokens.push(SpannedToken {
                    token: Token::Punct(p),
                    span,
                });
            }
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
