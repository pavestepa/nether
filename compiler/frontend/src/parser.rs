mod expression;
mod item;
mod pattern;
mod types;

use crate::{
    ast::*,
    lexer::{lex, Token, TokenKind},
    source::{Diagnostic, SourceId, Span},
};

type Result<T> = std::result::Result<T, Diagnostic>;

pub struct Parsed {
    pub module: Option<Module>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn parse(source: SourceId, text: &str) -> Parsed {
    let lexed = lex(source, text);
    if !lexed.diagnostics.is_empty() {
        return Parsed {
            module: None,
            diagnostics: lexed.diagnostics,
        };
    }
    let last = lexed.tokens[0].span;
    let mut parser = Parser {
        tokens: lexed.tokens,
        pos: 0,
        depth: 0,
        last,
        edits: Vec::new(),
    };
    match parser.module() {
        Ok(module) => Parsed {
            module: Some(module),
            diagnostics: Vec::new(),
        },
        Err(error) => Parsed {
            module: None,
            diagnostics: vec![error],
        },
    }
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    depth: usize,
    last: Span,
    edits: Vec<(usize, Token)>,
}

impl Parser {
    fn module(&mut self) -> Result<Module> {
        let mut items = Vec::new();
        self.separators();
        while !self.at_eof() {
            items.push(self.item()?);
            self.separators();
        }
        Ok(Module { items })
    }

    fn current(&self) -> &Token {
        &self.tokens[self.pos]
    }
    fn at_eof(&self) -> bool {
        self.current().kind == TokenKind::Eof
    }
    fn at(&self, symbol: &str) -> bool {
        matches!(&self.current().kind, TokenKind::Symbol(s) if *s == symbol)
    }
    fn word(&self, word: &str) -> bool {
        matches!(&self.current().kind, TokenKind::Word(s) if s == word)
    }
    fn newline(&self) -> bool {
        self.current().kind == TokenKind::Newline
    }
    fn bump(&mut self) -> Token {
        let token = self.current().clone();
        self.last = token.span;
        if !self.at_eof() {
            self.pos += 1;
        }
        token
    }
    fn eat(&mut self, symbol: &str) -> bool {
        if self.at(symbol) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn eat_word(&mut self, word: &str) -> bool {
        if self.word(word) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn lines(&mut self) {
        while self.newline() {
            self.bump();
        }
    }
    fn separators(&mut self) {
        while self.newline() || self.at(";") {
            self.bump();
        }
    }
    fn error(&self, message: impl Into<String>) -> Diagnostic {
        Diagnostic::new("E0200", message, self.current().span)
    }
    fn expect(&mut self, symbol: &str) -> Result<Span> {
        if self.eat(symbol) {
            Ok(self.last)
        } else {
            Err(self.error(format!("expected '{symbol}'")))
        }
    }
    fn expect_word(&mut self, word: &str) -> Result<Span> {
        if self.eat_word(word) {
            Ok(self.last)
        } else {
            Err(self.error(format!("expected '{word}'")))
        }
    }
    fn name(&mut self) -> Result<String> {
        self.unsupported()?;
        if let TokenKind::Word(name) = &self.current().kind {
            const RESERVED: &[&str] = &[
                "fn",
                "let",
                "var",
                "return",
                "break",
                "continue",
                "if",
                "else",
                "while",
                "match",
                "class",
                "struct",
                "enum",
                "interface",
                "constructor",
                "destructor",
                "const",
                "new",
                "unsafe",
                "extern",
                "import",
                "export",
                "from",
                "public",
                "private",
                "weak",
                "ref",
                "as",
                "true",
                "false",
                "this",
            ];
            if RESERVED.contains(&name.as_str()) {
                return Err(self.error("expected identifier, found reserved keyword"));
            }
            let name = name.clone();
            self.bump();
            Ok(name)
        } else {
            Err(self.error("expected identifier"))
        }
    }
    fn end(&mut self) -> Result<()> {
        if self.eat(";") || self.newline() {
            self.lines();
            Ok(())
        } else if self.at("}") || self.at_eof() {
            Ok(())
        } else if self.at("=>") {
            Err(Diagnostic::new(
                "E0104",
                "lambda syntax is not supported; use a named function",
                self.current().span,
            ))
        } else {
            Err(self.error("expected ';', newline, or end of body"))
        }
    }
    fn nested<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        if self.depth >= 32 {
            return Err(Diagnostic::new(
                "E0201",
                "syntax nesting limit exceeded",
                self.current().span,
            ));
        }
        self.depth += 1;
        let result = f(self);
        self.depth -= 1;
        result
    }
    fn unsupported(&self) -> Result<()> {
        let (code, feature) = match &self.current().kind {
            TokenKind::Word(s) => match s.as_str() {
                "async" | "await" | "spawn" | "task_scope" | "transfer" => ("E0100", "concurrency"),
                "dyn" | "super" | "virtual" | "override" => {
                    ("E0102", "runtime polymorphism or inheritance")
                }
                "for" | "loop" => ("E0103", "iteration syntax; use while"),
                "owns" | "type" => ("E0106", "ownership annotations or associated types"),
                _ => return Ok(()),
            },
            TokenKind::Symbol("...") => ("E0101", "variadics"),
            TokenKind::Symbol("?") => ("E0107", "propagation; use match"),
            TokenKind::Symbol(".." | "..=") => ("E0103", "range expressions"),
            _ => return Ok(()),
        };
        Err(Diagnostic::new(
            code,
            format!("{feature} is not supported in Nether 0.1"),
            self.current().span,
        ))
    }

    fn block(&mut self) -> Result<Block> {
        self.nested(|p| {
            p.lines();
            let start = p.expect("{")?;
            let mut statements = Vec::new();
            p.separators();
            while !p.at("}") {
                if p.at_eof() {
                    return Err(p.error("unterminated block"));
                }
                statements.push(p.statement()?);
                p.separators();
            }
            let end = p.expect("}")?;
            Ok(Block {
                statements,
                span: start.join(end),
            })
        })
    }

    fn statement(&mut self) -> Result<Statement> {
        self.unsupported()?;
        let start = self.current().span;
        let kind = if self.word("let") || self.word("var") {
            let mutable = self.eat_word("var");
            if !mutable {
                self.bump();
            }
            let pattern = self.pattern()?;
            let ty = if self.eat(":") {
                self.lines();
                Some(self.ty()?)
            } else {
                None
            };
            self.expect("=")?;
            self.lines();
            let value = self.expr(0, false, false)?;
            self.end()?;
            StatementKind::Binding {
                mutable,
                pattern,
                ty,
                value,
            }
        } else if self.eat_word("return") {
            let value = if self.newline() || self.at(";") || self.at("}") || self.at_eof() {
                None
            } else {
                Some(self.expr(0, false, false)?)
            };
            self.end()?;
            StatementKind::Return(value)
        } else if self.eat_word("break") {
            self.end()?;
            StatementKind::Break
        } else if self.eat_word("continue") {
            self.end()?;
            StatementKind::Continue
        } else if self.eat_word("unsafe") {
            StatementKind::Unsafe(self.block()?)
        } else {
            let value = self.expr(0, false, false)?;
            if !matches!(
                value.kind,
                ExprKind::If { .. } | ExprKind::While { .. } | ExprKind::Match { .. }
            ) {
                self.end()?;
            }
            StatementKind::Expression(value)
        };
        Ok(Statement {
            kind,
            span: start.join(self.last),
        })
    }
}
