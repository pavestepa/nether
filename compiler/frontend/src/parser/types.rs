use super::*;

impl Parser {
    pub(super) fn ty(&mut self) -> Result<Type> {
        self.nested(|p| p.type_inner())
    }

    fn type_inner(&mut self) -> Result<Type> {
        self.unsupported()?;
        let start = self.current().span;
        let kind = if self.eat("*") {
            let mutable = self.eat_word("var");
            self.lines();
            TypeKind::Pointer {
                mutable,
                pointee: Box::new(self.ty()?),
            }
        } else if self.eat("{") {
            self.lines();
            let element = Box::new(self.ty()?);
            self.expect(";")?;
            self.lines();
            let length = Box::new(self.expr(0, true, false)?);
            self.lines();
            self.expect("}")?;
            TypeKind::Array { element, length }
        } else if self.eat("(") {
            self.lines();
            let mut parameters = Vec::new();
            let mut comma = false;
            while !self.at(")") {
                let mutable = self.eat_word("var");
                let ty = self.ty()?;
                parameters.push((mutable, ty));
                self.lines();
                if !self.eat(",") {
                    break;
                }
                comma = true;
                self.lines();
            }
            self.expect(")")?;
            if self.eat("=>") {
                self.lines();
                TypeKind::Function {
                    parameters,
                    result: Box::new(self.ty()?),
                }
            } else {
                if parameters.iter().any(|(mutable, _)| *mutable) {
                    return Err(self.error("var in a type requires a function parameter"));
                }
                if parameters.len() == 1 && !comma {
                    let mut ty = parameters.pop().unwrap().1;
                    ty.span = start.join(self.last);
                    return Ok(ty);
                }
                TypeKind::Tuple(parameters.into_iter().map(|(_, ty)| ty).collect())
            }
        } else {
            let mut path = vec![self.name()?];
            while self.eat(".") {
                path.push(self.name()?);
            }
            let primitive = path.len() == 1
                && [
                    "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128",
                    "usize", "f32", "f64", "bool", "char", "str",
                ]
                .contains(&path[0].as_str());
            let arguments = if self.at("<") && !primitive {
                self.type_arguments()?
            } else {
                Vec::new()
            };
            TypeKind::Named { path, arguments }
        };
        Ok(Type {
            kind,
            span: start.join(self.last),
        })
    }

    pub(super) fn close_angle(&mut self) -> Result<()> {
        if self.eat(">") {
            return Ok(());
        }
        if self.at(">>") || self.at(">=") || self.at(">>=") {
            let span = self.current().span;
            self.last = Span {
                end: span.start + 1,
                ..span
            };
            self.edits.push((self.pos, self.tokens[self.pos].clone()));
            let remaining = match self.current().kind {
                TokenKind::Symbol(">>") => ">",
                TokenKind::Symbol(">=") => "=",
                _ => ">=",
            };
            self.tokens[self.pos] = Token {
                kind: TokenKind::Symbol(remaining),
                span: Span {
                    start: span.start + 1,
                    ..span
                },
            };
            Ok(())
        } else {
            Err(self.error("expected '>'"))
        }
    }

    pub(super) fn type_arguments(&mut self) -> Result<Vec<TypeArgument>> {
        self.expect("<")?;
        self.lines();
        let mut arguments = Vec::new();
        if self.at(">") || self.at(">>") {
            return Err(self.error("generic argument list cannot be empty"));
        }
        loop {
            let numeric = matches!(self.current().kind, TokenKind::Integer(_)) || self.at("-");
            let arithmetic_name = matches!(self.current().kind, TokenKind::Word(_))
                && self.tokens.get(self.pos + 1).is_some_and(|t| {
                    matches!(t.kind, TokenKind::Symbol("+" | "-" | "*" | "/" | "%"))
                });
            arguments.push(if numeric || arithmetic_name {
                TypeArgument::Const(self.expr(9, true, false)?)
            } else {
                TypeArgument::Type(self.ty()?)
            });
            self.lines();
            if !self.eat(",") {
                break;
            }
            self.lines();
            if self.at(">") || self.at(">>") {
                break;
            }
        }
        self.close_angle()?;
        Ok(arguments)
    }

    pub(super) fn generics(&mut self) -> Result<Vec<Generic>> {
        if !self.eat("<") {
            return Ok(Vec::new());
        }
        self.lines();
        let mut generics = Vec::new();
        let mut saw_default = false;
        loop {
            self.unsupported()?;
            let start = self.current().span;
            let constant = self.eat_word("const");
            let name = self.name()?;
            let kind = if constant {
                self.expect(":")?;
                let ty = self.ty()?;
                if !matches!(&ty.kind, TypeKind::Named { path, arguments } if path == &["usize"] && arguments.is_empty())
                {
                    return Err(self.error("const generic parameters must have type usize"));
                }
                let default = if self.eat("=") {
                    self.lines();
                    Some(self.expr(9, true, false)?)
                } else {
                    None
                };
                if default.is_none() && saw_default {
                    return Err(self.error("required generic parameter follows a default"));
                }
                saw_default |= default.is_some();
                GenericKind::Const { ty, default }
            } else {
                let mut constraints = Vec::new();
                if self.eat(":") {
                    loop {
                        self.lines();
                        constraints.push(self.ty()?);
                        if !self.eat("&") {
                            break;
                        }
                    }
                }
                let default = if self.eat("=") {
                    self.lines();
                    Some(self.ty()?)
                } else {
                    None
                };
                if default.is_none() && saw_default {
                    return Err(self.error("required generic parameter follows a default"));
                }
                saw_default |= default.is_some();
                GenericKind::Type {
                    constraints,
                    default,
                }
            };
            generics.push(Generic {
                name,
                kind,
                span: start.join(self.last),
            });
            self.lines();
            if !self.eat(",") {
                break;
            }
            self.lines();
            if self.at(">") || self.at(">>") {
                break;
            }
        }
        self.close_angle()?;
        Ok(generics)
    }
}
