use super::*;

pub(super) struct Lowerer<'a> {
    pub(super) resolved: &'a ResolvedNames,
    pub(super) expr_types: &'a HashMap<NodeId, Type>,
    pub(super) local_types_by_id: &'a HashMap<ResolverLocalId, Type>,
    pub(super) call_generic_args: &'a HashMap<NodeId, Vec<Type>>,
    pub(super) sigs: &'a Signatures,
    pub(super) fn_by_def: &'a HashMap<DefId, HirFnId>,
    pub(super) methods: &'a HashMap<(DefId, Symbol), MethodFnSet>,
    pub(super) locals_map: HashMap<ResolverLocalId, HirLocalId>,
    pub(super) next_local: u32,
    pub(super) generics: HashMap<Symbol, Option<GenericBound>>,
    pub(super) type_subst: HashMap<Symbol, Type>,
    pub(super) self_override: Option<(ResolverLocalId, Type)>,
    pub(super) array_owner: Option<DefId>,
}

impl Lowerer<'_> {
    pub(super) fn fresh_local(&mut self) -> HirLocalId {
        let id = HirLocalId(self.next_local);
        self.next_local += 1;
        id
    }

    pub(super) fn local_for(&mut self, orig: ResolverLocalId) -> HirLocalId {
        if let Some(id) = self.locals_map.get(&orig) {
            return *id;
        }
        let id = self.fresh_local();
        self.locals_map.insert(orig, id);
        id
    }

    pub(super) fn ty_of(&self, node_id: NodeId) -> Type {
        let ty = self
            .expr_types
            .get(&node_id)
            .cloned()
            .unwrap_or(Type::Error);
        subst_type(&ty, &self.type_subst)
    }

    pub(super) fn generic_args_for(&self, node_id: NodeId) -> Vec<Type> {
        self.call_generic_args
            .get(&node_id)
            .into_iter()
            .flatten()
            .map(|ty| subst_type(ty, &self.type_subst))
            .collect()
    }

    pub(super) fn local_ty(&self, orig: ResolverLocalId) -> Type {
        if let Some((self_local, ty)) = &self.self_override {
            if *self_local == orig {
                return ty.clone();
            }
        }
        let ty = self
            .local_types_by_id
            .get(&orig)
            .cloned()
            .unwrap_or(Type::Error);
        subst_type(&ty, &self.type_subst)
    }

    pub(super) fn lower_fn(&mut self, p: &PendingFn, id: HirFnId) -> HirFunction {
        self.locals_map.clear();
        self.next_local = 0;
        self.type_subst.clone_from(&p.type_subst);
        self.self_override = self.resolved.locals.get(&p.decl.id).copied().zip(
            p.owner
                .map(|owner| owner_type(owner, self.sigs, self.array_owner)),
        );

        let mut params = Vec::new();
        for (param_ast, param_sig) in p.decl.params.iter().zip(&p.sig.params) {
            let local = match self.resolved.locals.get(&param_ast.id) {
                Some(orig) => self.local_for(*orig),
                None => self.fresh_local(),
            };
            // `param_sig.ty` is the *element* type for a variadic
            // parameter — the callee's own body (and its actual runtime
            // ABI) sees an ordinary `Array<element>` value, matching what
            // `lower_variadic_aware_args` collects at each call site.
            let ty = if param_sig.variadic {
                Type::Array(Box::new(param_sig.ty.clone()))
            } else {
                param_sig.ty.clone()
            };
            params.push(HirParam {
                local,
                name: param_sig.name.clone(),
                mutable: param_sig.mutable,
                ty,
            });
        }
        // `self` isn't in `params` (matching FnSig's own self/params
        // split) but still needs its translated id reserved up front so
        // body references resolve consistently.
        let self_local = self
            .resolved
            .locals
            .get(&p.decl.id)
            .map(|orig| self.local_for(*orig));

        let body = p.decl.body.as_ref().expect(
            "standalone fns, impl methods, and inherited interface defaults always have a body",
        );
        let body_hir = self.lower_block_as_expr(body);

        HirFunction {
            id,
            name: p.name.clone(),
            owner: p.owner,
            self_param: p.decl.self_param,
            // A concrete specialization's `self` is already fully
            // concrete (`Option<i32>`, not a generic placeholder) — its
            // own `sig.generics` has no owner parameters to substitute
            // one in from (`owner_generics_for_impl`'s Case C), so
            // `owner_type`'s ordinary generic-placeholder shape would
            // otherwise leak an unsubstituted `Type::Generic` straight
            // through `monomorphization` (nothing in `finish_instantiation`'s
            // empty subst map would ever replace it) into codegen's ICE.
            self_ty: p.owner.map(|owner| match &p.specialization {
                Some(args) => concrete_owner_type(owner, self.sigs, self.array_owner, args),
                None => owner_type(owner, self.sigs, self.array_owner),
            }),
            self_local,
            generics: p.sig.generics.clone(),
            params,
            ret: p.sig.ret.clone(),
            body: body_hir,
        }
    }

    pub(super) fn lower_block_as_expr(&mut self, block: &Block) -> HirExpr {
        let (stmts, tail) = self.lower_block(block);
        // Mirrors `nether_typecheck::check::check_block_with_expected`: a
        // block with no explicit tail is ordinarily `()`, but if a
        // statement unconditionally diverges (`return`/`break`/
        // `continue`, `Type::Never`), the block's own type must be
        // `Never` too — there's no dead-code diagnostic in this language,
        // so a diverging statement isn't necessarily the last one.
        let ty = match &tail {
            Some(t) => t.ty.clone(),
            None => {
                let diverges = stmts.iter().any(|s| match &s.kind {
                    HirStmtKind::Expr(e) => matches!(e.ty, Type::Never),
                    HirStmtKind::Let { value, .. } => matches!(value.ty, Type::Never),
                });
                if diverges {
                    Type::Never
                } else {
                    Type::unit()
                }
            }
        };
        HirExpr {
            kind: HirExprKind::Block(stmts, tail),
            ty,
        }
    }

    pub(super) fn lower_block(&mut self, block: &Block) -> (Vec<HirStmt>, Option<Box<HirExpr>>) {
        let mut stmts = Vec::new();
        for stmt in &block.stmts {
            match stmt {
                Stmt::Let(let_stmt) => {
                    let value = self.lower_expr(&let_stmt.value);
                    let (local, ty) = match self.resolved.locals.get(&let_stmt.id) {
                        Some(orig) => (self.local_for(*orig), self.local_ty(*orig)),
                        None => (self.fresh_local(), value.ty.clone()),
                    };
                    stmts.push(HirStmt {
                        kind: HirStmtKind::Let { local, ty, value },
                    });
                }
                Stmt::Expr(e) => stmts.push(HirStmt {
                    kind: HirStmtKind::Expr(self.lower_expr(e)),
                }),
            }
        }
        let tail = block.tail.as_ref().map(|t| Box::new(self.lower_expr(t)));
        (stmts, tail)
    }
}
