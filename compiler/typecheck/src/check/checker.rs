use super::*;

pub(super) struct Checker<'a> {
    pub(super) resolved: &'a ResolvedNames,
    pub(super) sigs: &'a Signatures,
    pub(super) decls: &'a DeclIndex<'a>,
    pub(super) expr_types: &'a mut HashMap<NodeId, Type>,
    pub(super) local_types: &'a mut HashMap<NodeId, Type>,
    pub(super) call_generic_args: &'a mut HashMap<NodeId, Vec<Type>>,
    pub(super) diagnostics: &'a mut Vec<Diagnostic>,
    pub(super) locals: HashMap<LocalId, (Type, bool)>,
    /// Move-tracking state for the unique-ownership domain (language-spec
    /// §3, move-checking module docs) — present only for locals whose
    /// declared type is `Type::Unique(_)`; a local's absence here always
    /// means "live," never "not tracked" vs. "live" ambiguity, since
    /// non-unique locals are simply never inserted at all. `Some(span)` is
    /// where it was moved.
    pub(super) moved: HashMap<LocalId, Span>,
    /// Set only while re-walking a loop body's *first*, silent pass
    /// (`Checker::check_loop_body_with_fixpoint`) — every diagnostic-
    /// producing method (`Self::err`, `Self::check_move`) becomes a no-op
    /// while this is `true`, since that pass exists purely to discover
    /// which locals the body moves, not to report anything (the second,
    /// real pass — seeded with what the first one found — reports
    /// everything, without double-reporting what both passes would
    /// otherwise flag identically).
    pub(super) suppress_diagnostics: bool,
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
            moved: HashMap::new(),
            suppress_diagnostics: false,
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
            // A `let`/parameter binding always starts a fresh value —
            // clear any move-state a same-`LocalId` binding might have
            // picked up. Necessary (not just tidy) for a `let` declared
            // *inside* a loop body: `Checker::check_loop_body_with_fixpoint`
            // re-walks that one static binding site more than once to
            // model successive iterations, and each one must see a live
            // value, not the previous pass's leftover move-state.
            self.moved.remove(local_id);
        }
        self.local_types.insert(site, ty);
    }

    pub(super) fn err(&mut self, span: Span, message: impl Into<String>) {
        if self.suppress_diagnostics {
            return;
        }
        self.diagnostics
            .push(Diagnostic::error(message).with_label(span, "here"));
    }

    /// Checks a use of `local_id` (already known to have static type `ty`)
    /// against its current move-state, then — only if `consumes` is
    /// true — records it as moved at `span`. A no-op for anything but a
    /// `Type::Unique(_)`-typed local: ordinary `T`/`t` values are freely
    /// copyable/aliasable (language-spec §3), so they're never tracked at
    /// all (never inserted into `self.moved`, not merely always "live").
    ///
    /// `consumes` distinguishes a value being *read as itself* (a bare
    /// local reference used as an rvalue, or a `: self`-consuming method
    /// receiver) from a *read-through* (the base of a field/index
    /// projection, or a `: &self`/`: &mut self` borrowing receiver) — the
    /// latter still requires the local to be live, but doesn't itself
    /// consume it (language-spec §4.2: struct/tuple fields are always
    /// ordinary-typed, never themselves `Type::Unique`, so projecting
    /// through one never moves the aggregate it came from).
    pub(super) fn check_move(&mut self, local_id: LocalId, ty: &Type, span: Span, consumes: bool) {
        if !matches!(ty, Type::Unique(_)) {
            return;
        }
        if let Some(&prior) = self.moved.get(&local_id) {
            if !self.suppress_diagnostics {
                self.diagnostics.push(
                    Diagnostic::error("use of a value after it was moved")
                        .with_label(prior, "value moved here")
                        .with_label(span, "used again here, after the move"),
                );
            }
            return;
        }
        if consumes {
            self.moved.insert(local_id, span);
        }
    }

    /// Runs `f` against a private copy of the current move-state, restores
    /// the real state afterward, and returns `f`'s result alongside the
    /// move-state `f` produced — the building block both branch-merging
    /// ([`Self::merge_branches`]) and loop fixpoint analysis use, so
    /// checking one arm/pass never leaks its moves into a sibling one that
    /// didn't actually run after it.
    pub(super) fn moved_snapshot<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> T,
    ) -> (T, HashMap<LocalId, Span>) {
        let saved = self.moved.clone();
        let result = f(self);
        let branch_moved = std::mem::replace(&mut self.moved, saved);
        (result, branch_moved)
    }

    /// Merges the move-states of mutually exclusive branches (`if`/`else`,
    /// `match` arms) back into `self.moved`: a local is moved after the
    /// merge if it was moved on *any* branch that doesn't itself
    /// unconditionally diverge (a diverging branch's own moves can never
    /// actually reach the code after the merge, so they're excluded —
    /// mirrors `Type::Never`'s own "doesn't force sibling arms" treatment
    /// elsewhere in this checker). This is deliberately the conservative
    /// direction: a value moved on only *some* live-reaching branches
    /// counts as moved afterward (a "possibly moved" use is still
    /// rejected), not the reverse.
    pub(super) fn merge_branches(&mut self, branches: Vec<(bool, HashMap<LocalId, Span>)>) {
        for (diverges, moved) in branches {
            if diverges {
                continue;
            }
            for (id, span) in moved {
                self.moved.entry(id).or_insert(span);
            }
        }
    }

    /// Type-checks a loop body (`while`/`loop`/`for`-`in`) with a two-pass
    /// fixpoint so a value moved unconditionally inside the body is caught
    /// even on a hypothetical *next* iteration, not only textually later
    /// in the same one — the loop's own body is the only AST subtree this
    /// checker ever visits more than once.
    ///
    /// Pass 1 runs silently (`Self::suppress_diagnostics`) purely to
    /// discover which locals the body moves at all; pass 2 re-checks for
    /// real, seeded with pass 1's ending move-state as if it were "already
    /// true when this iteration started" — so a body that moves a
    /// captured-from-outside local and then (on what pass 2 models as the
    /// next go-around) uses it again is flagged, while a `let` declared
    /// *inside* the body is unaffected (`Checker::bind_local` clears
    /// move-state for its own `LocalId` the moment pass 2 reaches it,
    /// modeling that iteration's fresh binding).
    ///
    /// After both passes, this body's own net moves are folded into the
    /// caller's state the same way a branch's are (`Self::merge_branches`)
    /// — the loop might execute zero times, so nothing inside it is ever
    /// *unconditionally* moved from the perspective of code after the
    /// loop.
    pub(super) fn check_loop_body_with_fixpoint(&mut self, mut f: impl FnMut(&mut Self)) {
        let pre = self.moved.clone();
        let was_suppressed = self.suppress_diagnostics;
        self.suppress_diagnostics = true;
        f(self);
        self.suppress_diagnostics = was_suppressed;
        let discovered = std::mem::replace(&mut self.moved, pre.clone());
        for (id, span) in &discovered {
            self.moved.entry(*id).or_insert(*span);
        }
        f(self);
        let after_body = std::mem::replace(&mut self.moved, pre);
        self.merge_branches(vec![(false, after_body)]);
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
            // `ByMutRef` (`mut self`) and `OwnedMutRef` (`: &mut self`)
            // both grant mutation permission on `self` — the ARC and
            // owned-domain exclusive-receiver forms respectively.
            let mutable = matches!(
                f.self_param,
                Some(nether_ast::SelfParam::ByMutRef | nether_ast::SelfParam::OwnedMutRef)
            );
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
            // `param_ast.mutable` is the *ARC* `name mut Type` marker —
            // orthogonal to a `:&mut T` parameter's own, always-exclusive
            // mutation permission (Stage 2, slice 2), so a `Type::MutRef`
            // param counts as mutable here regardless of that marker.
            let mutable = param_ast.mutable || matches!(param_sig.ty, Type::MutRef(_));
            self.bind_local(param_ast.id, local_ty, mutable);
        }
        self.return_ty = sig.ret.clone();
        if let Some(ret) = &f.ret {
            self.validate_type_bounds(&self.return_ty.clone(), ret.span());
        }
        if let Some(body) = &f.body {
            let body_ty = self.check_fn_body(body);
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

    /// Checks the outermost block of a function or method body.
    ///
    /// Unlike an ordinary block expression, this block never yields its
    /// syntactic tail: Nether requires an explicit `return` for function
    /// results. The tail is still checked as a discarded expression so its
    /// names, calls, moves, and other effects remain semantically visible.
    fn check_fn_body(&mut self, block: &Block) -> Type {
        let mut diverges = false;
        for stmt in &block.stmts {
            if matches!(self.check_stmt(stmt), Type::Never) {
                diverges = true;
            }
        }
        if let Some(tail) = &block.tail {
            if matches!(self.check_expr(tail), Type::Never) {
                diverges = true;
            }
        }
        if diverges {
            Type::Never
        } else {
            Type::unit()
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
