use super::*;
pub(super) fn prepare(module: &a::Module) -> Result<a::Module> {
    let mut module = module.clone();
    for item in &mut module.items {
        let a::ItemKind::Aggregate {
            class: true,
            members,
            ..
        } = &mut item.kind
        else {
            continue;
        };
        constructors::declarations(members)?;
        if !members
            .iter()
            .any(|m| matches!(m, a::Member::Constructor { .. }))
            && !members.iter().any(|m| matches!(m, a::Member::Field { .. }))
        {
            members.push(a::Member::Constructor {
                parameters: vec![],
                body: a::Block {
                    statements: vec![],
                    span: item.span,
                },
                span: item.span,
            });
        }
        for member in members {
            if let a::Member::Constructor {
                parameters,
                body,
                span,
            } = member
            {
                let span = *span;
                let mut parameters = parameters.clone();
                parameters.insert(
                    0,
                    a::Parameter {
                        pattern: a::Pattern {
                            kind: a::PatternKind::Binding {
                                name: "this".into(),
                                by_ref: false,
                                mutable: true,
                            },
                            span,
                        },
                        mutable: true,
                        ty: None,
                        receiver: true,
                        span,
                    },
                );
                *member = a::Member::Method {
                    dispatch: a::MethodDispatch::Ordinary,
                    private: true,
                    function: a::Function {
                        name: "$init".into(),
                        generics: vec![],
                        parameters,
                        result: a::Type {
                            kind: a::TypeKind::Tuple(vec![]),
                            span,
                        },
                        body: Some(body.clone()),
                        c_abi: false,
                        unsafe_: false,
                        span,
                    },
                };
            }
        }
    }
    Ok(module)
}
impl Checker<'_> {
    pub(super) fn new_class(
        &mut self,
        ty: &a::Type,
        arguments: &[a::Expr],
        expected: Option<&Type>,
        span: Span,
    ) -> Result<h::Expr> {
        let logical = resolve_type_with(ty, self.constants, self.nominals)?;
        let ty = self
            .storage_hints
            .get(&storage::key(span))
            .or(expected)
            .filter(|t| matches!(t, Type::Class { .. }) && t.logical() == logical)
            .cloned()
            .unwrap_or(logical);
        let Type::Class { name, .. } = &ty else {
            return Err(error(span, "E0394", "new requires a class type"));
        };
        if let Some(expected) = expected {
            same(expected, &ty, span)?;
        }
        let candidate = format!("{}.$init", self.nominals.origin(name));
        if !self.signatures.contains_key(&candidate)
            && !self.mono.borrow().templates.contains_key(&candidate)
        {
            return Err(error(span, "E0394", "class has no constructor"));
        }
        self.scopes.push(BTreeMap::new());
        let receiver = self.local("<new receiver>".into(), ty.clone(), true, span)?;
        self.storage_roots.insert(receiver);
        let mut args = vec![a::Expr {
            kind: a::ExprKind::Name("<new receiver>".into()),
            span,
        }];
        args.extend_from_slice(arguments);
        let supplied = self
            .nominals
            .arguments(name)
            .into_iter()
            .map(|a| match a {
                records::Argument::Type(t) => {
                    a::TypeArgument::Type(generics::substitute::ast_type(&t, span))
                }
                records::Argument::Const(n) => a::TypeArgument::Const(a::Expr {
                    kind: a::ExprKind::Literal(a::Literal::Integer(n.to_string())),
                    span,
                }),
            })
            .collect::<Vec<_>>();
        let generic = self.mono.borrow().templates.contains_key(&candidate);
        let result = if generic {
            self.generic_call(&candidate, &supplied, &args, Some(&Type::Unit), span)
        } else {
            let signature = self.signatures[&candidate].clone();
            if signature.parameters.len() != args.len() {
                Err(error(
                    span,
                    "E0313",
                    "wrong number of constructor arguments",
                ))
            } else {
                let checked = args
                    .iter()
                    .zip(&signature.parameters)
                    .map(|(a, t)| self.expr(a, Some(t), true))
                    .collect::<Result<Vec<_>>>()?;
                self.call_entry(signature, &args, checked, span)
            }
        };
        self.scopes.pop();
        let h::ExprKind::Call(constructor, mut arguments) = result?.kind else {
            return Err(error(
                span,
                "E0900",
                "constructor argument staging requires additional lowering",
            ));
        };
        arguments.remove(0);
        Ok(h::Expr {
            kind: h::ExprKind::New {
                constructor,
                arguments,
            },
            ty,
            span,
        })
    }
}
