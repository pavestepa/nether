use super::*;

impl Parser {
    pub(super) fn literal(&mut self) -> Option<Literal> {
        let value = match self.current().kind.clone() {
            TokenKind::Integer(s) => Literal::Integer(s),
            TokenKind::Float(s) => Literal::Float(s),
            TokenKind::String(s) => Literal::String(s),
            TokenKind::Char(c) => Literal::Char(c),
            TokenKind::Word(s) if s == "true" || s == "false" => Literal::Bool(s == "true"),
            _ => return None,
        };
        self.bump();
        Some(value)
    }

    pub(super) fn expr(&mut self, minimum: u8, soft: bool, before_body: bool) -> Result<Expr> {
        self.nested(|p| p.expression(minimum, soft, before_body))
    }

    fn expression(&mut self, minimum: u8, soft: bool, before_body: bool) -> Result<Expr> {
        if soft {
            self.lines();
        }
        let mut left = self.prefix(soft, before_body)?;
        let mut operations = 0;
        loop {
            operations += 1;
            if operations > 512 {
                return Err(Diagnostic::new(
                    "E0201",
                    "expression chain limit exceeded",
                    self.current().span,
                ));
            }
            if soft {
                self.lines();
            } else if self.newline() {
                let (saved, last) = (self.pos, self.last);
                self.lines();
                if !self.at(".") {
                    self.pos = saved;
                    self.last = last;
                    break;
                }
            }
            if minimum <= 13 && self.eat(".") {
                self.lines();
                let name = if let TokenKind::Integer(s) = &self.current().kind {
                    let s = s.clone();
                    self.bump();
                    s
                } else {
                    self.name()?
                };
                let span = left.span.join(self.last);
                left = Expr {
                    kind: ExprKind::Member {
                        value: Box::new(left),
                        name,
                    },
                    span,
                };
                continue;
            }
            if minimum <= 13
                && self.at("<")
                && matches!(left.kind, ExprKind::Name(_) | ExprKind::Member { .. })
            {
                let (saved, last, edits) = (self.pos, self.last, self.edits.len());
                if let Ok(arguments) = self.type_arguments() {
                    if self.at("(") || self.at(".") || (!before_body && self.at("{")) {
                        let span = left.span.join(self.last);
                        left = Expr {
                            kind: ExprKind::Specialize {
                                value: Box::new(left),
                                arguments,
                            },
                            span,
                        };
                        continue;
                    }
                }
                for (index, token) in self.edits.drain(edits..).rev() {
                    self.tokens[index] = token;
                }
                self.pos = saved;
                self.last = last;
            }
            if minimum <= 13 && self.eat("(") {
                let arguments = self.expression_list(")")?;
                let span = left.span.join(self.last);
                let (callee, types) = match left.kind {
                    ExprKind::Specialize { value, arguments } => (value, arguments),
                    _ => (Box::new(left), Vec::new()),
                };
                left = Expr {
                    kind: ExprKind::Call {
                        callee,
                        types,
                        arguments,
                    },
                    span,
                };
                continue;
            }
            if minimum <= 13 && self.eat("[") {
                let index = self.expr(0, true, false)?;
                self.expect("]")?;
                let span = left.span.join(self.last);
                left = Expr {
                    kind: ExprKind::Index {
                        value: Box::new(left),
                        index: Box::new(index),
                    },
                    span,
                };
                continue;
            }
            if minimum <= 13 && !before_body && self.at("{") {
                if let Some(ty) = expression_type(&left) {
                    self.bump();
                    self.lines();
                    let mut fields = Vec::new();
                    while !self.at("}") {
                        let start = self.current().span;
                        let name = self.name()?;
                        let value = if self.eat(":") {
                            self.expr(0, true, false)?
                        } else {
                            Expr {
                                kind: ExprKind::Name(name.clone()),
                                span: start,
                            }
                        };
                        fields.push((name, value));
                        self.lines();
                        if !self.eat(",") {
                            break;
                        }
                        self.lines();
                    }
                    self.expect("}")?;
                    let span = left.span.join(self.last);
                    left = Expr {
                        kind: ExprKind::Construct { ty, fields },
                        span,
                    };
                    continue;
                }
            }
            if minimum <= 11 && self.eat_word("as") {
                self.lines();
                let ty = self.ty()?;
                let span = left.span.join(self.last);
                left = Expr {
                    kind: ExprKind::Cast {
                        value: Box::new(left),
                        ty,
                    },
                    span,
                };
                continue;
            }
            if self.at("?") || self.at("...") || self.at("..") || self.at("..=") {
                self.unsupported()?;
            }
            let Some((operator, precedence, assignment)) = infix(&self.current().kind) else {
                break;
            };
            if precedence < minimum {
                break;
            }
            if precedence == 4
                && matches!(&left.kind, ExprKind::Binary {operator,..} if ["==","!=","<",">","<=",">="].contains(operator))
            {
                return Err(self.error("comparison chains require parentheses"));
            }
            self.bump();
            self.lines();
            let right = self.expr(
                if assignment {
                    precedence
                } else {
                    precedence + 1
                },
                soft,
                before_body,
            )?;
            let span = left.span.join(right.span);
            left = Expr {
                kind: if assignment {
                    ExprKind::Assign {
                        operator,
                        place: Box::new(left),
                        value: Box::new(right),
                    }
                } else {
                    ExprKind::Binary {
                        operator,
                        left: Box::new(left),
                        right: Box::new(right),
                    }
                },
                span,
            };
        }
        Ok(left)
    }

    fn prefix(&mut self, soft: bool, before_body: bool) -> Result<Expr> {
        self.unsupported()?;
        let start = self.current().span;
        let kind = if let Some(value) = self.literal() {
            ExprKind::Literal(value)
        } else if let TokenKind::Symbol(operator @ ("-" | "!" | "*" | "&")) = self.current().kind {
            self.bump();
            let operator = if operator == "&" && self.eat_word("var") {
                "&var"
            } else {
                operator
            };
            self.lines();
            let value = self.expr(12, soft, before_body)?;
            ExprKind::Unary {
                operator,
                value: Box::new(value),
            }
        } else if self.eat("(") {
            self.lines();
            if self.eat(")") {
                ExprKind::Tuple(Vec::new())
            } else {
                let first = self.expr(0, true, false)?;
                if self.eat(",") {
                    let mut values = vec![first];
                    values.extend(self.expression_list(")")?);
                    ExprKind::Tuple(values)
                } else {
                    self.expect(")")?;
                    ExprKind::Group(Box::new(first))
                }
            }
        } else if self.eat("{") {
            ExprKind::Array(self.expression_list("}")?)
        } else if self.at("[") {
            return Err(Diagnostic::new(
                "E0105",
                "dynamic array literals are not supported",
                start,
            ));
        } else if self.eat_word("new") {
            let ty = self.ty()?;
            self.expect("(")?;
            let arguments = self.expression_list(")")?;
            ExprKind::New { ty, arguments }
        } else if self.eat_word("if") {
            if self.word("let") {
                return Err(Diagnostic::new(
                    "E0107",
                    "if let is not supported; use match",
                    self.current().span,
                ));
            }
            let condition = Box::new(self.expr(0, false, true)?);
            let then_block = self.block()?;
            let (saved, last) = (self.pos, self.last);
            self.lines();
            let else_branch = if self.eat_word("else") {
                self.lines();
                Some(Box::new(if self.word("if") {
                    self.expr(0, false, before_body)?
                } else {
                    let block = self.block()?;
                    Expr {
                        span: block.span,
                        kind: ExprKind::Branch(block),
                    }
                }))
            } else {
                self.pos = saved;
                self.last = last;
                None
            };
            ExprKind::If {
                condition,
                then_block,
                else_branch,
            }
        } else if self.eat_word("while") {
            if self.word("let") {
                return Err(Diagnostic::new(
                    "E0107",
                    "while let is not supported; use match",
                    self.current().span,
                ));
            }
            let condition = Box::new(self.expr(0, false, true)?);
            let body = self.block()?;
            ExprKind::While { condition, body }
        } else if self.eat_word("match") {
            let value = Box::new(self.expr(0, false, true)?);
            self.lines();
            self.expect("{")?;
            self.lines();
            let mut arms = Vec::new();
            while !self.at("}") {
                let start = self.current().span;
                let pattern = self.pattern()?;
                let guard = if self.eat_word("if") {
                    Some(self.expr(0, false, true)?)
                } else {
                    None
                };
                self.expect("=>")?;
                self.lines();
                let value = if self.at("{") {
                    let block = self.block()?;
                    Expr {
                        span: block.span,
                        kind: ExprKind::Branch(block),
                    }
                } else {
                    self.expr(0, false, false)?
                };
                arms.push(Arm {
                    pattern,
                    guard,
                    value,
                    span: start.join(self.last),
                });
                self.lines();
                if !self.eat(",") {
                    break;
                }
                self.lines();
            }
            self.expect("}")?;
            ExprKind::Match { value, arms }
        } else if self.eat_word("this") {
            ExprKind::Name("this".into())
        } else {
            ExprKind::Name(self.name()?)
        };
        Ok(Expr {
            kind,
            span: start.join(self.last),
        })
    }

    fn expression_list(&mut self, close: &str) -> Result<Vec<Expr>> {
        self.lines();
        let mut values = Vec::new();
        while !self.at(close) {
            values.push(self.expr(0, true, false)?);
            self.lines();
            if !self.eat(",") {
                break;
            }
            self.lines();
        }
        self.expect(close)?;
        Ok(values)
    }
}

fn infix(kind: &TokenKind) -> Option<(&'static str, u8, bool)> {
    let TokenKind::Symbol(operator) = kind else {
        return None;
    };
    let (precedence, assignment) = match *operator {
        "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | ">>=" => (1, true),
        "||" => (2, false),
        "&&" => (3, false),
        "==" | "!=" | "<" | ">" | "<=" | ">=" => (4, false),
        "|" => (5, false),
        "^" => (6, false),
        "&" => (7, false),
        "<<" | ">>" => (8, false),
        "+" | "-" => (9, false),
        "*" | "/" | "%" => (10, false),
        _ => return None,
    };
    Some((operator, precedence, assignment))
}

fn expression_type(expr: &Expr) -> Option<Type> {
    let (path, arguments) = match &expr.kind {
        ExprKind::Name(name) => (vec![name.clone()], Vec::new()),
        ExprKind::Member { value, name } => {
            let base = expression_type(value)?;
            let TypeKind::Named {
                mut path,
                arguments,
            } = base.kind
            else {
                return None;
            };
            path.push(name.clone());
            (path, arguments)
        }
        ExprKind::Specialize { value, arguments } => {
            let base = expression_type(value)?;
            let TypeKind::Named { path, .. } = base.kind else {
                return None;
            };
            (path, arguments.clone())
        }
        _ => return None,
    };
    Some(Type {
        kind: TypeKind::Named { path, arguments },
        span: expr.span,
    })
}
