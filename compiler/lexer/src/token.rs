use nether_diagnostics::Span;

/// A reserved word. Primitive type names (`i32`, `bool`, ...) are
/// deliberately **not** keywords — they lex as plain [`Token::Ident`] and
/// are recognized as built-in types later, by `resolver`/`typecheck`
/// (`docs/architecture/type-system.md`), consistent with types never being
/// distinguished by string comparison in this crate's own logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keyword {
    Let,
    Mut,
    Type,
    Impl,
    Interface,
    Enum,
    Fn,
    SelfLower,
    Use,
    Mod,
    Private,
    Weak,
    Match,
    If,
    Else,
    While,
    For,
    In,
    Loop,
    Break,
    Continue,
    Return,
    True,
    False,
}

pub fn keyword_from_str(s: &str) -> Option<Keyword> {
    Some(match s {
        "let" => Keyword::Let,
        "mut" => Keyword::Mut,
        "type" => Keyword::Type,
        "impl" => Keyword::Impl,
        "interface" => Keyword::Interface,
        "enum" => Keyword::Enum,
        "fn" => Keyword::Fn,
        "self" => Keyword::SelfLower,
        "use" => Keyword::Use,
        "mod" => Keyword::Mod,
        "private" => Keyword::Private,
        "weak" => Keyword::Weak,
        "match" => Keyword::Match,
        "if" => Keyword::If,
        "else" => Keyword::Else,
        "while" => Keyword::While,
        "for" => Keyword::For,
        "in" => Keyword::In,
        "loop" => Keyword::Loop,
        "break" => Keyword::Break,
        "continue" => Keyword::Continue,
        "return" => Keyword::Return,
        "true" => Keyword::True,
        "false" => Keyword::False,
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Punct {
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Semi,
    Colon,
    Dot,
    /// `...` — a variadic parameter's element type (`args: ...String`).
    DotDotDot,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Eq,
    EqEq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Bang,
    AmpAmp,
    PipePipe,
    /// `=>` — closure bodies (language-spec §11) and match arms (§9).
    FatArrow,
}

/// One segment of a lexed template string (language-spec §2.3). The `Expr`
/// segment carries raw, unparsed source text: `parser` re-invokes
/// [`crate::tokenize`] and its own expression entry point on that text, so
/// this crate never needs to know how to parse an expression.
#[derive(Debug, Clone, PartialEq)]
pub enum TemplatePartTok {
    Literal(String),
    Expr(String, Span),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Ident(String),
    Keyword(Keyword),
    Int(u128),
    Float(f64),
    Char(char),
    /// A plain `"..."` string — never scanned for `${` interpolation.
    Str(String),
    /// A backtick template string, already split into literal/expression
    /// segments.
    TemplateStr(Vec<TemplatePartTok>),
    /// Joined text of a `///` doc comment. Plain `//` comments are
    /// discarded entirely and never reach the token stream.
    DocComment(String),
    Punct(Punct),
    /// Emitted in place of an invalid character alongside a diagnostic —
    /// the lexer never fails outright (see this crate's module docs).
    Error,
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpannedToken {
    pub token: Token,
    pub span: Span,
}
