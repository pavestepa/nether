//! Demand-driven static owned/borrowed entry specialization.
use super::*;
use crate::mir::loans::Access;
impl Checker<'_> {
    pub(super) fn call_entry(
        &mut self,
        signature: Signature,
        source: &[a::Expr],
        mut arguments: Vec<h::Expr>,
        span: Span,
    ) -> Result<h::Expr> {
        let declaration = self.mono.borrow().bodies.get(&signature.id).cloned();
        let mut modes = signature.modes.clone();
        let mut prelude = Vec::new();
        let mut storage = Vec::new();
        if let Some(declaration) = &declaration {
            for (index, (argument, original)) in arguments.iter_mut().zip(source).enumerate() {
                if modes[index].is_some()
                    || (!argument.ty.copyable()
                        && views::place(argument)
                        && (views::view_root(argument) || self.needs_pattern_view(original)))
                {
                    let access = if declaration.parameters[index].mutable {
                        Access::Mutable
                    } else {
                        Access::Readonly
                    };
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
                    modes[index] = Some(access);
                    *argument = h::Expr {
                        ty: argument.ty.clone(),
                        span: argument.span,
                        kind: h::ExprKind::ArgumentView {
                            place: Box::new(argument.clone()),
                            access,
                        },
                    };
                }
            }
        }
        let selected = if modes != signature.modes {
            let key = format!("{}:{modes:?}", signature.id);
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
                    parameters: signature.parameters.clone(),
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
        let call = h::Expr {
            kind: h::ExprKind::Call(selected.id, arguments),
            ty: selected.result,
            span,
        };
        if prelude.is_empty() {
            Ok(call)
        } else {
            prelude.push(h::Statement::Evaluate(call.clone()));
            Ok(h::Expr {
                kind: h::ExprKind::Block(h::Block {
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
