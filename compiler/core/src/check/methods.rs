use super::*;
/// Methods become named functions; receiver flags force borrowed entries.
pub(super) fn collect(module: &a::Module) -> a::Module {
    let mut module = module.clone();
    let mut functions = Vec::new();
    for item in &mut module.items {
        let a::ItemKind::Aggregate {
            class: false,
            name,
            generics,
            members,
            ..
        } = &mut item.kind
        else {
            continue;
        };
        members.retain(|member| {
            let a::Member::Method { function, private } = member else {
                return true;
            };
            let mut function = function.clone();
            for parameter in &mut function.parameters {
                if parameter.receiver {
                    parameter.ty = Some(a::Type {
                        kind: a::TypeKind::Named {
                            path: vec![name.clone()],
                            arguments: generics
                                .iter()
                                .map(|generic| match generic.kind {
                                    a::GenericKind::Type { .. } => a::TypeArgument::Type(a::Type {
                                        kind: a::TypeKind::Named {
                                            path: vec![generic.name.clone()],
                                            arguments: vec![],
                                        },
                                        span: generic.span,
                                    }),
                                    a::GenericKind::Const { .. } => {
                                        a::TypeArgument::Const(a::Expr {
                                            kind: a::ExprKind::Name(generic.name.clone()),
                                            span: generic.span,
                                        })
                                    }
                                })
                                .collect(),
                        },
                        span: parameter.span,
                    });
                }
            }
            function.name = format!("{name}.{}", function.name);
            let mut parameters = generics.clone();
            parameters.append(&mut function.generics);
            function.generics = parameters;
            functions.push(a::Item {
                span: function.span,
                kind: a::ItemKind::Function(function),
                private: *private,
                attributes: vec![],
            });
            false
        });
    }
    module.items.extend(functions);
    module
}
pub(super) fn static_call(callee: &a::Expr) -> Option<(String, Vec<a::TypeArgument>)> {
    let a::ExprKind::Member { value, name } = &callee.kind else {
        return None;
    };
    match &value.kind {
        a::ExprKind::Name(owner) => Some((format!("{owner}.{name}"), vec![])),
        a::ExprKind::Specialize { value, arguments } => {
            let a::ExprKind::Name(owner) = &value.kind else {
                return None;
            };
            Some((format!("{owner}.{name}"), arguments.clone()))
        }
        _ => None,
    }
}

pub(super) fn receiver_modes(function: &a::Function) -> Vec<Option<crate::mir::loans::Access>> {
    function
        .parameters
        .iter()
        .map(|p| {
            p.receiver.then_some(if p.mutable {
                crate::mir::loans::Access::Mutable
            } else {
                crate::mir::loans::Access::Readonly
            })
        })
        .collect()
}
impl Checker<'_> {
    pub(super) fn instance_call(
        &mut self,
        callee: &a::Expr,
        types: &[a::TypeArgument],
        arguments: &[a::Expr],
        expected: Option<&Type>,
        span: Span,
    ) -> Result<Option<h::Expr>> {
        let (receiver, method) = match &callee.kind {
            a::ExprKind::Member { value, name } => {
                if let a::ExprKind::Name(name) = &value.kind {
                    if self.lookup(name).is_none() && self.nominals.contains(name) {
                        return Ok(None);
                    }
                }
                if matches!(value.kind, a::ExprKind::Specialize { .. }) {
                    return Ok(None);
                }
                ((**value).clone(), name.clone())
            }
            a::ExprKind::Name(name)
                if self.lookup(name).is_none() && self.lookup("this").is_some() =>
            {
                (
                    a::Expr {
                        kind: a::ExprKind::Name("this".into()),
                        span,
                    },
                    name.clone(),
                )
            }
            _ => return Ok(None),
        };
        let value = self.expr(&receiver, None, true)?;
        let Type::Record { name, .. } = &value.ty else {
            return Ok(None);
        };
        let candidate = format!("{}.{}", self.nominals.origin(name), method);
        let receiver_method = self
            .signatures
            .get(&candidate)
            .is_some_and(|s| s.modes.first().is_some_and(Option::is_some))
            || self
                .mono
                .borrow()
                .templates
                .get(&candidate)
                .is_some_and(|f| f.parameters.first().is_some_and(|p| p.receiver));
        if !receiver_method {
            return Ok(None);
        }
        let mut supplied = self
            .nominals
            .arguments(name)
            .into_iter()
            .map(|a| match a {
                records::Argument::Type(ty) => {
                    a::TypeArgument::Type(generics::substitute::ast_type(&ty, span))
                }
                records::Argument::Const(n) => a::TypeArgument::Const(a::Expr {
                    kind: a::ExprKind::Literal(a::Literal::Integer(n.to_string())),
                    span,
                }),
            })
            .collect::<Vec<_>>();
        supplied.extend_from_slice(types);
        let mut args = vec![receiver];
        args.extend_from_slice(arguments);
        let call = a::Expr {
            kind: a::ExprKind::Call {
                callee: Box::new(a::Expr {
                    kind: a::ExprKind::Name(candidate),
                    span,
                }),
                types: supplied,
                arguments: args,
            },
            span,
        };
        self.expr(&call, expected, true).map(Some)
    }
}
