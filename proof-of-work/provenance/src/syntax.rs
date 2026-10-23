//! Лексер и парсер подмножества Nether, достаточного для проверки гипотезы.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ty {
    Int,
    Unit,
    Class(String),
}

impl Ty {
    /// Copy-типы: `i32` и `()`. Любой class — non-Copy.
    pub fn is_copy(&self) -> bool {
        !matches!(self, Ty::Class(_))
    }
}

#[derive(Clone, Debug)]
pub struct ClassDef {
    pub name: String,
    pub fields: Vec<(String, Ty)>,
}

#[derive(Clone, Debug)]
pub struct ParamDef {
    pub name: String,
    pub var: bool,
    pub ty: Ty,
}

#[derive(Clone, Debug)]
pub struct FuncDef {
    pub name: String,
    pub params: Vec<ParamDef>,
    pub ret: Ty,
    pub body: Vec<Stmt>,
    pub line: usize,
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Let {
        name: String,
        var: bool,
        init: Expr,
        line: usize,
    },
    Assign {
        name: String,
        rhs: Expr,
        line: usize,
    },
    FieldAssign {
        base: String,
        field: String,
        rhs: Expr,
        line: usize,
    },
    Return {
        value: Option<Expr>,
        line: usize,
    },
    If {
        cond: Expr,
        then: Vec<Stmt>,
        els: Vec<Stmt>,
        line: usize,
    },
    While {
        cond: Expr,
        body: Vec<Stmt>,
        line: usize,
    },
    Expr {
        expr: Expr,
        line: usize,
    },
}

#[derive(Clone, Debug)]
pub enum Expr {
    Int(i64),
    Var(String),
    Field(String, String),
    New(String, Vec<Expr>),
    Call(String, Vec<Expr>),
    Bin(Box<Expr>, char, Box<Expr>),
}

#[derive(Clone, Debug, Default)]
pub struct Program {
    pub classes: Vec<ClassDef>,
    pub funcs: Vec<FuncDef>,
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Id(String),
    Int(i64),
    P(&'static str),
}

const PUNCT: [&str; 14] = [
    "+=", "==", "(", ")", "{", "}", ":", ",", ";", ".", "=", "+", "-", "<",
];

fn lex(src: &str) -> Result<Vec<(Tok, usize)>, String> {
    let cs: Vec<char> = src.chars().collect();
    let (mut i, mut line, mut out) = (0, 1, Vec::new());
    while i < cs.len() {
        let c = cs[i];
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else if c == '/' && cs.get(i + 1) == Some(&'/') {
            while i < cs.len() && cs[i] != '\n' {
                i += 1;
            }
        } else if c.is_ascii_digit() {
            let s = i;
            while i < cs.len() && cs[i].is_ascii_digit() {
                i += 1;
            }
            let n: String = cs[s..i].iter().collect();
            out.push((
                Tok::Int(n.parse().map_err(|_| format!("{line}: bad int"))?),
                line,
            ));
        } else if c.is_alphabetic() || c == '_' {
            let s = i;
            while i < cs.len() && (cs[i].is_alphanumeric() || cs[i] == '_') {
                i += 1;
            }
            out.push((Tok::Id(cs[s..i].iter().collect()), line));
        } else {
            let rest: String = cs[i..(i + 2).min(cs.len())].iter().collect();
            let p = PUNCT
                .iter()
                .find(|p| rest.starts_with(**p))
                .ok_or_else(|| format!("{line}: unexpected '{c}'"))?;
            i += p.len();
            out.push((Tok::P(p), line));
        }
    }
    Ok(out)
}

struct Parser {
    toks: Vec<(Tok, usize)>,
    pos: usize,
}

type R<T> = Result<T, String>;

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.0)
    }
    fn peek_at(&self, n: usize) -> Option<&Tok> {
        self.toks.get(self.pos + n).map(|t| &t.0)
    }
    fn line(&self) -> usize {
        self.toks
            .get(self.pos)
            .or(self.toks.last())
            .map_or(0, |t| t.1)
    }
    fn is_p(&self, p: &str) -> bool {
        matches!(self.peek(), Some(Tok::P(q)) if *q == p)
    }
    fn is_kw(&self, k: &str) -> bool {
        matches!(self.peek(), Some(Tok::Id(s)) if s == k)
    }
    fn eat_p(&mut self, p: &str) -> bool {
        let r = self.is_p(p);
        if r {
            self.pos += 1;
        }
        r
    }
    fn eat_kw(&mut self, k: &str) -> bool {
        let r = self.is_kw(k);
        if r {
            self.pos += 1;
        }
        r
    }
    fn expect_p(&mut self, p: &str) -> R<()> {
        if self.eat_p(p) {
            Ok(())
        } else {
            Err(format!(
                "{}: expected '{p}', found {:?}",
                self.line(),
                self.peek()
            ))
        }
    }
    fn ident(&mut self) -> R<String> {
        match self.peek().cloned() {
            Some(Tok::Id(s)) => {
                self.pos += 1;
                Ok(s)
            }
            t => Err(format!("{}: expected identifier, found {t:?}", self.line())),
        }
    }
    fn ty(&mut self) -> R<Ty> {
        if self.eat_p("(") {
            self.expect_p(")")?;
            return Ok(Ty::Unit);
        }
        let n = self.ident()?;
        Ok(if n == "i32" { Ty::Int } else { Ty::Class(n) })
    }

    fn program(&mut self) -> R<Program> {
        let mut p = Program::default();
        while self.peek().is_some() {
            if self.eat_kw("class") {
                let name = self.ident()?;
                self.expect_p("{")?;
                let mut fields = Vec::new();
                while !self.eat_p("}") {
                    let f = self.ident()?;
                    self.expect_p(":")?;
                    fields.push((f, self.ty()?));
                    self.eat_p(",");
                }
                p.classes.push(ClassDef { name, fields });
            } else if self.is_kw("fn") {
                p.funcs.push(self.func()?);
            } else {
                return Err(format!("{}: expected 'class' or 'fn'", self.line()));
            }
        }
        Ok(p)
    }

    fn func(&mut self) -> R<FuncDef> {
        let line = self.line();
        self.eat_kw("fn");
        let name = self.ident()?;
        self.expect_p("(")?;
        let mut params = Vec::new();
        while !self.eat_p(")") {
            let var = self.eat_kw("var");
            let name = self.ident()?;
            self.expect_p(":")?;
            params.push(ParamDef {
                name,
                var,
                ty: self.ty()?,
            });
            self.eat_p(",");
        }
        self.expect_p(":")?;
        let ret = self.ty()?;
        let body = self.block()?;
        Ok(FuncDef {
            name,
            params,
            ret,
            body,
            line,
        })
    }

    fn block(&mut self) -> R<Vec<Stmt>> {
        self.expect_p("{")?;
        let mut v = Vec::new();
        while !self.eat_p("}") {
            if self.peek().is_none() {
                return Err("unexpected end of file".into());
            }
            v.push(self.stmt()?);
            self.eat_p(";");
        }
        Ok(v)
    }

    fn stmt(&mut self) -> R<Stmt> {
        let line = self.line();
        if self.is_kw("let") || self.is_kw("var") {
            let var = self.is_kw("var");
            self.pos += 1;
            let name = self.ident()?;
            self.expect_p("=")?;
            return Ok(Stmt::Let {
                name,
                var,
                init: self.expr()?,
                line,
            });
        }
        if self.eat_kw("return") {
            let value = if self.is_p("}") || self.is_p(";") {
                None
            } else {
                Some(self.expr()?)
            };
            return Ok(Stmt::Return { value, line });
        }
        if self.eat_kw("if") {
            let cond = self.expr()?;
            let then = self.block()?;
            let els = if self.eat_kw("else") {
                if self.is_kw("if") {
                    vec![self.stmt()?]
                } else {
                    self.block()?
                }
            } else {
                Vec::new()
            };
            return Ok(Stmt::If {
                cond,
                then,
                els,
                line,
            });
        }
        if self.eat_kw("while") {
            let cond = self.expr()?;
            return Ok(Stmt::While {
                cond,
                body: self.block()?,
                line,
            });
        }
        if let (Some(Tok::Id(n)), Some(Tok::P("="))) = (self.peek().cloned(), self.peek_at(1)) {
            self.pos += 2;
            return Ok(Stmt::Assign {
                name: n,
                rhs: self.expr()?,
                line,
            });
        }
        if let (Some(Tok::Id(b)), Some(Tok::P(".")), Some(Tok::Id(f)), Some(Tok::P(op))) = (
            self.peek().cloned(),
            self.peek_at(1),
            self.peek_at(2).cloned(),
            self.peek_at(3),
        ) {
            if *op == "=" || *op == "+=" {
                self.pos += 4;
                return Ok(Stmt::FieldAssign {
                    base: b,
                    field: f,
                    rhs: self.expr()?,
                    line,
                });
            }
        }
        Ok(Stmt::Expr {
            expr: self.expr()?,
            line,
        })
    }

    fn expr(&mut self) -> R<Expr> {
        let l = self.add()?;
        for (p, c) in [("<", '<'), ("==", '=')] {
            if self.eat_p(p) {
                return Ok(Expr::Bin(Box::new(l), c, Box::new(self.add()?)));
            }
        }
        Ok(l)
    }

    fn add(&mut self) -> R<Expr> {
        let mut l = self.term()?;
        loop {
            let c = if self.eat_p("+") {
                '+'
            } else if self.eat_p("-") {
                '-'
            } else {
                break;
            };
            l = Expr::Bin(Box::new(l), c, Box::new(self.term()?));
        }
        Ok(l)
    }

    fn args(&mut self) -> R<Vec<Expr>> {
        self.expect_p("(")?;
        let mut a = Vec::new();
        while !self.eat_p(")") {
            a.push(self.expr()?);
            self.eat_p(",");
        }
        Ok(a)
    }

    fn term(&mut self) -> R<Expr> {
        match self.peek().cloned() {
            Some(Tok::Int(n)) => {
                self.pos += 1;
                Ok(Expr::Int(n))
            }
            Some(Tok::P("(")) => {
                self.pos += 1;
                let e = self.expr()?;
                self.expect_p(")")?;
                Ok(e)
            }
            Some(Tok::Id(k)) if k == "new" => {
                self.pos += 1;
                let c = self.ident()?;
                Ok(Expr::New(c, self.args()?))
            }
            Some(Tok::Id(n)) => {
                self.pos += 1;
                if self.is_p("(") {
                    Ok(Expr::Call(n, self.args()?))
                } else if self.eat_p(".") {
                    Ok(Expr::Field(n, self.ident()?))
                } else {
                    Ok(Expr::Var(n))
                }
            }
            t => Err(format!("{}: unexpected {t:?} in expression", self.line())),
        }
    }
}

pub fn parse(src: &str) -> Result<Program, String> {
    let toks = lex(src)?;
    Parser { toks, pos: 0 }.program()
}
