use std::collections::HashSet;

use nether_ast::SelfParam;
use nether_hir::{HirLocalId, HirPattern};
use nether_monomorphization::{
    MonoExpr, MonoExprKind, MonoFunction, MonoMatchArm, MonoModule, MonoStmtKind,
};
use nether_resolver::Definitions;
use nether_typecheck::{alloc_kind, PrimitiveKind, Signatures, Type};

use crate::node::{
    BasicBlock, BlockId, CallTarget, Instr, Local, LocalDecl, MirFunction, Operand, Place,
    Projection, Rvalue, Terminator,
};

mod control;
mod expr;
mod helpers;
mod pattern;
mod place;

use helpers::*;

/// Lowers every function in `module` into an explicit control-flow graph
/// (`docs/architecture/crates.md` § `compiler/mir`). `defs`/`sigs` are
/// `resolver`/`typecheck`'s own tables (not carried forward by
/// `nether_monomorphization::MonoModule`), needed for allocation
/// classification and enum variant payload types during `match` lowering.
///
/// The returned `Vec<MirFunction>` is indexed by the same
/// `nether_monomorphization::MonoFnId` every `CallTarget::Fn` here refers
/// to — this crate mints no separate function-id space of its own.
///
/// Scope-exit `Release`s (`arc-model.md` §3.2) are emitted directly by
/// this pass rather than deferred to `crate::insert_arc`, unlike the
/// bind-time-retain and call-argument rules — a deliberate difference
/// from `crates.md`'s loose "build_mir produces zero Retain/Release"
/// sketch: once a `Block` expression's nested `let`s are flattened into
/// this function's single `locals` vector, there is no way to recover
/// *which* locals belonged to *which* lexical scope from the flat CFG
/// alone, so those releases have to be placed while that structure is
/// still available (here). `insert_arc` handles the two rules that
/// genuinely are recoverable as a pattern over already-built, scope-blind
/// MIR: new-binding retains and call-argument retain/release.
pub fn build_mir(module: &MonoModule, defs: &Definitions, sigs: &Signatures) -> Vec<MirFunction> {
    module
        .functions
        .iter()
        .map(|f| FnBuilder::new(module, defs, sigs).build(f))
        .collect()
}

struct LoopCtx {
    break_block: BlockId,
    continue_block: BlockId,
    /// `scopes.len()` at loop entry — `break`/`continue` release every
    /// scope entered since, but never reach past this depth.
    scope_depth: usize,
}

struct BlockBuilder {
    instrs: Vec<Instr>,
    terminator: Option<Terminator>,
}

struct FnBuilder<'a> {
    module: &'a MonoModule,
    defs: &'a Definitions,
    sigs: &'a Signatures,
    locals: Vec<LocalDecl>,
    local_map: Vec<Option<Local>>,
    borrowed_params: HashSet<Local>,
    blocks: Vec<BlockBuilder>,
    current: BlockId,
    /// A stack of lexical `Block` scopes, each holding the heap-kind
    /// locals declared by a `let` directly in it (stack-kind locals never
    /// need releasing at all, so they're simply not tracked here). The
    /// outermost entry is pushed for the function's parameters before its
    /// body is lowered, so a `return` unwinding "every scope" also
    /// releases them, matching `arc-model.md`'s worked example releasing
    /// the parameter at function exit.
    scopes: Vec<Vec<Local>>,
    loop_stack: Vec<LoopCtx>,
}

impl<'a> FnBuilder<'a> {
    fn new(module: &'a MonoModule, defs: &'a Definitions, sigs: &'a Signatures) -> Self {
        FnBuilder {
            module,
            defs,
            sigs,
            locals: Vec::new(),
            local_map: Vec::new(),
            borrowed_params: HashSet::new(),
            blocks: Vec::new(),
            current: BlockId(0),
            scopes: Vec::new(),
            loop_stack: Vec::new(),
        }
    }

    fn build(mut self, f: &MonoFunction) -> MirFunction {
        let mut params = Vec::new();
        let mut param_scope = Vec::new();
        let mut closure_captures = Vec::new();
        for capture in &f.captures {
            let local = self.declare_local(capture.ty.clone(), false);
            self.bind_hir_local(capture.local, local);
            closure_captures.push(local);
        }
        if let Some(self_id) = f.self_local {
            let owner_ty = f.self_ty.clone().unwrap_or(Type::Error);
            let ty = match f.self_param {
                Some(SelfParam::Owned) => Type::Unique(Box::new(owner_ty)),
                Some(SelfParam::OwnedRef) => Type::Ref(Box::new(owner_ty)),
                Some(SelfParam::OwnedMutRef) => Type::MutRef(Box::new(owner_ty)),
                _ => owner_ty,
            };
            let mutable = matches!(
                f.self_param,
                Some(SelfParam::ByMutRef | SelfParam::OwnedMutRef)
            );
            let local = self.declare_local(ty.clone(), mutable);
            self.bind_hir_local(self_id, local);
            if mutable {
                self.borrowed_params.insert(local);
            }
            if self.sigs.has_managed_content(&ty, self.defs) && !mutable {
                param_scope.push(local);
            }
            params.push(local);
        }
        for p in &f.params {
            let local = self.declare_local(p.ty.clone(), p.mutable);
            self.bind_hir_local(p.local, local);
            if p.mutable {
                self.borrowed_params.insert(local);
            }
            if self.sigs.has_managed_content(&p.ty, self.defs) && !p.mutable {
                param_scope.push(local);
            }
            params.push(local);
        }
        self.scopes.push(param_scope);

        self.current = self.new_block();
        let entry = self.current;
        // `f.body` is always itself a `Block`, whose own `lower_block`
        // call already resolved its own escaping tail value correctly
        // (including any parameter-retain, via `lower_escaping_value`) —
        // this step only needs to release the parameter scope
        // (`lower_block` already popped and released its own).
        let body_diverges = matches!(f.body.ty, Type::Never);
        let result = self.lower_expr(&f.body);
        if body_diverges {
            // The body's own control flow already reached a `return`/
            // `break`/`continue` on every path (`Type::Never`, mirrors
            // `nether_typecheck::check::check_block_with_expected`) — the
            // block `self.current` now points at is unreachable dead
            // code, reached only via `terminate_current`'s "fresh block
            // after a terminator" bookkeeping. `lower_block`'s own
            // `Operand::Unit` placeholder for "no tail" would otherwise
            // get used as this function's `Terminator::Return` operand
            // here, which fails LLVM verification whenever `f.ret` isn't
            // itself `()` — LLVM requires every terminator to be
            // well-typed for its function even in an unreachable block.
            self.terminate_current(Terminator::Unreachable);
        } else {
            self.release_scopes(0, escaping_local(&result));
            self.terminate_current(Terminator::Return(result));
        }

        let blocks = self
            .blocks
            .into_iter()
            .enumerate()
            .map(|(i, b)| BasicBlock {
                id: BlockId(i as u32),
                instrs: b.instrs,
                terminator: b.terminator.unwrap_or(Terminator::Unreachable),
            })
            .collect();

        MirFunction {
            id: f.id,
            name: f.name.clone(),
            owner: f.owner,
            is_closure: f.is_closure,
            is_async: f.is_async,
            closure_captures,
            params,
            ret: f.ret.clone(),
            locals: self.locals,
            blocks,
            entry,
        }
    }

    fn declare_local(&mut self, ty: Type, mutable: bool) -> Local {
        let alloc = alloc_kind(&ty, self.defs);
        let needs_drop = self.sigs.has_managed_content(&ty, self.defs);
        let id = Local(self.locals.len() as u32);
        self.locals.push(LocalDecl {
            ty,
            mutable,
            alloc,
            needs_drop,
        });
        id
    }

    fn new_block(&mut self) -> BlockId {
        let id = BlockId(self.blocks.len() as u32);
        self.blocks.push(BlockBuilder {
            instrs: Vec::new(),
            terminator: None,
        });
        id
    }

    fn push_instr(&mut self, instr: Instr) {
        self.blocks[self.current.0 as usize].instrs.push(instr);
    }

    /// Sets `current`'s terminator and moves `current` to a fresh, empty
    /// block — anything lowered after a diverging expression
    /// (`return`/`break`/`continue`) lands in that fresh block, which
    /// simply never gets reached, so it's left to default to
    /// `Terminator::Unreachable` in [`Self::build`] unless something else
    /// (a `match` arm's fallthrough, say) claims it first.
    fn terminate_current(&mut self, term: Terminator) {
        let cur = self.current;
        self.blocks[cur.0 as usize].terminator = Some(term);
        self.current = self.new_block();
    }

    fn materialize(&mut self, rvalue: Rvalue, ty: Type) -> Local {
        let local = self.declare_local(ty, false);
        self.push_instr(Instr::Assign(Place::local(local), rvalue));
        local
    }

    fn as_local(&mut self, op: Operand, ty: Type) -> Local {
        match op {
            Operand::Local(l) => l,
            other => self.materialize(Rvalue::Use(other), ty),
        }
    }

    /// Evaluates `value` for binding into a *new*, independent heap-kind
    /// owner — a `let`, a `match`'s own scrutinee slot, an `if`/`match`
    /// arm's contribution to its shared result slot, or a field/index
    /// store's incoming value. This is the one place this crate decides
    /// whether that binding needs a freshly-inserted `Retain` of its own.
    ///
    /// A bare reference to an *already-named* local ([`is_trivial_local_alias`]
    /// — a parameter, or another `let`/pattern binding) always needs one:
    /// that source keeps its own independent life afterward (or, for a
    /// parameter, never had an independent credit to begin with — see
    /// [`Self::lower_escaping_value`]'s docs), so this binds a retained *copy* via
    /// `Rvalue::Use`, with the `Retain` inserted directly, right here —
    /// deliberately not left for `insert_arc` to infer from the `Use`
    /// itself, which is what let an earlier version of this crate
    /// double-count it (see `crate::arc`'s module docs).
    ///
    /// Anything else (a field/index read, a call, a construction, an
    /// `if`/`match` whose own result this same rule already credited
    /// correctly, ...) is returned as `lower_expr` produced it, unwrapped:
    /// each of those already carries exactly the right credit from its
    /// own evaluation (an aliasing container-read retains itself via
    /// `insert_arc`'s `Field`/`VariantField`/`Index` rule; a fresh
    /// construction or a call's result starts pre-owned) — binding it to
    /// a second name here needs no further action.
    fn prepare_new_binding(&mut self, value: &MonoExpr) -> Operand {
        if is_trivial_local_alias(value) {
            let op = self.lower_expr(value);
            let moved_source = match op {
                Operand::Local(source)
                    if matches!(self.locals[source.index()].ty, Type::Unique(_)) =>
                {
                    Some(source)
                }
                _ => None,
            };
            let local = self.materialize(Rvalue::Use(op), value.ty.clone());
            if self.sigs.has_managed_content(&value.ty, self.defs)
                && !matches!(value.ty, Type::Unique(_))
                && moved_source.is_none()
            {
                self.push_instr(Instr::Retain(local));
            }
            if let Some(source) = moved_source {
                self.push_instr(Instr::Clear(source));
            }
            Operand::Local(local)
        } else {
            self.lower_expr(value)
        }
    }

    /// Releases every heap-kind local in scopes `[from_depth..]` (deepest
    /// scope first, and within a scope, its locals in reverse declaration
    /// order — `arc-model.md` §3.2's drop order), skipping exactly one
    /// occurrence of `skip`.
    fn release_scopes(&mut self, from_depth: usize, mut skip: Option<Local>) {
        let mut to_release = Vec::new();
        for scope in self.scopes[from_depth..].iter().rev() {
            to_release.extend(scope.iter().rev().copied());
        }
        for local in to_release {
            if skip == Some(local) {
                skip = None;
                continue;
            }
            self.push_instr(Instr::Release(local));
        }
    }

    /// Evaluates `expr` as a value escaping *this* scope (a `Block`'s own
    /// tail, or a `return`'s operand). A bare local reference (parameter
    /// or otherwise) needs no extra `Retain` here: `release_scopes`'
    /// `skip` handling already leaves whatever local is chosen un
    /// -released at this scope's exit, so its existing credit simply
    /// passes through as the escaping value's own — for an ordinary `let`
    /// -bound local that credit came from its own bind-time retain; for a
    /// bare **parameter**, it's the credit the caller's own pre-call
    /// retain gave it (`arc-model.md` §3.3, `nether_mir::arc`'s own
    /// module docs) — a `Call` never also releases that credit after the
    /// call returns, precisely so it can be handed through here instead
    /// of needing a second, manufactured one.
    fn lower_escaping_value(&mut self, expr: &MonoExpr) -> Operand {
        if let MonoExprKind::Local(id) = &expr.kind {
            let source = self.local_for(*id);
            if self.borrowed_params.contains(&source)
                && self.sigs.has_managed_content(&expr.ty, self.defs)
            {
                let local = self.materialize(Rvalue::Use(Operand::Local(source)), expr.ty.clone());
                self.push_instr(Instr::Retain(local));
                return Operand::Local(local);
            }
        }
        self.lower_expr(expr)
    }

    fn bind_hir_local(&mut self, id: HirLocalId, local: Local) {
        if self.local_map.len() <= id.index() {
            self.local_map.resize(id.index() + 1, None);
        }
        self.local_map[id.index()] = Some(local);
    }

    fn local_for(&self, id: HirLocalId) -> Local {
        self.local_map
            .get(id.index())
            .and_then(|local| *local)
            .unwrap_or_else(|| panic!("mir: reference to a local before it was bound — nether_hir's own invariant guarantees this can't happen for a well-typed program"))
    }
}
