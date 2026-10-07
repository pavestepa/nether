use crate::source::{Diagnostic, SourceId, Span};

#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    Word(String),
    Integer(String),
    Float(String),
    String(String),
    Char(char),
    Symbol(&'static str),
    Newline,
    Eof,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

#[derive(Debug)]
pub struct Lexed {
    pub tokens: Vec<Token>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn lex(source: SourceId, text: &str) -> Lexed {
    Lexer {
        source,
        text,
        pos: 0,
        tokens: Vec::new(),
        diagnostics: Vec::new(),
    }
    .run()
}

struct Lexer<'a> {
    source: SourceId,
    text: &'a str,
    pos: usize,
    tokens: Vec<Token>,
    diagnostics: Vec<Diagnostic>,
}

impl Lexer<'_> {
    fn run(mut self) -> Lexed {
        if self.text.starts_with('\u{feff}') {
            self.bump();
        }
        while let Some(c) = self.peek() {
            let start = self.pos;
            match c {
                ' ' | '\t' | '\x0c' => {
                    self.bump();
                }
                '\r' | '\n' => self.newline(),
                '/' if self.rest().starts_with("//") => {
                    while self.peek().is_some_and(|c| c != '\r' && c != '\n') {
                        self.bump();
                    }
                }
                '/' if self.rest().starts_with("/*") => self.comment(),
                'a'..='z' | 'A'..='Z' | '_' => {
                    while self
                        .peek()
                        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
                    {
                        self.bump();
                    }
                    self.emit(
                        start,
                        TokenKind::Word(self.text[start..self.pos].to_owned()),
                    );
                }
                '0'..='9' => self.number(),
                '"' | '\'' => self.quoted(c),
                '`' => {
                    self.bump();
                    while self.peek().is_some_and(|c| c != '`') {
                        self.bump();
                    }
                    if self.peek() == Some('`') {
                        self.bump();
                    }
                    self.error(
                        start,
                        "E0105",
                        "interpolation is not supported in Nether 0.1",
                    );
                }
                _ => {
                    const SYMBOLS: &[&str] = &[
                        "<<=", ">>=", "...", "..=", "=>", "==", "!=", "<=", ">=", "&&", "||", "<<",
                        ">>", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "..", "(", ")", "{",
                        "}", "[", "]", ",", ";", ":", ".", "+", "-", "*", "/", "%", "&", "|", "^",
                        "!", "=", "<", ">", "#", "?",
                    ];
                    if let Some(symbol) = SYMBOLS
                        .iter()
                        .find(|symbol| self.rest().starts_with(**symbol))
                    {
                        self.pos += symbol.len();
                        self.emit(start, TokenKind::Symbol(symbol));
                    } else {
                        self.bump();
                        let code = if c.is_alphabetic() { "E0110" } else { "E0001" };
                        self.error(
                            start,
                            code,
                            "unexpected character; identifiers must use ASCII",
                        );
                    }
                }
            }
        }
        self.emit(self.pos, TokenKind::Eof);
        Lexed {
            tokens: self.tokens,
            diagnostics: self.diagnostics,
        }
    }

    fn rest(&self) -> &str {
        &self.text[self.pos..]
    }
    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }
    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }
    fn span(&self, start: usize) -> Span {
        Span {
            source: self.source,
            start,
            end: self.pos,
        }
    }
    fn emit(&mut self, start: usize, kind: TokenKind) {
        self.tokens.push(Token {
            kind,
            span: self.span(start),
        });
    }
    fn error(&mut self, start: usize, code: &'static str, message: &str) {
        self.diagnostics
            .push(Diagnostic::new(code, message, self.span(start)));
    }

    fn newline(&mut self) {
        let start = self.pos;
        if self.bump() == Some('\r') && self.peek() == Some('\n') {
            self.bump();
        }
        self.emit(start, TokenKind::Newline);
    }

    fn comment(&mut self) {
        let start = self.pos;
        self.pos += 2;
        let mut depth = 1usize;
        while self.peek().is_some() {
            if self.rest().starts_with("/*") {
                self.pos += 2;
                depth += 1;
            } else if self.rest().starts_with("*/") {
                self.pos += 2;
                depth -= 1;
                if depth == 0 {
                    return;
                }
            } else if matches!(self.peek(), Some('\r' | '\n')) {
                self.newline();
            } else {
                self.bump();
            }
        }
        self.error(start, "E0002", "unterminated block comment");
    }

    fn number(&mut self) {
        let start = self.pos;
        let base = if self.rest().starts_with("0x") {
            16
        } else if self.rest().starts_with("0o") {
            8
        } else if self.rest().starts_with("0b") {
            2
        } else {
            10
        };
        if base != 10 {
            self.pos += 2;
            while self
                .peek()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                self.bump();
            }
            if !digits(&self.text[start + 2..self.pos], base) {
                self.error(
                    start,
                    "E0003",
                    "invalid digits or separators in integer literal",
                );
            }
            self.emit(
                start,
                TokenKind::Integer(self.text[start..self.pos].to_owned()),
            );
            return;
        }
        while self.peek().is_some_and(|c| c.is_ascii_digit() || c == '_') {
            self.bump();
        }
        let whole_end = self.pos;
        let mut valid = digits(&self.text[start..whole_end], 10);
        let mut float = false;
        if self.peek() == Some('.')
            && self
                .rest()
                .as_bytes()
                .get(1)
                .is_some_and(u8::is_ascii_digit)
        {
            float = true;
            self.bump();
            let fraction = self.pos;
            while self.peek().is_some_and(|c| c.is_ascii_digit() || c == '_') {
                self.bump();
            }
            valid &= digits(&self.text[fraction..self.pos], 10);
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            float = true;
            self.bump();
            if matches!(self.peek(), Some('+' | '-')) {
                self.bump();
            }
            let exponent = self.pos;
            while self.peek().is_some_and(|c| c.is_ascii_digit() || c == '_') {
                self.bump();
            }
            valid &= digits(&self.text[exponent..self.pos], 10);
        }
        if self
            .peek()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        {
            valid = false;
            while self
                .peek()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                self.bump();
            }
        }
        if !valid {
            self.error(
                start,
                "E0003",
                "invalid numeric literal; suffixes are not supported",
            );
        }
        let spelling = self.text[start..self.pos].to_owned();
        self.emit(
            start,
            if float {
                TokenKind::Float(spelling)
            } else {
                TokenKind::Integer(spelling)
            },
        );
    }

    fn quoted(&mut self, quote: char) {
        let start = self.pos;
        self.bump();
        let mut value = String::new();
        let mut closed = false;
        while let Some(c) = self.peek() {
            if c == quote {
                self.bump();
                closed = true;
                break;
            }
            if matches!(c, '\n' | '\r') {
                break;
            }
            if c == '\\' {
                let escape_start = self.pos;
                self.bump();
                if let Some(decoded) = self.escape() {
                    value.push(decoded);
                } else {
                    self.error(
                        escape_start,
                        "E0004",
                        "invalid escape sequence or Unicode scalar",
                    );
                }
            } else {
                self.bump();
                value.push(c);
            }
        }
        if !closed {
            self.error(
                start,
                "E0005",
                "unterminated literal; raw newlines are forbidden",
            );
        }
        if quote == '"' {
            self.emit(start, TokenKind::String(value));
        } else {
            let mut chars = value.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => self.emit(start, TokenKind::Char(c)),
                _ => self.error(
                    start,
                    "E0006",
                    "char literal must contain exactly one Unicode scalar",
                ),
            }
        }
    }

    fn escape(&mut self) -> Option<char> {
        // Leave line terminators for normal newline recovery.
        if matches!(self.peek(), Some('\r' | '\n')) {
            return None;
        }
        match self.bump()? {
            '\\' => Some('\\'),
            '"' => Some('"'),
            '\'' => Some('\''),
            'n' => Some('\n'),
            'r' => Some('\r'),
            't' => Some('\t'),
            '0' => Some('\0'),
            'x' => {
                let a = self.peek()?.to_digit(16)?;
                self.bump();
                let b = self.peek()?.to_digit(16)?;
                self.bump();
                let value = a * 16 + b;
                (value <= 127).then(|| char::from_u32(value).unwrap())
            }
            'u' => {
                if self.peek() != Some('{') {
                    return None;
                }
                self.bump();
                let mut value = 0u32;
                let mut count = 0;
                while let Some(digit) = self.peek().and_then(|c| c.to_digit(16)) {
                    self.bump();
                    count += 1;
                    if count <= 6 {
                        value = value * 16 + digit;
                    }
                }
                if self.peek() != Some('}') {
                    return None;
                }
                self.bump();
                if count == 0 || count > 6 {
                    return None;
                }
                char::from_u32(value)
            }
            _ => None,
        }
    }
}

fn digits(text: &str, radix: u32) -> bool {
    let mut previous_digit = false;
    for c in text.chars() {
        if c == '_' {
            if !previous_digit {
                return false;
            }
            previous_digit = false;
        } else if c.is_digit(radix) {
            previous_digit = true;
        } else {
            return false;
        }
    }
    previous_digit
}
