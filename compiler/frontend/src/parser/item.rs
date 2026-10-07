use super::*;

impl Parser {
    pub(super) fn item(&mut self) -> Result<Item> {
        self.unsupported()?;
        let start = self.current().span;
        let mut attributes = Vec::new();
        while self.eat("#") {
            self.expect("[")?;
            let name = self.name()?;
            let attribute = match name.as_str() {
                "Copy" => Attribute::Copy,
                "repr" => {
                    self.expect("(")?;
                    self.expect_word("C")?;
                    self.expect(")")?;
                    Attribute::ReprC
                }
                "link" => {
                    self.expect("(")?;
                    let value = self.string()?;
                    self.expect(")")?;
                    Attribute::Link(value)
                }
                _ => return Err(Diagnostic::new("E0108", "unsupported attribute", self.last)),
            };
            self.expect("]")?;
            attributes.push(attribute);
            self.lines();
        }
        let private = self.eat_word("private");
        if !private {
            self.eat_word("public");
        }
        let kind = if self.word("fn") {
            ItemKind::Function(self.function(false, false, false)?)
        } else if self.word("struct") || self.word("class") {
            self.aggregate()?
        } else if self.eat_word("enum") {
            let name = self.name()?;
            let generics = self.generics()?;
            self.lines();
            self.expect("{")?;
            self.lines();
            let mut variants = Vec::new();
            while !self.at("}") {
                let start = self.current().span;
                let name = self.name()?;
                let fields = if self.eat("(") {
                    let mut fields = Vec::new();
                    self.lines();
                    while !self.at(")") {
                        fields.push(self.ty()?);
                        self.lines();
                        if !self.eat(",") {
                            break;
                        }
                        self.lines();
                    }
                    self.expect(")")?;
                    VariantFields::Tuple(fields)
                } else if self.eat("{") {
                    let mut fields = Vec::new();
                    self.lines();
                    while !self.at("}") {
                        let field = self.name()?;
                        self.expect(":")?;
                        self.lines();
                        let ty = self.ty()?;
                        fields.push((field, ty));
                        self.lines();
                        if !self.eat(",") {
                            break;
                        }
                        self.lines();
                    }
                    self.expect("}")?;
                    VariantFields::Named(fields)
                } else {
                    VariantFields::Unit
                };
                variants.push(Variant {
                    name,
                    fields,
                    span: start.join(self.last),
                });
                self.lines();
                if !self.eat(",") {
                    break;
                }
                self.lines();
            }
            self.expect("}")?;
            ItemKind::Enum {
                name,
                generics,
                variants,
            }
        } else if self.eat_word("interface") {
            let name = self.name()?;
            let generics = self.generics()?;
            self.lines();
            self.expect("{")?;
            let mut methods = Vec::new();
            self.separators();
            while !self.at("}") {
                methods.push(self.function(true, true, false)?);
                self.end()?;
                self.separators();
            }
            self.expect("}")?;
            ItemKind::Interface {
                name,
                generics,
                methods,
            }
        } else if self.eat_word("const") {
            let (name, ty, value) = self.constant()?;
            ItemKind::Const { name, ty, value }
        } else if self.word("import") || self.word("export") {
            let export = self.eat_word("export");
            if !export {
                self.bump();
            }
            self.expect("{")?;
            self.lines();
            let mut names = Vec::new();
            while !self.at("}") {
                names.push(self.name()?);
                self.lines();
                if !self.eat(",") {
                    break;
                }
                self.lines();
            }
            self.expect("}")?;
            self.expect_word("from")?;
            let from = self.string()?;
            self.end()?;
            ItemKind::Import {
                export,
                names,
                from,
            }
        } else if self.eat_word("extern") {
            if self.string()? != "C" {
                return Err(self.error("only extern C is supported"));
            }
            self.lines();
            if self.word("fn") {
                ItemKind::Function(self.function(false, false, true)?)
            } else {
                self.expect("{")?;
                self.separators();
                let mut functions = Vec::new();
                while !self.at("}") {
                    functions.push(self.function(false, true, true)?);
                    self.end()?;
                    self.separators();
                }
                self.expect("}")?;
                ItemKind::Extern(functions)
            }
        } else {
            self.unsupported()?;
            return Err(self.error("expected declaration"));
        };
        Ok(Item {
            kind,
            private,
            attributes,
            span: start.join(self.last),
        })
    }

    fn string(&mut self) -> Result<String> {
        if let TokenKind::String(value) = &self.current().kind {
            let value = value.clone();
            self.bump();
            Ok(value)
        } else {
            Err(self.error("expected string literal"))
        }
    }

    fn constant(&mut self) -> Result<(String, Type, Expr)> {
        let name = self.name()?;
        self.expect(":")?;
        self.lines();
        let ty = self.ty()?;
        self.expect("=")?;
        self.lines();
        let value = self.expr(0, false, false)?;
        self.end()?;
        Ok((name, ty, value))
    }

    fn aggregate(&mut self) -> Result<ItemKind> {
        let class = self.eat_word("class");
        if !class {
            self.expect_word("struct")?;
        }
        let name = self.name()?;
        let generics = self.generics()?;
        let mut interfaces = Vec::new();
        if self.eat(":") {
            loop {
                self.lines();
                interfaces.push(self.ty()?);
                if !self.eat(",") {
                    break;
                }
            }
        }
        self.lines();
        self.expect("{")?;
        self.separators();
        let mut members = Vec::new();
        while !self.at("}") {
            self.unsupported()?;
            let start = self.current().span;
            let private = self.eat_word("private");
            if !private {
                self.eat_word("public");
            }
            let member = if self.word("fn") {
                Member::Method {
                    function: self.function(true, false, false)?,
                    private,
                }
            } else if self.eat_word("constructor") {
                if !class {
                    return Err(self.error("struct uses a data literal, not a constructor"));
                }
                let parameters = self.parameters(false)?;
                let body = self.block()?;
                Member::Constructor {
                    parameters,
                    body,
                    span: start.join(self.last),
                }
            } else if self.eat_word("destructor") {
                Member::Destructor(self.block()?)
            } else if self.eat_word("const") {
                let (name, ty, value) = self.constant()?;
                Member::Const {
                    name,
                    ty,
                    value,
                    span: start.join(self.last),
                }
            } else {
                let weak = self.eat_word("weak");
                if weak && !class {
                    return Err(self.error("weak fields are only allowed in class"));
                }
                let name = self.name()?;
                self.expect(":")?;
                self.lines();
                let ty = self.ty()?;
                self.end()?;
                Member::Field {
                    name,
                    ty,
                    weak,
                    private,
                    span: start.join(self.last),
                }
            };
            members.push(member);
            self.separators();
        }
        self.expect("}")?;
        Ok(ItemKind::Aggregate {
            class,
            name,
            generics,
            interfaces,
            members,
        })
    }

    pub(super) fn function(
        &mut self,
        method: bool,
        signature: bool,
        c_abi: bool,
    ) -> Result<Function> {
        let start = self.expect_word("fn")?;
        let name = self.name()?;
        let generics = self.generics()?;
        if c_abi && !generics.is_empty() {
            return Err(self.error("C functions cannot be generic"));
        }
        let parameters = self.parameters(method)?;
        self.expect(":")?;
        self.lines();
        let result = self.ty()?;
        let body = if signature { None } else { Some(self.block()?) };
        Ok(Function {
            name,
            generics,
            parameters,
            result,
            body,
            c_abi,
            span: start.join(self.last),
        })
    }

    fn parameters(&mut self, method: bool) -> Result<Vec<Parameter>> {
        self.expect("(")?;
        self.lines();
        let mut parameters = Vec::new();
        while !self.at(")") {
            self.unsupported()?;
            let start = self.current().span;
            let mutable = self.eat_word("var");
            let receiver = self.eat_word("this");
            let (pattern, ty) = if receiver {
                if !method || !parameters.is_empty() {
                    return Err(self.error("receiver must be the first parameter of a method"));
                }
                (
                    Pattern {
                        kind: PatternKind::Binding {
                            name: "this".into(),
                            by_ref: true,
                            mutable,
                        },
                        span: self.last,
                    },
                    None,
                )
            } else {
                let pattern = self.pattern()?;
                self.expect(":")?;
                self.lines();
                let ty = self.ty()?;
                (pattern, Some(ty))
            };
            parameters.push(Parameter {
                pattern,
                mutable,
                ty,
                receiver,
                span: start.join(self.last),
            });
            self.lines();
            if !self.eat(",") {
                break;
            }
            self.lines();
        }
        self.expect(")")?;
        Ok(parameters)
    }
}
