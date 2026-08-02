use super::*;

impl Mono<'_> {
    pub(super) fn subst_expr(&mut self, expr: &HirExpr, subst: &HashMap<Symbol, Type>) -> MonoExpr {
        let ty = subst_type(&expr.ty, subst);
        let kind = match &expr.kind {
            HirExprKind::Literal(l) => MonoExprKind::Literal(l.clone()),
            HirExprKind::Local(id) => MonoExprKind::Local(*id),
            HirExprKind::FnRef(fn_id) => {
                let hir_fn = self.hir.get(*fn_id);
                assert!(hir_fn.generics.is_empty(), "monomorphization: cannot take a bare reference to generic function `{}` without a call site to infer its type arguments from (see this crate's module docs)", hir_fn.name);
                let target = self.instantiate_plain(*fn_id);
                MonoExprKind::Closure {
                    function: self.instantiate_adapter(target),
                    captures: Vec::new(),
                }
            }
            HirExprKind::Unit => MonoExprKind::Unit,
            HirExprKind::Tuple(items) => MonoExprKind::Tuple(self.subst_exprs(items, subst)),
            HirExprKind::Array(items) => MonoExprKind::Array(self.subst_exprs(items, subst)),
            HirExprKind::Concat(items) => MonoExprKind::Concat(self.subst_exprs(items, subst)),
            HirExprKind::ToString(inner) => {
                MonoExprKind::ToString(Box::new(self.subst_expr(inner, subst)))
            }
            HirExprKind::Unary { op, expr } => MonoExprKind::Unary {
                op: *op,
                expr: Box::new(self.subst_expr(expr, subst)),
            },
            HirExprKind::Binary { op, lhs, rhs } => MonoExprKind::Binary {
                op: *op,
                lhs: Box::new(self.subst_expr(lhs, subst)),
                rhs: Box::new(self.subst_expr(rhs, subst)),
            },
            HirExprKind::Assign { target, value } => MonoExprKind::Assign {
                target: Box::new(self.subst_expr(target, subst)),
                value: Box::new(self.subst_expr(value, subst)),
            },
            HirExprKind::Call { callee, args } => MonoExprKind::Call {
                callee: Box::new(self.subst_expr(callee, subst)),
                args: self.subst_exprs(args, subst),
            },
            HirExprKind::CallStatic {
                fn_id,
                generic_args,
                args,
            } => {
                let args = self.subst_exprs(args, subst);
                let generic_args: Vec<Type> = generic_args
                    .iter()
                    .map(|ty| subst_type(ty, subst))
                    .collect();
                let mono_id = self.resolve_call(*fn_id, &args, &generic_args);
                MonoExprKind::CallStatic {
                    fn_id: mono_id,
                    args,
                }
            }
            HirExprKind::CallBuiltin { name, args } => MonoExprKind::CallBuiltin {
                name: name.clone(),
                args: self.subst_exprs(args, subst),
            },
            HirExprKind::Field { base, index } => MonoExprKind::Field {
                base: Box::new(self.subst_expr(base, subst)),
                index: *index,
            },
            HirExprKind::Index { base, index } => MonoExprKind::Index {
                base: Box::new(self.subst_expr(base, subst)),
                index: Box::new(self.subst_expr(index, subst)),
            },
            HirExprKind::Construct { ty, fields } => MonoExprKind::Construct {
                ty: *ty,
                fields: self.subst_exprs(fields, subst),
            },
            HirExprKind::ConstructVariant {
                enum_id,
                variant,
                payload,
            } => MonoExprKind::ConstructVariant {
                enum_id: *enum_id,
                variant: *variant,
                payload: self.subst_exprs(payload, subst),
            },
            HirExprKind::CallGenericMethod {
                receiver,
                method_name,
                is_static,
                generic_args,
                args,
                ..
            } => {
                let receiver = self.subst_expr(receiver, subst);
                let args = self.subst_exprs(args, subst);
                let generic_args: Vec<Type> = generic_args
                    .iter()
                    .map(|ty| subst_type(ty, subst))
                    .collect();
                self.resolve_generic_method_call(
                    receiver,
                    method_name,
                    *is_static,
                    args,
                    &generic_args,
                    ty.clone(),
                )
            }
            HirExprKind::CallMethod {
                receiver,
                method_name,
                generic_args,
                args,
            } => {
                let receiver = self.subst_expr(receiver, subst);
                let args = self.subst_exprs(args, subst);
                let generic_args: Vec<Type> = generic_args
                    .iter()
                    .map(|ty| subst_type(ty, subst))
                    .collect();
                self.resolve_generic_method_call(
                    receiver,
                    method_name,
                    false,
                    args,
                    &generic_args,
                    ty.clone(),
                )
            }
            HirExprKind::CallArrayMethod {
                receiver,
                method,
                args,
            } => MonoExprKind::CallArrayMethod {
                receiver: Box::new(self.subst_expr(receiver, subst)),
                method: method.clone(),
                args: self.subst_exprs(args, subst),
            },
            HirExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => MonoExprKind::If {
                cond: Box::new(self.subst_expr(cond, subst)),
                then_branch: Box::new(self.subst_expr(then_branch, subst)),
                else_branch: else_branch
                    .as_ref()
                    .map(|e| Box::new(self.subst_expr(e, subst))),
            },
            HirExprKind::Match { scrutinee, arms } => MonoExprKind::Match {
                scrutinee: Box::new(self.subst_expr(scrutinee, subst)),
                arms: arms
                    .iter()
                    .map(|arm| MonoMatchArm {
                        pattern: arm.pattern.clone(),
                        body: self.subst_expr(&arm.body, subst),
                    })
                    .collect(),
            },
            HirExprKind::Block(stmts, tail) => {
                let stmts = stmts
                    .iter()
                    .map(|s| MonoStmt {
                        kind: match &s.kind {
                            HirStmtKind::Let { local, ty, value } => MonoStmtKind::Let {
                                local: *local,
                                ty: subst_type(ty, subst),
                                value: self.subst_expr(value, subst),
                            },
                            HirStmtKind::Expr(e) => MonoStmtKind::Expr(self.subst_expr(e, subst)),
                        },
                    })
                    .collect();
                MonoExprKind::Block(
                    stmts,
                    tail.as_ref().map(|t| Box::new(self.subst_expr(t, subst))),
                )
            }
            HirExprKind::While { cond, body } => MonoExprKind::While {
                cond: Box::new(self.subst_expr(cond, subst)),
                body: Box::new(self.subst_expr(body, subst)),
            },
            HirExprKind::Loop { body } => MonoExprKind::Loop {
                body: Box::new(self.subst_expr(body, subst)),
            },
            HirExprKind::Break(v) => {
                MonoExprKind::Break(v.as_ref().map(|e| Box::new(self.subst_expr(e, subst))))
            }
            HirExprKind::Continue => MonoExprKind::Continue,
            HirExprKind::Return(v) => {
                MonoExprKind::Return(v.as_ref().map(|e| Box::new(self.subst_expr(e, subst))))
            }
            HirExprKind::Closure {
                params,
                captures,
                body,
            } => {
                let function = self.instantiate_closure(params, captures, body, &ty, subst);
                let captures = captures
                    .iter()
                    .map(|capture| MonoExpr {
                        kind: MonoExprKind::Local(capture.local),
                        ty: subst_type(&capture.ty, subst),
                    })
                    .collect();
                MonoExprKind::Closure { function, captures }
            }
        };
        MonoExpr { kind, ty }
    }

    pub(super) fn instantiate_closure(
        &mut self,
        params: &[HirParam],
        captures: &[HirCapture],
        body: &HirExpr,
        closure_ty: &Type,
        subst: &HashMap<Symbol, Type>,
    ) -> MonoFnId {
        let id = MonoFnId(self.functions.len() as u32);
        let (param_tys, ret) = match closure_ty {
            Type::Function(params, ret) => (params.clone(), (**ret).clone()),
            _ => (Vec::new(), Type::Error),
        };
        let mono_params = params
            .iter()
            .zip(param_tys)
            .map(|(param, ty)| MonoParam {
                local: param.local,
                name: param.name.clone(),
                mutable: param.mutable,
                ty,
            })
            .collect();
        let mono_captures = captures
            .iter()
            .map(|capture| MonoCapture {
                local: capture.local,
                ty: subst_type(&capture.ty, subst),
            })
            .collect();
        self.functions.push(MonoFunction {
            id,
            name: Symbol::new(format!("closure_{}", id.0)),
            owner: None,
            is_closure: true,
            self_param: None,
            self_ty: None,
            self_local: None,
            captures: mono_captures,
            params: mono_params,
            ret: ret.clone(),
            body: MonoExpr {
                kind: MonoExprKind::Unit,
                ty: ret,
            },
        });
        let mono_body = self.subst_expr(body, subst);
        self.functions[id.0 as usize].body = mono_body;
        id
    }

    pub(super) fn instantiate_adapter(&mut self, target: MonoFnId) -> MonoFnId {
        if let Some(&adapter) = self.adapters.get(&target) {
            return adapter;
        }
        let target_fn = &self.functions[target.0 as usize];
        let params = target_fn.params.clone();
        let ret = target_fn.ret.clone();
        let args = params
            .iter()
            .map(|param| MonoExpr {
                kind: MonoExprKind::Local(param.local),
                ty: param.ty.clone(),
            })
            .collect();
        let id = MonoFnId(self.functions.len() as u32);
        self.functions.push(MonoFunction {
            id,
            name: Symbol::new(format!("fn_adapter_{}", target.0)),
            owner: None,
            is_closure: true,
            self_param: None,
            self_ty: None,
            self_local: None,
            captures: Vec::new(),
            params,
            ret: ret.clone(),
            body: MonoExpr {
                kind: MonoExprKind::CallStatic {
                    fn_id: target,
                    args,
                },
                ty: ret,
            },
        });
        self.adapters.insert(target, id);
        id
    }

    pub(super) fn subst_exprs(
        &mut self,
        exprs: &[HirExpr],
        subst: &HashMap<Symbol, Type>,
    ) -> Vec<MonoExpr> {
        exprs.iter().map(|e| self.subst_expr(e, subst)).collect()
    }

    /// `receiver`/`args` are already substituted; only the target method
    /// still needs resolving now that `receiver.ty` is concrete.
    pub(super) fn resolve_generic_method_call(
        &mut self,
        receiver: MonoExpr,
        method_name: &Symbol,
        is_static: bool,
        args: Vec<MonoExpr>,
        generic_args: &[Type],
        result_ty: Type,
    ) -> MonoExprKind {
        if !is_static && method_name.as_str() == "into_string" && args.is_empty() {
            match &receiver.ty {
                Type::Primitive(_) => return MonoExprKind::ToString(Box::new(receiver)),
                Type::String => return receiver.kind,
                _ => {}
            }
        }
        let owner = owner_def_id(&receiver.ty, self.hir.array_owner).unwrap_or_else(|| {
            panic!("monomorphization: generic method call's receiver substituted to non-nominal type {:?} (see this crate's module docs)", receiver.ty)
        });
        // `receiver.ty` is already fully substituted here (the caller
        // always passes an already-`subst_expr`'d receiver) — its owner
        // arguments are the exact-match key an `impl Owner<ConcreteArgs>`
        // specialization was registered under, so this is the one place
        // that override actually takes effect, including for a call site
        // written inside another still-generic function: *that* function
        // only reaches here once monomorphized for one concrete
        // instantiation, at which point `receiver.ty` is concrete too
        // (`nether_hir::MethodFnSet::for_args`; mirrors `nether_typecheck`
        // picking the same override at typecheck time whenever the
        // receiver was already concrete there too).
        let receiver_owner_args: &[Type] = match &receiver.ty {
            Type::Struct(_, args) | Type::TupleStruct(_, args) | Type::Enum(_, args) => args,
            Type::Array(elem) => std::slice::from_ref(elem.as_ref()),
            _ => &[],
        };
        let target_hir_id = self
            .hir
            .methods
            .get(&(owner, method_name.clone()))
            .and_then(|set| set.for_args(receiver_owner_args))
            .unwrap_or_else(|| panic!("monomorphization: no impl of method `{method_name}` found for the substituted receiver type (typecheck should have rejected this earlier)"));
        let mut target_generic_args = receiver_owner_args.to_vec();
        target_generic_args.extend_from_slice(generic_args);
        let mut full_args = Vec::with_capacity(args.len() + usize::from(!is_static));
        if !is_static {
            full_args.push(receiver.clone());
        }
        full_args.extend(args);
        let mono_id = self.resolve_call(target_hir_id, &full_args, &target_generic_args);
        let call = MonoExprKind::CallStatic {
            fn_id: mono_id,
            args: full_args,
        };
        if is_static {
            MonoExprKind::Block(
                vec![MonoStmt {
                    kind: MonoStmtKind::Expr(receiver),
                }],
                Some(Box::new(MonoExpr {
                    kind: call,
                    ty: result_ty,
                })),
            )
        } else {
            call
        }
    }
}
