//! Demand-driven static owned/borrowed entry specialization.
use super::*;
use crate::mir::loans::Access;
pub(super) fn result_key(declaration: &a::Function, signature: &Signature) -> String {
    format!(
        "{:?}:{:?}:{:?}:{:?}:{:?}",
        declaration.span,
        declaration.body,
        signature.parameters,
        signature.modes,
        signature.result.logical()
    )
}
pub(super) fn source_key(declaration: &a::Function, signature: &Signature) -> String {
    format!(
        "{:?}:{:?}:{:?}:{:?}",
        declaration.span,
        declaration.body,
        signature
            .parameters
            .iter()
            .map(Type::logical)
            .collect::<Vec<_>>(),
        signature.result.logical()
    )
}
impl Checker<'_> {
    pub(super) fn consuming_parameter(&self, declaration: &a::Function, index: usize) -> bool {
        let mono = self.mono.borrow();
        let table = mono.consuming.borrow();
        table
            .get(consume::base_name(&declaration.name))
            .is_some_and(|flags| flags.get(index) == Some(&true))
    }
    /// Pass `argument` as a view of its place; a value without a place gets a
    /// temporary that lives for the whole call.
    pub(super) fn borrow_argument(
        &mut self,
        argument: &mut h::Expr,
        access: Access,
        prelude: &mut Vec<h::Statement>,
        storage: &mut Vec<usize>,
    ) -> Result<()> {
        if !views::place(argument) {
            let id = self.locals.len();
            self.locals.push(h::Local {
                view: None,
                name: "<receiver temporary>".into(),
                ty: argument.ty.clone(),
                mutable: true,
                span: argument.span,
            });
            prelude.push(h::Statement::Initialize(id, argument.clone()));
            storage.push(id);
            argument.kind = h::ExprKind::Local(id);
        }
        if access == Access::Mutable {
            self.mutable_place(argument)?;
        }
        *argument = h::Expr {
            ty: argument.ty.clone(),
            span: argument.span,
            kind: h::ExprKind::ArgumentView {
                place: Box::new(argument.clone()),
                access,
            },
        };
        Ok(())
    }
    pub(super) fn contextual_result(
        &mut self,
        mut value: h::Expr,
        expected: Option<&Type>,
    ) -> Result<h::Expr> {
        if let Some(expected) = expected {
            let expected = match expected {
                Type::StoredView(target) => &**target,
                other => other,
            };
            same(expected, &value.ty, value.span)?;
            if expected != &value.ty && self.request_storage_mode(&value, expected)? {
                value.ty = storage::union(&value.ty, expected, value.span)?;
            }
        }
        Ok(value)
    }
    pub(super) fn request_result_mode(&mut self, value: &h::Expr, required: &Type) -> Result<bool> {
        if matches!(required, Type::StoredView(_)) {
            return Ok(false);
        }
        let targets = match &value.kind {
            h::ExprKind::Call(id, _)
            | h::ExprKind::BorrowedCall { function: id, .. }
            | h::ExprKind::StorageCall { function: id, .. } => vec![*id],
            h::ExprKind::CallIndirect(_, _, contract) => contract.targets.clone(),
            _ => return Ok(false),
        };
        let mut changed = false;
        for id in targets {
            let mut mono = self.mono.borrow_mut();
            let Some(signature) = self
                .signatures
                .values()
                .chain(mono.instances.values())
                .chain(mono.entries.values())
                .find(|s| s.id == id)
                .cloned()
            else {
                continue;
            };
            let key = result_key(&mono.bodies[&id], &signature);
            let known = mono
                .result_requests
                .get(&key)
                .or_else(|| mono.result_contracts.get(&key).map(|i| &i.ty))
                .unwrap_or(&value.ty);
            let wanted = storage::union(known, required, value.span)?;
            if known != &wanted {
                mono.result_requests.insert(key, wanted);
                changed = true;
            }
        }
        Ok(changed)
    }
    pub(super) fn call_entry(
        &mut self,
        signature: Signature,
        source: &[a::Expr],
        arguments: Vec<h::Expr>,
        span: Span,
    ) -> Result<h::Expr> {
        self.call_entry_modes(signature, source, arguments, span, None)
    }
    pub(super) fn call_entry_modes(
        &mut self,
        signature: Signature,
        source: &[a::Expr],
        mut arguments: Vec<h::Expr>,
        span: Span,
        forced: Option<&[Option<Access>]>,
    ) -> Result<h::Expr> {
        for (argument, ty) in arguments.iter().zip(&signature.parameters) {
            same(ty, &argument.ty, argument.span)?;
        }
        let parameters = arguments.iter().map(|a| a.ty.clone()).collect::<Vec<_>>();
        let declaration = self.mono.borrow().bodies.get(&signature.id).cloned();
        let mut modes = forced.map_or_else(|| signature.modes.clone(), |m| m.to_vec());
        let mut prelude = Vec::new();
        let mut storage = Vec::new();
        if let Some(declaration) = &declaration {
            for (index, (argument, original)) in arguments.iter_mut().zip(source).enumerate() {
                // NMM §11: a borrowed parameter takes a view of the caller's place
                // (or of a temporary kept alive for the call). Owned entries are
                // declared only for parameters the body can consume; they take
                // ownership from an owner or a fresh value, and keep a view entry
                // for an alias, which cannot supply an owner token.
                let _ = original;
                // An owner (or fresh value) passed to a consuming parameter hands
                // over its token: select the owned entry and pass by value.
                if forced.is_none()
                    && modes[index].is_some()
                    && !argument.ty.copyable()
                    && !views::view_root(argument)
                    && self.consuming_parameter(declaration, index)
                {
                    modes[index] = None;
                    continue;
                }
                if modes[index].is_some() {
                    let access = if declaration.parameters[index].mutable {
                        Access::Mutable
                    } else {
                        Access::Readonly
                    };
                    modes[index] = Some(access);
                    self.borrow_argument(argument, access, &mut prelude, &mut storage)?;
                }
            }
        }
        let selected = if modes != signature.modes || parameters != signature.parameters {
            let key = format!("{}:{modes:?}:{parameters:?}", signature.id);
            let mut mono = self.mono.borrow_mut();
            if let Some(entry) = mono.entries.get(&key) {
                entry.clone()
            } else {
                if mono.entries.len() >= 256 {
                    return Err(error(span, "E0372", "entry specialization limit exceeded"));
                }
                let mut declaration = declaration.unwrap();
                let id = mono.next_id;
                mono.next_id += 1;
                declaration.name = format!("{}$entry{id}", declaration.name);
                let entry = Signature {
                    id,
                    modes,
                    parameters,
                    result: signature.result.clone(),
                };
                mono.entries.insert(key, entry.clone());
                mono.bodies.insert(id, declaration.clone());
                mono.pending.push((entry.clone(), declaration));
                entry
            }
        } else {
            signature
        };
        let info = {
            let mut mono = self.mono.borrow_mut();
            let known = mono
                .bodies
                .get(&selected.id)
                .and_then(|d| mono.result_contracts.get(&result_key(d, &selected)))
                .cloned();
            if known.as_ref().is_none_or(|info| !info.complete)
                && (!selected.result.copyable()
                    || selected.parameters.iter().any(|p| !p.copyable()))
            {
                mono.pending_results.insert(selected.id);
            }
            known
        };
        if let Some(info) = &info {
            for (index, required) in info.parameters.iter().enumerate() {
                if let Some(access) = info.modes[index] {
                    if self.result.contains_views() {
                        let place = match &arguments[index].kind {
                            h::ExprKind::ArgumentView { place, .. } => &**place,
                            _ => &arguments[index],
                        };
                        self.request_parameter_view(place);
                    }
                    if !matches!(arguments[index].kind, h::ExprKind::ArgumentView { .. }) {
                        if !views::place(&arguments[index]) {
                            return Err(error(
                                arguments[index].span,
                                "E0396",
                                "borrowed result requires an argument with an existing owner",
                            ));
                        }
                        let place = arguments[index].clone();
                        if access == Access::Mutable {
                            self.mutable_place(&place)?;
                        }
                        arguments[index] = h::Expr {
                            ty: place.ty.clone(),
                            span: place.span,
                            kind: h::ExprKind::ArgumentView {
                                place: Box::new(place),
                                access,
                            },
                        };
                    }
                }
                if arguments[index].ty == *required {
                    continue;
                }
                let (place, access) = match &arguments[index].kind {
                    h::ExprKind::ArgumentView { place, access } => {
                        ((**place).clone(), Some(*access))
                    }
                    _ => (arguments[index].clone(), None),
                };
                if !self.request_storage_mode(&place, required)? {
                    let value = self.expr(&source[index], Some(required), true)?;
                    self.same_storage_mode(required, &value.ty, value.span)?;
                    arguments[index] = if let Some(access) = access {
                        h::Expr {
                            ty: value.ty.clone(),
                            span: value.span,
                            kind: h::ExprKind::ArgumentView {
                                place: Box::new(value),
                                access,
                            },
                        }
                    } else {
                        value
                    };
                }
            }
        }
        let result = info
            .as_ref()
            .map_or_else(|| selected.result.clone(), |info| info.ty.clone());
        let call = h::Expr {
            kind: if let Some(contract) = info.as_ref().and_then(|info| info.view.clone()) {
                h::ExprKind::BorrowedCall {
                    function: selected.id,
                    arguments,
                    contract,
                }
            } else if let Some(info) = info.filter(|info| !info.storage.is_empty()) {
                h::ExprKind::StorageCall {
                    function: selected.id,
                    arguments,
                    contract: info.storage,
                }
            } else {
                h::ExprKind::Call(selected.id, arguments)
            },
            ty: result,
            span,
        };
        if prelude.is_empty() {
            Ok(call)
        } else {
            prelude.push(h::Statement::Evaluate(call.clone()));
            Ok(h::Expr {
                kind: h::ExprKind::Block(h::Block {
                    construction: vec![],
                    storage,
                    statements: prelude,
                    result: call.ty.clone(),
                    diverges: false,
                    span,
                }),
                ty: call.ty,
                span,
            })
        }
    }
}
