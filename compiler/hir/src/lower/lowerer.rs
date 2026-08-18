use super::*;
use nether_ast::SelfParam;
use nether_typecheck::{alloc_kind, AllocKind};

pub(super) struct Lowerer<'a> {
    pub(super) resolved: &'a ResolvedNames,
    pub(super) expr_types: &'a HashMap<NodeId, Type>,
    pub(super) local_types_by_id: &'a HashMap<ResolverLocalId, Type>,
    pub(super) call_generic_args: &'a HashMap<NodeId, Vec<Type>>,
    pub(super) existential_coercions: &'a HashMap<NodeId, Type>,
    pub(super) sigs: &'a Signatures,
    pub(super) fn_by_def: &'a HashMap<DefId, HirFnId>,
    pub(super) methods: &'a HashMap<(DefId, Symbol, ReceiverDomain), MethodFnSet>,
    pub(super) locals_map: HashMap<ResolverLocalId, HirLocalId>,
    pub(super) next_local: u32,
    pub(super) generics: HashMap<Symbol, Vec<GenericBound>>,
    pub(super) type_subst: HashMap<Symbol, Type>,
    pub(super) self_override: Option<(ResolverLocalId, Type)>,
    pub(super) array_owner: Option<DefId>,
}

impl Lowerer<'_> {
    pub(super) fn runtime_ty(&self, ty: &Type) -> Type {
        let ty = subst_type(ty, &self.type_subst);
        match ty {
            Type::Unique(inner)
                if alloc_kind(&inner, &self.resolved.definitions) == AllocKind::Heap =>
            {
                Type::Unique(inner)
            }
            Type::Unique(inner) => *inner,
            Type::Ref(inner) | Type::MutRef(inner)
                if alloc_kind(&inner, &self.resolved.definitions) == AllocKind::Heap =>
            {
                *inner
            }
            other => other,
        }
    }

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

    /// Translates typecheck types to their runtime form. Heap `Unique` is
    /// preserved so MIR/codegen selects unique allocation and move rules;
    /// inline `Unique` is erased because it has the same bits as its inner
    /// value. Reference wrappers are preserved at storage/ABI sites and
    /// lowered to pointers.
    pub(super) fn ty_of(&self, node_id: NodeId) -> Type {
        let ty = self
            .expr_types
            .get(&node_id)
            .cloned()
            .unwrap_or(Type::Error);
        match ty {
            Type::Ref(_) | Type::MutRef(_) => subst_type(&ty, &self.type_subst),
            _ => self.runtime_ty(&ty),
        }
    }

    /// A method-call receiver's ownership domain, read directly from
    /// `expr_types` before runtime-type translation. Which
    /// `ReceiverDomain`-keyed method-set entry a call resolves to depends
    /// on the source ownership wrapper, and that
    /// information doesn't exist anywhere else once a value's type has
    /// passed through `ty_of`.
    pub(super) fn receiver_domain_of(&self, node_id: NodeId) -> ReceiverDomain {
        let ty = self
            .expr_types
            .get(&node_id)
            .cloned()
            .unwrap_or(Type::Error);
        ReceiverDomain::of_receiver_ty(&ty)
    }

    /// [`Self::receiver_domain_of`]'s counterpart for a bare local
    /// resolved directly as a path's base (`nether_resolver::Resolution::
    /// Local`, e.g. `owned_dog` in `owned_dog.greet()`) — reads
    /// `local_types_by_id` directly, bypassing `local_ty`'s stripping,
    /// for the same reason.
    pub(super) fn local_domain_of(&self, orig: ResolverLocalId) -> ReceiverDomain {
        let ty = self
            .local_types_by_id
            .get(&orig)
            .cloned()
            .unwrap_or(Type::Error);
        ReceiverDomain::of_receiver_ty(&ty)
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
        match ty {
            Type::Ref(_) | Type::MutRef(_) => subst_type(&ty, &self.type_subst),
            _ => self.runtime_ty(&ty),
        }
    }

    pub(super) fn lower_fn(&mut self, p: &PendingFn, id: HirFnId) -> HirFunction {
        self.locals_map.clear();
        self.next_local = 0;
        self.type_subst.clone_from(&p.type_subst);
        self.self_override = self
            .resolved
            .locals
            .get(&p.decl.id)
            .copied()
            .zip(p.owner.map(|owner| {
                let owner = owner_type(owner, self.sigs, self.array_owner);
                match p.decl.self_param {
                    Some(SelfParam::Owned) => Type::Unique(Box::new(owner)),
                    Some(SelfParam::OwnedRef) => Type::Ref(Box::new(owner)),
                    Some(SelfParam::OwnedMutRef) => Type::MutRef(Box::new(owner)),
                    _ => owner,
                }
            }));

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
            // `strip_indirection()` here for the same reason as
            // `Self::ty_of` above: `FnSig.params[].ty` is computed
            // straight from the AST by `typecheck::lower_type_expr`, a
            // separate path from `expr_types`/`local_types_by_id` that
            // `ty_of`/`local_ty` already strip — this is the other place a
            // raw `Type::Unique`/`Ref`/`MutRef` could otherwise leak into
            // HIR/MIR/codegen. A `:&T`/`:&mut T` parameter is `Type::Ref`/
            // `Type::MutRef` directly (never wrapped in an outer `Unique`
            // — confirmed by the parser: `:&T` parses straight to
            // `TypeExpr::Ref`, no wrapping `TypeExpr::Unique` node), so
            // this is the one place a bare (non-`Unique`) `Ref`/`MutRef`
            // needed stripping that `strip_unique()` alone would have
            // missed entirely.
            let ty = if param_sig.variadic {
                Type::FixedArray(
                    Box::new(param_sig.ty.clone()),
                    Box::new(Type::Generic(nether_typecheck::variadic_len_param())),
                )
            } else {
                match &param_sig.ty {
                    Type::Ref(_) | Type::MutRef(_) => subst_type(&param_sig.ty, &self.type_subst),
                    _ => self.runtime_ty(&param_sig.ty),
                }
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
            "standalone fns, impl methods, and inherited trait defaults always have a body",
        );
        let body_hir = self.lower_fn_body(body);

        HirFunction {
            id,
            name: p.name.clone(),
            is_async: p.sig.is_async,
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
            ret: match &p.sig.ret {
                Type::Ref(_) | Type::MutRef(_) => subst_type(&p.sig.ret, &self.type_subst),
                _ => self.runtime_ty(&p.sig.ret),
            },
            body: body_hir,
        }
    }

    /// Lowers the outermost function/method block with explicit-return-only
    /// semantics. Ordinary block expressions keep their tail value; here the
    /// syntactic tail becomes a discarded expression statement.
    fn lower_fn_body(&mut self, block: &Block) -> HirExpr {
        let (mut stmts, tail) = self.lower_block(block);
        if let Some(tail) = tail {
            stmts.push(HirStmt {
                kind: HirStmtKind::Expr(*tail),
            });
        }
        let diverges = stmts.iter().any(|stmt| match &stmt.kind {
            HirStmtKind::Expr(expr) => matches!(expr.ty, Type::Never),
            HirStmtKind::Let { value, .. } => matches!(value.ty, Type::Never),
        });
        HirExpr {
            kind: HirExprKind::Block(stmts, None),
            ty: if diverges { Type::Never } else { Type::unit() },
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
                    let mut value = self.lower_expr(&let_stmt.value);
                    let (local, ty) = match self.resolved.locals.get(&let_stmt.id) {
                        Some(orig) => (self.local_for(*orig), self.local_ty(*orig)),
                        None => (self.fresh_local(), value.ty.clone()),
                    };
                    if matches!(ty, Type::Ref(_) | Type::MutRef(_))
                        && !matches!(value.ty, Type::Ref(_) | Type::MutRef(_))
                    {
                        value = HirExpr {
                            kind: HirExprKind::Borrow(Box::new(value)),
                            ty: ty.clone(),
                        };
                    }
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
