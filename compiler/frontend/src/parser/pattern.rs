use super::*;

impl Parser {
    pub(super) fn pattern(&mut self) -> Result<Pattern> {
        self.nested(|p| p.pattern_inner())
    }
    fn pattern_inner(&mut self) -> Result<Pattern> {
        let start = self.current().span;
        let kind = if self.eat("..") {
            PatternKind::Rest
        } else if self.eat("(") {
            let fields = self.pattern_list(")")?;
            PatternKind::Tuple(fields)
        } else if self.eat("{") {
            let fields = self.pattern_list("}")?;
            PatternKind::Array(fields)
        } else {
            let negative = self.eat("-");
            if let Some(value) = self.literal() {
                if negative && !matches!(value, Literal::Integer(_) | Literal::Float(_)) {
                    return Err(self.error("only numeric patterns can be negated"));
                }
                PatternKind::Literal { negative, value }
            } else {
                if negative {
                    return Err(self.error("expected numeric literal after '-' in pattern"));
                }
                let by_ref = self.eat_word("ref");
                let mutable = self.eat_word("var");
                let name = self.name()?;
                if name == "_" {
                    if by_ref || mutable {
                        return Err(self.error("wildcard cannot have binding modifiers"));
                    }
                    PatternKind::Wildcard
                } else {
                    let mut path = vec![name];
                    while self.eat(".") {
                        path.push(self.name()?);
                    }
                    if self.at("(") || self.at("{") || path.len() > 1 {
                        if by_ref || mutable {
                            return Err(self.error("put binding modifiers on payload bindings"));
                        }
                        let fields = if self.eat("(") {
                            PatternFields::Tuple(self.pattern_list(")")?)
                        } else if self.eat("{") {
                            let mut fields = Vec::new();
                            let mut rest = false;
                            self.lines();
                            while !self.at("}") {
                                if self.eat("..") {
                                    if rest {
                                        return Err(self.error("duplicate rest pattern"));
                                    }
                                    rest = true;
                                } else {
                                    let field_start = self.current().span;
                                    let by_ref = self.eat_word("ref");
                                    let mutable = self.eat_word("var");
                                    let name = self.name()?;
                                    let value = if self.eat(":") {
                                        if by_ref || mutable {
                                            return Err(self.error("put modifiers after ':' in a renamed field pattern"));
                                        }
                                        self.pattern()?
                                    } else {
                                        Pattern {
                                            kind: PatternKind::Binding {
                                                name: name.clone(),
                                                by_ref,
                                                mutable,
                                            },
                                            span: field_start.join(self.last),
                                        }
                                    };
                                    fields.push((name, value));
                                }
                                self.lines();
                                if !self.eat(",") {
                                    break;
                                }
                                self.lines();
                            }
                            self.expect("}")?;
                            PatternFields::Named(fields, rest)
                        } else {
                            PatternFields::Unit
                        };
                        PatternKind::Variant { path, fields }
                    } else {
                        PatternKind::Binding {
                            name: path.pop().unwrap(),
                            by_ref,
                            mutable,
                        }
                    }
                }
            }
        };
        Ok(Pattern {
            kind,
            span: start.join(self.last),
        })
    }

    fn pattern_list(&mut self, close: &str) -> Result<Vec<Pattern>> {
        self.lines();
        let mut fields = Vec::new();
        let mut rest = false;
        while !self.at(close) {
            let field = self.pattern()?;
            if field.kind == PatternKind::Rest {
                if rest {
                    return Err(self.error("only one rest pattern is allowed per level"));
                }
                rest = true;
            }
            fields.push(field);
            self.lines();
            if !self.eat(",") {
                break;
            }
            self.lines();
        }
        self.expect(close)?;
        Ok(fields)
    }
}
