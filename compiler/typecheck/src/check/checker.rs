use super::*;

pub(super) struct Checker<'a> {
    pub(super) resolved: &'a ResolvedNames,
    pub(super) sigs: &'a Signatures,
    pub(super) decls: &'a DeclIndex<'a>,
    pub(super) expr_types: &'a mut HashMap<NodeId, Type>,
    pub(super) local_types: &'a mut HashMap<NodeId, Type>,
    pub(super) call_generic_args: &'a mut HashMap<NodeId, Vec<Type>>,
    pub(super) diagnostics: &'a mut Vec<Diagnostic>,
    pub(super) locals: HashMap<nether_resolver::LocalId, (Type, bool)>,
    pub(super) generics: HashMap<Symbol, Option<GenericBound>>,
    pub(super) return_ty: Type,
    pub(super) loop_depth: usize,
}

impl<'a> Checker<'a> {
    pub(super) fn new(
        resolved: &'a ResolvedNames,
        sigs: &'a Signatures,
        decls: &'a DeclIndex<'a>,
        expr_types: &'a mut HashMap<NodeId, Type>,
        local_types: &'a mut HashMap<NodeId, Type>,
        call_generic_args: &'a mut HashMap<NodeId, Vec<Type>>,
        diagnostics: &'a mut Vec<Diagnostic>,
    ) -> Self {
        Checker {
            resolved,
            sigs,
            decls,
            expr_types,
            local_types,
            call_generic_args,
            diagnostics,
            locals: HashMap::new(),
            generics: HashMap::new(),
            return_ty: Type::unit(),
            loop_depth: 0,
        }
    }

    /// Records a binding site's final type (see [`TypedTables::local_types`]
    /// docs) alongside registering it for use within this function's own
    /// body-checking (`self.locals`).
    pub(super) fn bind_local(&mut self, site: NodeId, ty: Type, mutable: bool) {
        if let Some(local_id) = self.resolved.locals.get(&site) {
            self.locals.insert(*local_id, (ty.clone(), mutable));
        }
        self.local_types.insert(site, ty);
    }

    pub(super) fn err(&mut self, span: Span, message: impl Into<String>) {
        self.diagnostics
            .push(Diagnostic::error(message).with_label(span, "here"));
    }

    pub(super) fn describe(&self, ty: &Type) -> String {
        describe_type(ty, self.resolved)
    }

    pub(super) fn lower_call_generic_args(&mut self, args: &[TypeExpr]) -> Vec<Type> {
        args.iter()
            .map(|arg| {
                let ty = lower_type_expr(arg, self.resolved, self.decls, self.diagnostics);
                self.validate_type_bounds(&ty, arg.span());
                ty
            })
            .collect()
    }

    pub(super) fn check_fn_decl(&mut self, f: &FnDecl, sig: &FnSig, self_ty: Option<Type>) {
        self.generics = sig.generics.iter().cloned().collect();
        self.locals.clear();
        if let Some(ty) = self_ty {
            let mutable = matches!(f.self_param, Some(nether_ast::SelfParam::ByMutRef));
            self.bind_local(f.id, ty, mutable);
        }
        for (param_ast, param_sig) in f.params.iter().zip(&sig.params) {
            self.validate_type_bounds(&param_sig.ty, param_ast.ty.span());
            // `param_sig.ty` is the *element* type for a variadic
            // parameter — the body sees an ordinary `Array<element>`
            // local, matching what the call site actually passes in
            // (`nether_hir::lower::lower_variadic_aware_args`).
            let local_ty = if param_sig.variadic {
                Type::Array(Box::new(param_sig.ty.clone()))
            } else {
                param_sig.ty.clone()
            };
            self.bind_local(param_ast.id, local_ty, param_ast.mutable);
        }
        self.return_ty = sig.ret.clone();
        if let Some(ret) = &f.ret {
            self.validate_type_bounds(&self.return_ty.clone(), ret.span());
        }
        if let Some(body) = &f.body {
            let expected = self.return_ty.clone();
            let body_ty = self.check_block_with_expected(body, Some(&expected));
            if !body_ty.compatible(&self.return_ty) {
                let expected = self.describe(&self.return_ty.clone());
                let found = self.describe(&body_ty);
                self.err(
                    body.span,
                    format!("expected return type `{expected}`, found `{found}`"),
                );
            }
        }
    }

    pub(super) fn check_block(&mut self, block: &Block) -> Type {
        self.check_block_with_expected(block, None)
    }

    pub(super) fn check_block_with_expected(
        &mut self,
        block: &Block,
        expected: Option<&Type>,
    ) -> Type {
        // No explicit tail means the block would ordinarily be `()` — but
        // if a statement unconditionally diverges (`return`/`break`/
        // `continue`, `Type::Never`), the block never actually falls
        // through to "after the last statement" at all, so its type
        // should be `Never` too (compatible with anything, per
        // `Type::compatible`'s own doc) rather than a spurious `()`. There
        // is no dead-code diagnostic in this language, so a diverging
        // statement isn't necessarily the *last* one — track any, not
        // just the final one.
        let mut diverges = false;
        for stmt in &block.stmts {
            if matches!(self.check_stmt(stmt), Type::Never) {
                diverges = true;
            }
        }
        match &block.tail {
            Some(tail) => self.check_expr_with_expected(tail, expected),
            None if diverges => Type::Never,
            None => Type::unit(),
        }
    }

    pub(super) fn check_stmt(&mut self, stmt: &Stmt) -> Type {
        match stmt {
            Stmt::Let(let_stmt) => {
                let declared_ty = let_stmt
                    .ty
                    .as_ref()
                    .map(|t| lower_type_expr(t, self.resolved, self.decls, self.diagnostics));
                let has_declared_type = declared_ty.is_some();
                let value_ty = self.check_expr_with_expected(&let_stmt.value, declared_ty.as_ref());
                let diverges = matches!(value_ty, Type::Never);
                let final_ty = match declared_ty {
                    Some(declared) => {
                        if !value_ty.compatible(&declared) {
                            let expected = self.describe(&declared);
                            let found = self.describe(&value_ty);
                            self.err(
                                let_stmt.value.span,
                                format!("expected `{expected}`, found `{found}`"),
                            );
                        }
                        declared
                    }
                    None => value_ty,
                };
                if !has_declared_type && !final_ty.is_error() && final_ty.contains_error() {
                    self.err(
                        let_stmt.value.span,
                        "cannot infer all generic type arguments from this initializer; add a type annotation",
                    );
                }
                self.bind_local(let_stmt.id, final_ty, let_stmt.mutable);
                if diverges {
                    Type::Never
                } else {
                    Type::unit()
                }
            }
            Stmt::Expr(expr) => {
                let ty = self.check_expr(expr);
                if !ty.is_error() && ty.contains_error() {
                    self.err(
                        expr.span,
                        "cannot infer all generic type arguments for this expression",
                    );
                }
                ty
            }
        }
    }
}
