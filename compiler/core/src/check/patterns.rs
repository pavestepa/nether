use super::*;

impl Checker<'_> {
    pub(super) fn match_expression(
        &mut self,
        value: &a::Expr,
        arms: &[a::Arm],
        expected: Option<&Type>,
        value_context: bool,
        span: Span,
    ) -> Result<h::Expr> {
        let match_storage_start = self.locals.len();
        let borrow =
            arms.iter().any(|arm| views::has_ref(&arm.pattern)) || self.needs_pattern_view(value);
        let value = self.expr(value, None, true)?;
        let mut prelude = Vec::new();
        let scrutinee = if borrow {
            if !views::place(&value) {
                return Err(error(
                    span,
                    "E0382",
                    "borrowed match requires an existing place",
                ));
            }
            let place = self.freeze_place(value.clone(), &mut prelude);
            let id = self.locals.len();
            self.locals.push(h::Local {
                name: "<match address>".into(),
                ty: place.ty.clone(),
                view: Some(crate::mir::loans::Access::Readonly),
                mutable: false,
                span,
            });
            prelude.push(h::Statement::Evaluate(h::Expr {
                kind: h::ExprKind::Borrow {
                    local: id,
                    place: Box::new(place.clone()),
                },
                ty: Type::Unit,
                span,
            }));
            place
        } else {
            let id = self.locals.len();
            self.locals.push(h::Local {
                view: None,
                name: "<match value>".into(),
                ty: value.ty.clone(),
                mutable: false,
                span,
            });
            prelude.push(h::Statement::Initialize(id, value.clone()));
            h::Expr {
                kind: h::ExprKind::Local(id),
                ty: value.ty.clone(),
                span,
            }
        };
        let mut variants = match &value.ty {
            Type::Enum { variants, .. } => vec![false; variants.len()],
            _ => Vec::new(),
        };
        let mut exhaustive = matches!(&value.ty,Type::Enum {variants,..} if variants.is_empty());
        let mut booleans = [false; 2];
        let mut checked = Vec::new();
        let mut result = expected.cloned();
        for arm in arms {
            if exhaustive {
                return Err(error(arm.span, "E0341", "unreachable match arm"));
            }
            let arm_storage_start = self.locals.len();
            let pattern = if borrow {
                views::inferred_pattern(&arm.pattern, false)
            } else {
                arm.pattern.clone()
            };
            self.scopes.push(BTreeMap::new());
            let previous = self.pattern_views;
            self.pattern_views = borrow;
            let mut bindings = Vec::new();
            let (condition, irrefutable, variant) =
                self.pattern(&pattern, &scrutinee, &mut bindings)?;
            let mut guard_bindings = Vec::new();
            let guard = if let Some(guard) = &arm.guard {
                // Guard bindings are readonly and exist before mutable/consuming body bindings.
                self.scopes.push(BTreeMap::new());
                let mut guard_pattern = pattern.clone();
                readonly_pattern(&mut guard_pattern);
                if !scrutinee.ty.copyable() {
                    guard_pattern = views::inferred_pattern(&guard_pattern, false);
                }
                self.pattern_views = true;
                self.pattern(&guard_pattern, &scrutinee, &mut guard_bindings)?;
                let guard = self.expr(guard, Some(&Type::Bool), true)?;
                self.scopes.pop();
                self.pattern_views = borrow;
                if !readonly_guard(&guard) {
                    return Err(error(guard.span, "E0343", "match guard must only read"));
                }
                Some(guard)
            } else {
                if let a::PatternKind::Literal {
                    value: a::Literal::Bool(b),
                    ..
                } = arm.pattern.kind
                {
                    booleans[b as usize] = true;
                }
                if let Some(index) = variant {
                    variants[index] = true;
                }
                exhaustive = irrefutable
                    || booleans.iter().all(|b| *b)
                    || !variants.is_empty() && variants.iter().all(|v| *v);
                None
            };
            self.pattern_views = previous;
            let mut body = self.branch(&arm.value, result.as_ref(), value_context)?;
            body.storage = (arm_storage_start..self.locals.len()).collect();
            if !body.diverges && value_context {
                if let Some(ty) = &result {
                    same(ty, &body.result, arm.span)?;
                } else {
                    result = Some(body.result.clone());
                }
            }
            self.scopes.pop();
            checked.push(h::MatchArm {
                guard_bindings,
                bindings,
                condition,
                guard,
                body,
            });
        }
        if !exhaustive {
            return Err(error(
                span,
                "E0340",
                "non-exhaustive match; add an unguarded fallback pattern",
            ));
        }
        let result = if value_context {
            result.unwrap_or(Type::Unit)
        } else {
            Type::Unit
        };
        let diverges = checked.iter().all(|arm| arm.body.diverges);
        prelude.push(h::Statement::Evaluate(h::Expr {
            kind: h::ExprKind::Match(checked),
            ty: result.clone(),
            span,
        }));
        let block = h::Block {
            storage: (match_storage_start..self.locals.len()).collect(),
            statements: prelude,
            result: result.clone(),
            diverges,
            span,
        };
        Ok(h::Expr {
            kind: h::ExprKind::Block(block),
            ty: result,
            span,
        })
    }

    pub(super) fn pattern(
        &mut self,
        pattern: &a::Pattern,
        value: &h::Expr,
        bindings: &mut Vec<h::Statement>,
    ) -> Result<(h::Expr, bool, Option<usize>)> {
        let span = pattern.span;
        let yes = || h::Expr {
            kind: h::ExprKind::Value(Value::Bool(true)),
            ty: Type::Bool,
            span,
        };
        match &pattern.kind {
            a::PatternKind::Wildcard => Ok((yes(), true, None)),
            a::PatternKind::Binding {
                name,
                mutable,
                by_ref: false,
            } => {
                if self.scopes.last().unwrap().contains_key(name) {
                    return Err(error(span, "E0301", "duplicate binding in pattern"));
                }
                let id = self.local(name.clone(), value.ty.clone(), *mutable, span)?;
                bindings.push(h::Statement::Initialize(id, value.clone()));
                Ok((yes(), true, None))
            }
            a::PatternKind::Binding {
                name,
                mutable,
                by_ref: true,
            } => {
                if !(super::views::view_root(value) || self.pattern_views && views::place(value)) {
                    return Err(error(
                        span,
                        "E0900",
                        "ref pattern requires source view lowering for this context",
                    ));
                }
                if self.scopes.last().unwrap().contains_key(name) {
                    return Err(error(span, "E0301", "duplicate binding in pattern"));
                }
                if *mutable {
                    self.mutable_place(value)?;
                }
                let id = self.local(name.clone(), value.ty.clone(), *mutable, span)?;
                self.locals[id].view = Some(if *mutable {
                    crate::mir::loans::Access::Mutable
                } else {
                    crate::mir::loans::Access::Readonly
                });
                bindings.push(h::Statement::Evaluate(h::Expr {
                    kind: h::ExprKind::Borrow {
                        local: id,
                        place: Box::new(value.clone()),
                    },
                    ty: Type::Unit,
                    span,
                }));
                Ok((yes(), true, None))
            }
            a::PatternKind::Literal {
                negative,
                value: literal_value,
            } => {
                if !matches!(value.ty, Type::Bool | Type::Char | Type::Integer(_)) {
                    return Err(error(
                        span,
                        "E0342",
                        "literal pattern requires bool, char or integer",
                    ));
                }
                let (kind, ty) = literal(literal_value, Some(&value.ty), *negative, span)?;
                same(&value.ty, &ty, span)?;
                Ok((
                    h::Expr {
                        kind: h::ExprKind::Binary(
                            "==",
                            Box::new(value.clone()),
                            Box::new(h::Expr { kind, ty, span }),
                        ),
                        ty: Type::Bool,
                        span,
                    },
                    false,
                    None,
                ))
            }
            a::PatternKind::Variant { path, fields } => {
                if let Type::Record {
                    name,
                    fields: definition,
                    ..
                } = &value.ty
                {
                    if path != &[name.clone()] && path != &[self.nominals.origin(name)] {
                        return Err(error(span, "E0342", "struct pattern type mismatch"));
                    }
                    let a::PatternFields::Named(fields, rest) = fields else {
                        return Err(error(span, "E0342", "struct pattern requires named fields"));
                    };
                    let mut assigned = std::collections::BTreeSet::new();
                    let mut condition = yes();
                    let mut irrefutable = true;
                    for (name, pattern) in fields {
                        let index =
                            definition
                                .iter()
                                .position(|f| f.name == *name)
                                .ok_or_else(|| {
                                    error(pattern.span, "E0342", "unknown struct pattern field")
                                })?;
                        if !assigned.insert(index) {
                            return Err(error(
                                pattern.span,
                                "E0342",
                                "duplicate struct pattern field",
                            ));
                        }
                        let field = &definition[index];
                        if field.private && field.source != span.source {
                            return Err(error(
                                pattern.span,
                                "E0352",
                                "private field is not accessible from this module",
                            ));
                        }
                        let projection = h::Expr {
                            kind: h::ExprKind::Field(Box::new(value.clone()), index),
                            ty: field.ty.clone(),
                            span: pattern.span,
                        };
                        let (test, all, _) = self.pattern(pattern, &projection, bindings)?;
                        irrefutable &= all;
                        condition = h::Expr {
                            kind: h::ExprKind::Binary("&&", Box::new(condition), Box::new(test)),
                            ty: Type::Bool,
                            span,
                        };
                    }
                    if !rest && assigned.len() != definition.len() {
                        return Err(error(span, "E0342", "missing struct pattern fields"));
                    }
                    return Ok((condition, irrefutable, None));
                }

                let Some((ty, index, definition)) = self.enum_pattern(&value.ty, path, span)?
                else {
                    return Err(error(span, "E0342", "unknown enum pattern"));
                };
                same(&value.ty, &ty, span)?;
                let patterns = match (fields, &definition.shape) {
                    (a::PatternFields::Unit, h::VariantShape::Unit) => Vec::new(),
                    (a::PatternFields::Tuple(patterns), h::VariantShape::Tuple)
                        if patterns.len() == definition.fields.len() =>
                    {
                        patterns.clone()
                    }
                    (a::PatternFields::Named(patterns, rest), h::VariantShape::Named(names)) => {
                        let mut result = vec![
                            a::Pattern {
                                kind: a::PatternKind::Wildcard,
                                span
                            };
                            names.len()
                        ];
                        let mut assigned = std::collections::BTreeSet::new();
                        for (name, pattern) in patterns {
                            let i = names
                                .iter()
                                .position(|n| n == name)
                                .ok_or_else(|| error(span, "E0342", "unknown variant field"))?;
                            if !assigned.insert(i) {
                                return Err(error(
                                    span,
                                    "E0342",
                                    "duplicate variant pattern field",
                                ));
                            }
                            result[i] = pattern.clone();
                        }
                        if !rest && assigned.len() != names.len() {
                            return Err(error(span, "E0342", "missing variant pattern fields"));
                        }
                        result
                    }
                    _ => return Err(error(span, "E0342", "variant pattern shape mismatch")),
                };
                let payload = h::Expr {
                    kind: h::ExprKind::Payload(Box::new(value.clone()), index),
                    ty: Type::Tuple(definition.fields.clone()),
                    span,
                };
                let mut test = yes();
                let mut unconditional = true;
                for (i, pattern) in patterns.iter().enumerate() {
                    let projection = h::Expr {
                        kind: h::ExprKind::Field(Box::new(payload.clone()), i),
                        ty: definition.fields[i].clone(),
                        span,
                    };
                    let (part, all, _) = self.pattern(pattern, &projection, bindings)?;
                    unconditional &= all;
                    test = h::Expr {
                        kind: h::ExprKind::Binary("&&", Box::new(test), Box::new(part)),
                        ty: Type::Bool,
                        span,
                    };
                }
                let tag = h::Expr {
                    kind: h::ExprKind::Tag(Box::new(value.clone())),
                    ty: Type::Integer(I::U32),
                    span,
                };
                let target = h::Expr {
                    kind: h::ExprKind::Value(Value::Integer(Integer::from_bits(
                        I::U32,
                        index as u128,
                    ))),
                    ty: Type::Integer(I::U32),
                    span,
                };
                let condition = h::Expr {
                    kind: h::ExprKind::Binary("==", Box::new(tag), Box::new(target)),
                    ty: Type::Bool,
                    span,
                };
                let condition = h::Expr {
                    kind: h::ExprKind::Binary("&&", Box::new(condition), Box::new(test)),
                    ty: Type::Bool,
                    span,
                };
                let single = matches!(ty,Type::Enum {ref variants,..} if variants.len()==1);
                Ok((
                    condition,
                    unconditional && single,
                    unconditional.then_some(index),
                ))
            }
            a::PatternKind::Tuple(patterns) | a::PatternKind::Array(patterns) => {
                let (length, tuple) = match (&pattern.kind, &value.ty) {
                    (a::PatternKind::Tuple(_), Type::Tuple(fields)) => (fields.len(), true),
                    (a::PatternKind::Tuple(_), Type::Unit) => (0, true),
                    (a::PatternKind::Array(_), Type::Array(_, count)) => (*count as usize, false),
                    _ => {
                        return Err(error(
                            span,
                            "E0342",
                            "pattern shape does not match value type",
                        ))
                    }
                };
                let rest = patterns.iter().position(|p| p.kind == a::PatternKind::Rest);
                let explicit = patterns.len() - usize::from(rest.is_some());
                if explicit > length || rest.is_none() && explicit != length {
                    return Err(error(span, "E0342", "wrong number of pattern elements"));
                }
                let mut condition = yes();
                let mut irrefutable = true;
                for (position, pattern) in patterns.iter().enumerate() {
                    if pattern.kind == a::PatternKind::Rest {
                        continue;
                    }
                    let index = if rest.is_some_and(|r| position > r) {
                        length - (patterns.len() - position)
                    } else {
                        position
                    };
                    let ty = match &value.ty {
                        Type::Tuple(fields) => fields[index].clone(),
                        Type::Array(element, _) => (**element).clone(),
                        _ => unreachable!(),
                    };
                    let kind = if tuple {
                        h::ExprKind::Field(Box::new(value.clone()), index)
                    } else {
                        h::ExprKind::Index(
                            Box::new(value.clone()),
                            Box::new(h::Expr {
                                kind: h::ExprKind::Value(Value::Integer(Integer::from_bits(
                                    I::Usize,
                                    index as u128,
                                ))),
                                ty: Type::Integer(I::Usize),
                                span,
                            }),
                        )
                    };
                    let (test, unconditional, _) =
                        self.pattern(pattern, &h::Expr { kind, ty, span }, bindings)?;
                    irrefutable &= unconditional;
                    condition = h::Expr {
                        kind: h::ExprKind::Binary("&&", Box::new(condition), Box::new(test)),
                        ty: Type::Bool,
                        span,
                    };
                }
                Ok((condition, irrefutable, None))
            }
            _ => Err(error(
                span,
                "E0900",
                "this pattern requires nominal types or ownership checking",
            )),
        }
    }
}

fn readonly_guard(expr: &h::Expr) -> bool {
    use h::ExprKind::*;
    fn block(block: &h::Block) -> bool {
        block
            .statements
            .iter()
            .all(|s| matches!(s, h::Statement::Evaluate(e) if readonly_guard(e)))
    }
    match &expr.kind {
        Value(_) | Local(_) | View(_) => true,
        ArgumentView { place, access } => {
            *access == crate::mir::loans::Access::Readonly && readonly_guard(place)
        }
        // The admitted value subset has Copy arguments and no foreign calls,
        // mutable captures, globals or resource operations. Thus these calls
        // cannot mutate a guard's input; ownership/FFI support must use summaries.
        Call(_, values) | Tuple(values) | Array(values) => values.iter().all(readonly_guard),
        Unary(_, value) | Cast(value) | Field(value, _) | Tag(value) | Payload(value, _) => {
            readonly_guard(value)
        }
        Binary(_, left, right) | Index(left, right) => {
            readonly_guard(left) && readonly_guard(right)
        }
        Block(body) => block(body),
        If { condition, yes, no } => {
            readonly_guard(condition) && block(yes) && no.as_ref().is_none_or(block)
        }
        _ => false,
    }
}

fn readonly_pattern(pattern: &mut a::Pattern) {
    match &mut pattern.kind {
        a::PatternKind::Binding { mutable, .. } => *mutable = false,
        a::PatternKind::Tuple(v) | a::PatternKind::Array(v) => {
            v.iter_mut().for_each(readonly_pattern)
        }
        a::PatternKind::Variant { fields, .. } => match fields {
            a::PatternFields::Tuple(v) => v.iter_mut().for_each(readonly_pattern),
            a::PatternFields::Named(v, _) => v.iter_mut().for_each(|(_, p)| readonly_pattern(p)),
            _ => (),
        },
        _ => (),
    }
}
