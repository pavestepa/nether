use super::*;
pub(super) const PENDING_CALLABLE: &str = "E0397";

// Plain code pointers retain their statically selected ABI, provenance and
// effects through annotations, aggregates and higher-order parameters.

impl Checker<'_> {
    pub(super) fn static_function_value(
        &mut self,
        source: &a::Expr,
        arguments: &[a::TypeArgument],
        expected: Option<&Type>,
        span: Span,
    ) -> Result<Option<h::Expr>> {
        let a::ExprKind::Member { value, .. } = &source.kind else {
            return Ok(None);
        };
        let owner = match &value.kind {
            a::ExprKind::Name(owner) => owner,
            a::ExprKind::Specialize { value, .. } => {
                let a::ExprKind::Name(owner) = &value.kind else {
                    return Ok(None);
                };
                owner
            }
            _ => return Ok(None),
        };
        if self.lookup(owner).is_some() {
            return Ok(None);
        }
        let Some((name, mut types)) = methods::resolved_static_call(source, self.nominals) else {
            return Ok(None);
        };
        types.extend_from_slice(arguments);
        self.function_value(&name, &types, expected, span)
    }
    pub(super) fn function_value(
        &mut self,
        name: &str,
        types: &[a::TypeArgument],
        expected: Option<&Type>,
        span: Span,
    ) -> Result<Option<h::Expr>> {
        self.check_function_access(name, span)?;
        if self.mono.borrow().templates.contains_key(name) {
            return self
                .generic_function_value(name, types, expected, span)
                .map(Some);
        }
        let Some(signature) = self.signatures.get(name) else {
            return Ok(None);
        };
        if !types.is_empty() {
            return Err(error(span, "E0370", "function is not generic"));
        }
        self.pointer_entry(signature.clone(), expected, span)
            .map(Some)
    }
    fn pointer_entry(
        &mut self,
        signature: Signature,
        expected: Option<&Type>,
        span: Span,
    ) -> Result<h::Expr> {
        if self
            .mono
            .borrow()
            .bodies
            .get(&signature.id)
            .is_some_and(|d| d.unsafe_)
        {
            return Err(error(
                span,
                "E0501",
                "unsafe function pointers are not supported; use a checked safe wrapper",
            ));
        }
        let wanted = match expected {
            Some(Type::Function {
                parameters,
                contract: Some(contract),
                ..
            }) => Some((parameters, contract)),
            _ => None,
        };
        // The entry's inferred storage modes (for example a borrowed parameter
        // whose body stores views) are part of its physical parameter types.
        let inferred = {
            let mono = self.mono.borrow();
            mono.bodies
                .get(&signature.id)
                .and_then(|d| {
                    mono.result_contracts
                        .get(&entries::result_key(d, &signature))
                })
                .filter(|info| info.complete)
                .map(|info| info.parameters.clone())
        };
        let parameters = wanted.map_or_else(
            || inferred.unwrap_or_else(|| signature.parameters.clone()),
            |(p, _)| p.iter().map(|(_, ty)| ty.clone()).collect(),
        );
        let modes = wanted.map_or_else(|| signature.modes.clone(), |(_, c)| c.modes.clone());
        let start = self.locals.len();
        self.scopes.push(BTreeMap::new());
        let result = (|| {
            let mut source = Vec::new();
            let mut args = Vec::new();
            for (i, ty) in parameters.into_iter().enumerate() {
                let name = format!("<pointer argument {i}>");
                let id = self.local(name.clone(), ty.clone(), true, span)?;
                self.callable_parameters.insert(id);
                source.push(a::Expr {
                    kind: a::ExprKind::Name(name),
                    span,
                });
                args.push(h::Expr {
                    kind: h::ExprKind::Local(id),
                    ty,
                    span,
                });
            }
            let call = self.call_entry_modes(signature, &source, args, span, Some(&modes))?;
            let mut pointer = self.pointer_from_call(call, span)?;
            if let (
                Some((_, wanted)),
                Type::Function {
                    contract: Some(actual),
                    ..
                },
                h::ExprKind::Value(Value::Function(id)),
            ) = (wanted, &pointer.ty, &pointer.kind)
            {
                if wanted.view.is_some()
                    && actual.view.is_none()
                    && !self.mono.borrow().pending_results.contains(id)
                {
                    return Err(error(
                        span,
                        "E0396",
                        "incompatible owned/borrowed function result ABI",
                    ));
                }
            }
            if let Some(expected) = expected {
                pointer.ty = storage::union(&pointer.ty, expected, span)?;
            }
            Ok(pointer)
        })();
        self.scopes.pop();
        self.locals.truncate(start);
        self.callable_parameters.retain(|id| *id < start);
        result
    }
    fn pointer_from_call(&self, call: h::Expr, span: Span) -> Result<h::Expr> {
        let (_, _, call) = split_temporaries(call);
        let id = match &call.kind {
            h::ExprKind::Call(id, _) => *id,
            h::ExprKind::BorrowedCall { function, .. }
            | h::ExprKind::StorageCall { function, .. } => *function,
            _ => {
                return Err(error(
                    span,
                    "E0390",
                    "function pointer requires a concrete callable entry",
                ))
            }
        };
        let mono = self.mono.borrow();
        let signature = self
            .signatures
            .values()
            .chain(mono.instances.values())
            .chain(mono.entries.values())
            .find(|s| s.id == id)
            .ok_or_else(|| error(span, "E0390", "function pointer signature is unavailable"))?;
        let d = &mono.bodies[&id];
        let info = mono
            .result_contracts
            .get(&entries::result_key(d, signature));
        let parameters = info
            .map_or(&signature.parameters, |info| &info.parameters)
            .iter()
            .enumerate()
            .map(|(i, ty)| (d.parameters[i].mutable, ty.clone()))
            .collect();
        let contract = h::FunctionContract {
            sources: vec![entries::source_key(d, signature)],
            modes: info.map_or_else(|| signature.modes.clone(), |i| i.modes.clone()),
            view: info.and_then(|i| i.view.clone()),
            storage: info.map_or_else(Vec::new, |i| i.storage.clone()),
            writes: info.map_or_else(Vec::new, |i| i.writes.clone()),
        };
        Ok(h::Expr {
            kind: h::ExprKind::Value(Value::Function(id)),
            ty: Type::Function {
                parameters,
                result: Box::new(call.ty),
                contract: Some(Box::new(contract)),
            },
            span,
        })
    }
    fn generic_function_value(
        &mut self,
        name: &str,
        types: &[a::TypeArgument],
        expected: Option<&Type>,
        span: Span,
    ) -> Result<h::Expr> {
        let Some(Type::Function {
            parameters, result, ..
        }) = expected
        else {
            return Err(error(
                span,
                "E0390",
                "generic function value requires a concrete function type",
            ));
        };
        let template = self.mono.borrow().templates[name].clone();
        if template.unsafe_ {
            return Err(error(
                span,
                "E0501",
                "unsafe function pointers are not supported; use a checked safe wrapper",
            ));
        }
        if template.parameters.len() != parameters.len()
            || template
                .parameters
                .iter()
                .zip(parameters)
                .any(|(p, (mutable, _))| p.mutable != *mutable)
        {
            return Err(error(
                span,
                "E0390",
                "function pointer parameter modes or count do not match",
            ));
        }
        // Typed synthetic arguments reuse ordinary specialization and abstract
        // body auditing, but never enter the resulting HIR.
        let start = self.locals.len();
        self.scopes.push(BTreeMap::new());
        let resolved = (|| {
            let mut arguments = Vec::new();
            for (index, (mutable, ty)) in parameters.iter().enumerate() {
                let name = format!("<function value argument {index}>");
                let id = self.local(name.clone(), ty.clone(), *mutable, span)?;
                self.callable_parameters.insert(id);
                arguments.push(a::Expr {
                    kind: a::ExprKind::Name(name),
                    span,
                });
            }
            self.generic_call(name, types, &arguments, Some(result), span)
        })();
        self.scopes.pop();
        self.locals.truncate(start);
        self.callable_parameters.retain(|id| *id < start);
        let call = resolved?;
        same(result, &call.ty, span)?;
        let pointer = self.pointer_from_call(call, span)?;
        let Value::Function(id) = (match &pointer.kind {
            h::ExprKind::Value(v) => v.clone(),
            _ => unreachable!(),
        }) else {
            unreachable!()
        };
        let signature = {
            let mono = self.mono.borrow();
            mono.instances
                .values()
                .chain(mono.entries.values())
                .find(|s| s.id == id)
                .cloned()
                .unwrap()
        };
        self.pointer_entry(signature, expected, span)
    }
    pub(super) fn indirect_call(
        &mut self,
        callee: &a::Expr,
        types: &[a::TypeArgument],
        arguments: &[a::Expr],
        span: Span,
    ) -> Result<Option<h::Expr>> {
        if let a::ExprKind::Name(name) = &callee.kind {
            let implicit_field = self.lookup("this").is_some_and(|id| {
                matches!(&self.locals[id].ty, Type::Record { fields, .. } if fields.iter().any(|field|field.name==*name))
            });
            if self.lookup(name).is_none() && !implicit_field {
                return Ok(None);
            }
        } else if let Some((name, _)) = methods::resolved_static_call(callee, self.nominals) {
            let shadowed = matches!(&callee.kind,a::ExprKind::Member {value,..} if matches!(&value.kind,a::ExprKind::Name(owner) if self.lookup(owner).is_some()));
            if !shadowed
                && (self.signatures.contains_key(&name)
                    || self.mono.borrow().templates.contains_key(&name))
            {
                return Ok(None);
            }
        }
        if !types.is_empty() {
            return Err(error(
                span,
                "E0390",
                "function pointer calls cannot supply generic arguments",
            ));
        }
        let callee = self.expr(callee, None, true)?;
        let Type::Function {
            parameters,
            result,
            contract,
        } = &callee.ty
        else {
            return Err(error(
                span,
                "E0390",
                "called value is not a function pointer",
            ));
        };
        if parameters.len() != arguments.len() {
            return Err(error(span, "E0313", "wrong number of arguments"));
        }
        let mut values = Vec::new();
        for (argument, (_, ty)) in arguments.iter().zip(parameters) {
            let value = self.expr(argument, Some(ty), true)?;
            if !self.request_storage_mode(&value, ty)?
                && storage::union(ty, &value.ty, value.span)? != value.ty
            {
                self.same_storage_mode(ty, &value.ty, value.span)?;
            }
            values.push(value);
        }
        let mut call_contract = h::CallContract::default();
        let mut result = (**result).clone();
        // Temporaries kept alive for borrowed parameters of the selected entry.
        let mut temporaries = (Vec::new(), Vec::new());
        if let Some(contract) = contract {
            let mut selected_type = None;
            let original_values = values.clone();
            for key in &contract.sources {
                let signature = {
                    let mono = self.mono.borrow();
                    self.signatures
                        .values()
                        .chain(mono.instances.values())
                        .chain(mono.entries.values())
                        .find(|s| {
                            mono.bodies
                                .get(&s.id)
                                .is_some_and(|d| entries::source_key(d, s) == *key)
                        })
                        .cloned()
                }
                .ok_or_else(|| {
                    error(
                        span,
                        PENDING_CALLABLE,
                        "function pointer source is not yet instantiated",
                    )
                })?;
                let call = self.call_entry_modes(
                    signature,
                    arguments,
                    original_values.clone(),
                    span,
                    Some(&contract.modes),
                )?;
                let pointer = self.pointer_from_call(call.clone(), span)?;
                let h::ExprKind::Value(Value::Function(id)) = pointer.kind else {
                    unreachable!()
                };
                let (prelude, storage, call) = split_temporaries(call);
                temporaries = (prelude, storage);
                call_contract.targets.push(id);
                if let Some(old) = selected_type {
                    selected_type = Some(storage::union(&old, &pointer.ty, span)?)
                } else {
                    selected_type = Some(pointer.ty)
                }
                values = match call.kind {
                    h::ExprKind::Call(_, args)
                    | h::ExprKind::BorrowedCall {
                        arguments: args, ..
                    }
                    | h::ExprKind::StorageCall {
                        arguments: args, ..
                    } => args,
                    _ => unreachable!(),
                };
                result = call.ty;
            }
            if let Some(required) = selected_type {
                let Type::Function {
                    contract: Some(meta),
                    ..
                } = &required
                else {
                    unreachable!()
                };
                call_contract.view = meta.view.clone();
                call_contract.storage = meta.storage.clone();
                call_contract.writes = meta.writes.clone();
                if !self.request_storage_mode(&callee, &required)? {
                    self.same_storage_mode(&callee.ty, &required, span)?;
                }
            }
        } else {
            // A pointer type without a known entry uses the default ABI: non-Copy
            // parameters are borrowed (NMM §11), Copy values are passed by value.
            for (value, (mutable, ty)) in values.iter_mut().zip(parameters) {
                if !ty.copyable() {
                    let access = if *mutable {
                        crate::mir::loans::Access::Mutable
                    } else {
                        crate::mir::loans::Access::Readonly
                    };
                    self.borrow_argument(value, access, &mut temporaries.0, &mut temporaries.1)?;
                }
                storage::same_mode(ty, &value.ty, value.span)?;
            }
        }
        let call = h::Expr {
            kind: h::ExprKind::CallIndirect(Box::new(callee), values, call_contract),
            ty: result,
            span,
        };
        let (mut prelude, storage) = temporaries;
        if prelude.is_empty() {
            return Ok(Some(call));
        }
        prelude.push(h::Statement::Evaluate(call.clone()));
        Ok(Some(h::Expr {
            ty: call.ty.clone(),
            kind: h::ExprKind::Block(h::Block {
                construction: vec![],
                storage,
                statements: prelude,
                result: call.ty,
                diverges: false,
                span,
            }),
            span,
        }))
    }
}

/// A call whose borrowed arguments needed temporaries is wrapped in a block
/// `{ tmp = value; call }`; return the prelude, its storage and the bare call.
fn split_temporaries(call: h::Expr) -> (Vec<h::Statement>, Vec<usize>, h::Expr) {
    match call.kind {
        h::ExprKind::Block(mut block) if block.construction.is_empty() => {
            match block.statements.pop() {
                Some(h::Statement::Evaluate(inner)) => (block.statements, block.storage, inner),
                Some(last) => {
                    block.statements.push(last);
                    (
                        vec![],
                        vec![],
                        h::Expr {
                            kind: h::ExprKind::Block(block),
                            ..call
                        },
                    )
                }
                None => (
                    vec![],
                    vec![],
                    h::Expr {
                        kind: h::ExprKind::Block(block),
                        ..call
                    },
                ),
            }
        }
        _ => (vec![], vec![], call),
    }
}
