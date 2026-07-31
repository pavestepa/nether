use std::collections::{HashMap, HashSet};

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
    local_map: HashMap<HirLocalId, Local>,
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
            local_map: HashMap::new(),
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
            self.local_map.insert(capture.local, local);
            closure_captures.push(local);
        }
        if let Some(self_id) = f.self_local {
            let ty = f.self_ty.clone().unwrap_or(Type::Error);
            let mutable = matches!(f.self_param, Some(SelfParam::ByMutRef));
            let local = self.declare_local(ty.clone(), mutable);
            self.local_map.insert(self_id, local);
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
            self.local_map.insert(p.local, local);
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
        let result = self.lower_expr(&f.body);
        self.release_scopes(0, escaping_local(&result));
        self.terminate_current(Terminator::Return(result));

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
            let local = self.materialize(Rvalue::Use(op), value.ty.clone());
            if self.sigs.has_managed_content(&value.ty, self.defs) {
                self.push_instr(Instr::Retain(local));
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

    fn local_for(&self, id: HirLocalId) -> Local {
        *self.local_map.get(&id).unwrap_or_else(|| panic!("mir: reference to a local before it was bound — nether_hir's own invariant guarantees this can't happen for a well-typed program"))
    }

    fn lower_place(&mut self, expr: &MonoExpr) -> Place {
        match &expr.kind {
            MonoExprKind::Local(id) => Place::local(self.local_for(*id)),
            MonoExprKind::Field { base, index } => {
                let mut place = self.lower_place(base);
                place.projection.push(Projection::Field(*index));
                place
            }
            MonoExprKind::Index { base, index } => {
                let mut place = self.lower_place(base);
                let index_op = self.lower_expr(index);
                place.projection.push(Projection::Index(index_op));
                place
            }
            other => panic!("mir: {other:?} is not an assignable place (typecheck should have rejected this target)"),
        }
    }

    /// Reads a place's current value as an `Operand` — the mirror image
    /// of [`Self::lower_place`], used only to read the *outgoing* value
    /// out of a field/index store before overwriting it (see the
    /// `Assign` case in [`Self::lower_expr`]).
    fn read_place(&mut self, place: &Place, ty: Type) -> Operand {
        if place.projection.is_empty() {
            return Operand::Local(place.local);
        }
        let mut current = Operand::Local(place.local);
        let mut current_ty = self.locals[place.local.index()].ty.clone();
        let mut intermediates = Vec::new();
        for (position, projection) in place.projection.iter().enumerate() {
            let projected_ty = self
                .projected_type(&current_ty, projection)
                .unwrap_or_else(|| {
                    if position + 1 == place.projection.len() {
                        ty.clone()
                    } else {
                        Type::Error
                    }
                });
            let rvalue = match projection {
                Projection::Field(index) => Rvalue::Field {
                    base: current.clone(),
                    index: *index,
                },
                Projection::Index(index) => Rvalue::Index {
                    base: current.clone(),
                    index: index.clone(),
                },
                Projection::VariantField { variant, index } => Rvalue::VariantField {
                    base: current.clone(),
                    variant: *variant,
                    index: *index,
                },
            };
            let next = self.materialize(rvalue, projected_ty.clone());
            if position > 0 {
                if let Operand::Local(previous) = current {
                    if self.sigs.has_managed_content(&current_ty, self.defs) {
                        intermediates.push(previous);
                    }
                }
            }
            current = Operand::Local(next);
            current_ty = projected_ty;
        }
        for intermediate in intermediates {
            self.push_instr(Instr::Release(intermediate));
        }
        current
    }

    fn projected_type(&self, base: &Type, projection: &Projection) -> Option<Type> {
        match projection {
            Projection::Field(index) => match base {
                Type::Tuple(items) => items.get(*index as usize).cloned(),
                _ => self.sigs.type_fields(base)?.get(*index as usize).cloned(),
            },
            Projection::VariantField { variant, index } => self
                .sigs
                .enum_payload(base, *variant)?
                .get(*index as usize)
                .cloned(),
            Projection::Index(_) => match base {
                Type::Array(elem) => Some((**elem).clone()),
                _ => None,
            },
        }
    }

    fn lower_exprs(&mut self, exprs: &[MonoExpr]) -> Vec<Operand> {
        exprs.iter().map(|e| self.lower_expr(e)).collect()
    }

    fn lower_call_args(
        &mut self,
        args: &[MonoExpr],
        params: &[Type],
    ) -> (Vec<Operand>, Vec<Option<Operand>>) {
        let mut operands = Vec::with_capacity(args.len());
        let mut weak_sources = Vec::with_capacity(args.len());
        for (index, arg) in args.iter().enumerate() {
            let constructs_weak = matches!(params.get(index), Some(Type::Weak(_)))
                && !matches!(&arg.ty, Type::Weak(_));
            if constructs_weak {
                // Keep a freshly-computed strong argument alive through
                // the call's full expression. The weak temporary and the
                // strong source are both cleaned up immediately after
                // the call below.
                let strong = self.lower_expr(arg);
                let weak = Operand::Local(self.materialize(
                    Rvalue::Use(strong.clone()),
                    Type::Weak(Box::new(arg.ty.clone())),
                ));
                if let Operand::Local(local) = weak {
                    self.push_instr(Instr::WeakRetain(local));
                    operands.push(Operand::Local(local));
                }
                weak_sources.push(Some(strong));
            } else {
                operands.push(self.lower_expr(arg));
                weak_sources.push(None);
            }
        }
        (operands, weak_sources)
    }

    fn release_call_arg_temporaries(
        &mut self,
        args: &[MonoExpr],
        operands: &[Operand],
        weak_sources: &[Option<Operand>],
    ) {
        for ((arg, operand), weak_source) in args.iter().zip(operands).zip(weak_sources) {
            if let Some(strong) = weak_source {
                if let Operand::Local(local) = operand {
                    self.push_instr(Instr::Release(*local));
                }
                self.release_temporary_value(arg, strong);
            } else {
                self.release_temporary_value(arg, operand);
            }
        }
    }

    /// Drops the initial ownership credit of a computed temporary after
    /// its consumer has taken/retained what it needs. Bare local
    /// references are not temporaries; their lexical scope remains their
    /// owner. Constructors routed through `prepare_new_binding` transfer
    /// their credit instead and deliberately do not call this helper.
    fn release_temporary_value(&mut self, expr: &MonoExpr, operand: &Operand) {
        if is_trivial_local_alias(expr) || !self.sigs.has_managed_content(&expr.ty, self.defs) {
            return;
        }
        if let Operand::Local(local) = operand {
            self.push_instr(Instr::Release(*local));
        }
    }

    /// Like [`Self::lower_exprs`], but for operands that become a *stored*
    /// field inside a freshly constructed container (`Construct`,
    /// `ConstructVariant`, `Tuple`, `Array`) — each one goes through
    /// [`Self::prepare_new_binding`], exactly like a field/index store's
    /// incoming value (that method's own doc comment already lists this
    /// case), since the new container now independently owns whatever
    /// heap-kind value ends up in each slot, same as any other freshly
    /// bound owner. Deliberately not used for `Call`/`CallBuiltin`/
    /// `CallArrayMethod` arguments — `insert_arc`'s `call_operands` rule
    /// already wraps those in its own retain/release around the call
    /// instruction; routing them through here too would double-retain.
    fn lower_stored_exprs(&mut self, exprs: &[MonoExpr]) -> Vec<Operand> {
        exprs.iter().map(|e| self.prepare_new_binding(e)).collect()
    }

    /// Like [`Self::lower_stored_exprs`], but for a `Construct`'s own
    /// fields specifically, since — unlike a `Tuple`/`Array`/
    /// `ConstructVariant`'s elements — a struct's fields have their own,
    /// individually declared types, and a field declared `weak T` needs
    /// [`Self::prepare_weak_binding`] instead of the ordinary heap-retain
    /// logic `prepare_new_binding` applies (which only knows how to
    /// retain a *strong* reference of the source expression's own type,
    /// not construct a fresh weak one). Tuples/enum payloads/arrays
    /// holding a `weak`-typed element directly are a documented gap —
    /// see this module's own docs.
    fn lower_construct_fields(&mut self, ty: &Type, fields: &[MonoExpr]) -> Vec<Operand> {
        let declared = self.field_declared_types(ty);
        fields
            .iter()
            .enumerate()
            .map(|(i, f)| match declared.get(i) {
                Some(Type::Weak(_)) => self.prepare_weak_binding(f),
                _ => self.prepare_new_binding(f),
            })
            .collect()
    }

    fn field_declared_types(&self, ty: &Type) -> Vec<Type> {
        self.sigs.type_fields(ty).unwrap_or_default()
    }

    /// Evaluates `value` (of heap type `T`) for storage into a `weak T`
    /// -typed field — the value's own strong reference is left
    /// completely untouched (spec §13: "`weak T` never affects the
    /// retain count of its referent"); instead a fresh [`Instr::WeakRetain`]
    /// runs on a newly materialized `weak T`-typed copy, bumping the
    /// *weak* count so `runtime/arc` can safely defer freeing the header
    /// until every weak reference to it has also gone (see
    /// `runtime/arc`'s own module docs on why a weak count exists at
    /// all).
    fn prepare_weak_binding(&mut self, value: &MonoExpr) -> Operand {
        let op = self.lower_expr(value);
        let weak_ty = Type::Weak(Box::new(value.ty.clone()));
        let local = self.materialize(Rvalue::Use(op.clone()), weak_ty);
        self.push_instr(Instr::WeakRetain(local));
        // A weak binding never consumes a strong ownership credit.
        // Named values keep their lexical owner; a freshly-computed
        // referent must drop its temporary strong +1 after the weak count
        // has been established.
        self.release_temporary_value(value, &op);
        Operand::Local(local)
    }

    fn lower_expr(&mut self, expr: &MonoExpr) -> Operand {
        match &expr.kind {
            MonoExprKind::Literal(l) => {
                let operand = Operand::Literal(l.clone(), expr.ty.clone());
                if self.sigs.has_managed_content(&expr.ty, self.defs) {
                    Operand::Local(self.materialize(Rvalue::Use(operand), expr.ty.clone()))
                } else {
                    operand
                }
            }
            MonoExprKind::Local(id) => Operand::Local(self.local_for(*id)),
            MonoExprKind::FnRef(id) => Operand::Local(self.materialize(
                Rvalue::Closure {
                    function: *id,
                    captures: Vec::new(),
                },
                expr.ty.clone(),
            )),
            MonoExprKind::Unit => Operand::Unit,
            MonoExprKind::Tuple(items) => {
                let ops = self.lower_stored_exprs(items);
                Operand::Local(self.materialize(Rvalue::Tuple(ops), expr.ty.clone()))
            }
            MonoExprKind::Array(items) => {
                // `runtime/array_push` retains each stored element via
                // the element shim, so array literals borrow their input
                // operands here and drop only freshly-computed input
                // temporaries after construction.
                let ops = self.lower_exprs(items);
                let result =
                    Operand::Local(self.materialize(Rvalue::Array(ops.clone()), expr.ty.clone()));
                for (item, operand) in items.iter().zip(&ops) {
                    self.release_temporary_value(item, operand);
                }
                result
            }
            MonoExprKind::Concat(items) => {
                let ops = self.lower_exprs(items);
                let result =
                    Operand::Local(self.materialize(Rvalue::Concat(ops.clone()), expr.ty.clone()));
                for (item, operand) in items.iter().zip(&ops) {
                    self.release_temporary_value(item, operand);
                }
                result
            }
            MonoExprKind::ToString(inner) => {
                let op = self.lower_expr(inner);
                Operand::Local(self.materialize(Rvalue::ToString(op), expr.ty.clone()))
            }
            MonoExprKind::Unary { op, expr: inner } => {
                let inner_op = self.lower_expr(inner);
                Operand::Local(self.materialize(Rvalue::Unary(*op, inner_op), expr.ty.clone()))
            }
            MonoExprKind::Binary { op, lhs, rhs } => {
                let l = self.lower_expr(lhs);
                let r = self.lower_expr(rhs);
                Operand::Local(self.materialize(Rvalue::Binary(*op, l, r), expr.ty.clone()))
            }
            MonoExprKind::Assign { target, value } => {
                self.lower_assign(target, value);
                Operand::Unit
            }
            MonoExprKind::Call { callee, args } => {
                let callee_op = self.lower_expr(callee);
                let params = match &callee.ty {
                    Type::Function(params, _) => params.clone(),
                    _ => Vec::new(),
                };
                let (arg_ops, weak_sources) = self.lower_call_args(args, &params);
                let target = match callee_op {
                    Operand::Fn(id) => CallTarget::Fn(id),
                    ref other => CallTarget::Dynamic(other.clone()),
                };
                let result = Operand::Local(self.materialize(
                    Rvalue::Call {
                        target,
                        args: arg_ops.clone(),
                    },
                    expr.ty.clone(),
                ));
                self.release_temporary_value(callee, &callee_op);
                self.release_call_arg_temporaries(args, &arg_ops, &weak_sources);
                result
            }
            MonoExprKind::CallStatic { fn_id, args } => {
                let target = self.module.get(*fn_id);
                let mut params = Vec::with_capacity(args.len());
                if target.self_param.is_some() {
                    params.push(target.self_ty.clone().unwrap_or(Type::Error));
                }
                params.extend(target.params.iter().map(|param| param.ty.clone()));
                let (arg_ops, weak_sources) = self.lower_call_args(args, &params);
                let result = Operand::Local(self.materialize(
                    Rvalue::Call {
                        target: CallTarget::Fn(*fn_id),
                        args: arg_ops.clone(),
                    },
                    expr.ty.clone(),
                ));
                self.release_call_arg_temporaries(args, &arg_ops, &weak_sources);
                result
            }
            MonoExprKind::CallBuiltin { name, args } => {
                let arg_ops = self.lower_exprs(args);
                let result = Operand::Local(self.materialize(
                    Rvalue::CallBuiltin {
                        name: name.clone(),
                        args: arg_ops.clone(),
                    },
                    expr.ty.clone(),
                ));
                for (arg, operand) in args.iter().zip(&arg_ops) {
                    self.release_temporary_value(arg, operand);
                }
                result
            }
            MonoExprKind::CallArrayMethod {
                receiver,
                method,
                args,
            } => {
                let recv_op = self.lower_expr(receiver);
                let arg_ops = self.lower_exprs(args);
                let result = Operand::Local(self.materialize(
                    Rvalue::CallArrayMethod {
                        receiver: recv_op.clone(),
                        method: method.clone(),
                        args: arg_ops.clone(),
                    },
                    expr.ty.clone(),
                ));
                self.release_temporary_value(receiver, &recv_op);
                for (arg, operand) in args.iter().zip(&arg_ops) {
                    self.release_temporary_value(arg, operand);
                }
                result
            }
            MonoExprKind::Field { base, index } => {
                let base_op = self.lower_expr(base);
                let result = Operand::Local(self.materialize(
                    Rvalue::Field {
                        base: base_op.clone(),
                        index: *index,
                    },
                    expr.ty.clone(),
                ));
                self.release_temporary_value(base, &base_op);
                result
            }
            MonoExprKind::Index { base, index } => {
                let base_op = self.lower_expr(base);
                let index_op = self.lower_expr(index);
                let result = Operand::Local(self.materialize(
                    Rvalue::Index {
                        base: base_op.clone(),
                        index: index_op.clone(),
                    },
                    expr.ty.clone(),
                ));
                self.release_temporary_value(base, &base_op);
                self.release_temporary_value(index, &index_op);
                result
            }
            MonoExprKind::Construct { ty, fields } => {
                let ops = self.lower_construct_fields(&expr.ty, fields);
                Operand::Local(self.materialize(
                    Rvalue::Construct {
                        ty: *ty,
                        fields: ops,
                    },
                    expr.ty.clone(),
                ))
            }
            MonoExprKind::ConstructVariant {
                enum_id,
                variant,
                payload,
            } => {
                let ops = self.lower_stored_exprs(payload);
                Operand::Local(self.materialize(
                    Rvalue::ConstructVariant {
                        enum_id: *enum_id,
                        variant: *variant,
                        payload: ops,
                    },
                    expr.ty.clone(),
                ))
            }
            MonoExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => self.lower_if(expr, cond, then_branch, else_branch.as_deref()),
            MonoExprKind::Match { scrutinee, arms } => self.lower_match(expr, scrutinee, arms),
            MonoExprKind::Block(stmts, tail) => self.lower_block(stmts, tail.as_deref()),
            MonoExprKind::While { cond, body } => self.lower_while(cond, body),
            MonoExprKind::Loop { body } => self.lower_loop(body),
            MonoExprKind::Break(value) => {
                if let Some(v) = value {
                    let value = self.lower_expr(v);
                    self.release_temporary_value(v, &value);
                }
                let ctx_depth = self
                    .loop_stack
                    .last()
                    .expect("`break` outside a loop — typecheck rejects this")
                    .scope_depth;
                let break_block = self.loop_stack.last().unwrap().break_block;
                self.release_scopes(ctx_depth, None);
                self.terminate_current(Terminator::Goto(break_block));
                Operand::Unit
            }
            MonoExprKind::Continue => {
                let ctx_depth = self
                    .loop_stack
                    .last()
                    .expect("`continue` outside a loop — typecheck rejects this")
                    .scope_depth;
                let continue_block = self.loop_stack.last().unwrap().continue_block;
                self.release_scopes(ctx_depth, None);
                self.terminate_current(Terminator::Goto(continue_block));
                Operand::Unit
            }
            MonoExprKind::Return(value) => {
                let op = match value {
                    Some(v) => self.lower_escaping_value(v),
                    None => Operand::Unit,
                };
                self.release_scopes(0, escaping_local(&op));
                self.terminate_current(Terminator::Return(op));
                Operand::Unit
            }
            MonoExprKind::Closure { function, captures } => {
                let captures = self.lower_stored_exprs(captures);
                Operand::Local(self.materialize(
                    Rvalue::Closure {
                        function: *function,
                        captures,
                    },
                    expr.ty.clone(),
                ))
            }
        }
    }

    /// Lowers `target = value` — whether `target` is a plain, already-
    /// bound `mut` variable or a field/index store (`self.name = x;`).
    /// Neither is one of `arc-model.md`'s literal worked examples (which
    /// only covers fresh `let` bindings), but the same §3.1 reasoning
    /// extends symmetrically to mutation: whatever `target` held loses an
    /// owner here and needs releasing, while the incoming value gains one
    /// (via [`Self::prepare_new_binding`]) — see this crate's module docs
    /// for why this counts as build_mir's job, not insert_arc's.
    fn lower_assign(&mut self, target: &MonoExpr, value: &MonoExpr) {
        let place = self.lower_place(target);
        if matches!(&target.ty, Type::Weak(_)) {
            self.lower_weak_assign(place, value);
            return;
        }
        let has_projection = !place.projection.is_empty();
        let value_ty = value.ty.clone();
        let old = if self.sigs.has_managed_content(&value_ty, self.defs) {
            Some(if place.projection.is_empty() {
                // Snapshot the old bits before overwriting the slot.
                // This is an ownership move, not an aliasing copy, so it
                // deliberately receives no retain; the release below
                // consumes the slot's previous credit.
                Operand::Local(
                    self.materialize(Rvalue::Use(Operand::Local(place.local)), value_ty.clone()),
                )
            } else {
                self.read_place(&place, value_ty.clone())
            })
        } else {
            None
        };
        let val_op = self.prepare_new_binding(value);
        self.push_instr(Instr::Assign(place, Rvalue::Use(val_op)));
        if let Some(Operand::Local(old_local)) = old {
            self.push_instr(Instr::Release(old_local));
            if has_projection {
                // For a plain `x = value;` reassignment, `read_place`
                // just names `x`'s own existing storage directly (no new
                // instruction, no incidental retain), so the one release
                // above already correctly drops its one original claim.
                // For a field/index store, `read_place` instead had to
                // *materialize* the old value via its own fresh Field/
                // Index rvalue — which `insert_arc` retains
                // unconditionally, exactly like any other such read (it
                // can't tell this one apart from a normal one). This
                // second release cancels that incidental retain, so the
                // first one above is the only one that nets against the
                // place's actual original reference — see
                // `Self::read_place`.
                self.push_instr(Instr::Release(old_local));
            }
        }
    }

    /// [`Self::lower_assign`]'s branch for a `target` declared `weak T` —
    /// a distinct, simpler path from the general heap case: the old value
    /// (if any — a field/index projection's own previous pointer) is
    /// released via [`Instr::WeakRelease`], never [`Instr::Release`], and
    /// the incoming value is bound via [`Self::prepare_weak_binding`]
    /// rather than `prepare_new_binding`'s ordinary heap-retain logic
    /// (which only knows how to retain a *strong* reference of the
    /// source's own type, not construct a fresh weak one). No incidental
    /// -retain-cancelling second release is needed here the way the heap
    /// case needs one: `insert_arc`'s bind-time-retain rule only ever
    /// fires for a *heap-kind* destination, so materializing the old
    /// value through a fresh `Field`/`Index` read never gets one to begin
    /// with.
    fn lower_weak_assign(&mut self, place: Place, value: &MonoExpr) {
        let has_projection = !place.projection.is_empty();
        let weak_ty = Type::Weak(Box::new(value.ty.clone()));
        let old_op = if place.projection.is_empty() {
            Operand::Local(self.materialize(Rvalue::Use(Operand::Local(place.local)), weak_ty))
        } else {
            self.read_place(&place, weak_ty)
        };
        let val_op = self.prepare_weak_binding(value);
        self.push_instr(Instr::Assign(place, Rvalue::Use(val_op)));
        if let Operand::Local(old_local) = old_op {
            self.push_instr(Instr::WeakRelease(old_local));
            if has_projection {
                // The projected read received an incidental weak retain
                // from the generic aliasing-read rule; cancel that in
                // addition to dropping the field's original weak credit.
                self.push_instr(Instr::WeakRelease(old_local));
            }
        }
    }

    fn lower_if(
        &mut self,
        expr: &MonoExpr,
        cond: &MonoExpr,
        then_branch: &MonoExpr,
        else_branch: Option<&MonoExpr>,
    ) -> Operand {
        let cond_op = self.lower_expr(cond);
        let then_block = self.new_block();
        let else_block = self.new_block();
        let merge_block = self.new_block();
        self.terminate_current(Terminator::Branch {
            cond: cond_op,
            then_block,
            else_block,
        });

        let result = self.declare_local(expr.ty.clone(), false);

        self.current = then_block;
        let then_op = self.prepare_new_binding(then_branch);
        self.push_instr(Instr::Assign(Place::local(result), Rvalue::Use(then_op)));
        self.terminate_current_to(merge_block);

        self.current = else_block;
        let else_op = match else_branch {
            Some(e) => self.prepare_new_binding(e),
            None => Operand::Unit,
        };
        self.push_instr(Instr::Assign(Place::local(result), Rvalue::Use(else_op)));
        self.terminate_current_to(merge_block);

        self.current = merge_block;
        Operand::Local(result)
    }

    /// Like [`Self::terminate_current`], but joins into an
    /// *already-created* block (an `if`/`match` merge point) instead of a
    /// fresh dead one.
    fn terminate_current_to(&mut self, target: BlockId) {
        let cur = self.current;
        self.blocks[cur.0 as usize].terminator = Some(Terminator::Goto(target));
        self.current = self.new_block();
        // This fresh block is unreachable (the real successor is
        // `target`) — left for `build`'s `Unreachable` default, same as
        // `terminate_current`.
    }

    fn lower_block(
        &mut self,
        stmts: &[nether_monomorphization::MonoStmt],
        tail: Option<&MonoExpr>,
    ) -> Operand {
        self.scopes.push(Vec::new());
        for stmt in stmts {
            match &stmt.kind {
                MonoStmtKind::Let { local, ty, value } => {
                    let value_ty = ty.clone();
                    let val_op = if matches!(ty, Type::Weak(_)) {
                        self.prepare_weak_binding(value)
                    } else {
                        self.prepare_new_binding(value)
                    };
                    let mir_local = self.as_local(val_op, value_ty.clone());
                    self.local_map.insert(*local, mir_local);
                    if self.sigs.has_managed_content(&value_ty, self.defs) {
                        self.scopes.last_mut().unwrap().push(mir_local);
                    }
                }
                MonoStmtKind::Expr(e) => {
                    let operand = self.lower_expr(e);
                    self.release_temporary_value(e, &operand);
                }
            }
        }
        let result = match tail {
            Some(t) => self.lower_escaping_value(t),
            None => Operand::Unit,
        };
        let depth = self.scopes.len() - 1;
        self.release_scopes(depth, escaping_local(&result));
        self.scopes.pop();
        result
    }

    fn lower_while(&mut self, cond: &MonoExpr, body: &MonoExpr) -> Operand {
        let header = self.new_block();
        self.terminate_current_to(header);
        self.current = header;
        let cond_op = self.lower_expr(cond);
        let body_block = self.new_block();
        let exit_block = self.new_block();
        self.terminate_current(Terminator::Branch {
            cond: cond_op,
            then_block: body_block,
            else_block: exit_block,
        });

        self.loop_stack.push(LoopCtx {
            break_block: exit_block,
            continue_block: header,
            scope_depth: self.scopes.len(),
        });
        self.current = body_block;
        self.lower_expr(body);
        self.terminate_current_to(header);
        self.loop_stack.pop();

        self.current = exit_block;
        Operand::Unit
    }

    fn lower_loop(&mut self, body: &MonoExpr) -> Operand {
        let header = self.new_block();
        self.terminate_current_to(header);
        let exit_block = self.new_block();

        self.loop_stack.push(LoopCtx {
            break_block: exit_block,
            continue_block: header,
            scope_depth: self.scopes.len(),
        });
        self.current = header;
        self.lower_expr(body);
        self.terminate_current_to(header);
        self.loop_stack.pop();

        self.current = exit_block;
        // `loop`'s own type is always unit regardless of `break value` —
        // `nether_typecheck`'s own documented simplification, carried
        // forward unchanged (see this crate's module docs).
        Operand::Unit
    }

    fn lower_match(
        &mut self,
        expr: &MonoExpr,
        scrutinee: &MonoExpr,
        arms: &[MonoMatchArm],
    ) -> Operand {
        let scrutinee_ty = scrutinee.ty.clone();
        // Goes through `prepare_new_binding`, not the cheaper
        // `lower_expr_to_local`, for exactly the same reason a `let`
        // does: a bare-parameter/bare-local scrutinee needs its own
        // fresh credit before every arm binding (including a top-level
        // `HirPattern::Binding` catch-all) can safely reuse it directly.
        let scrutinee_op = self.prepare_new_binding(scrutinee);
        let scrutinee_local = self.as_local(scrutinee_op, scrutinee_ty.clone());
        let match_scope_depth = self.scopes.len();
        let mut match_scope = Vec::new();
        if self.sigs.has_managed_content(&scrutinee_ty, self.defs) {
            match_scope.push(scrutinee_local);
        }
        self.scopes.push(match_scope);
        let result = self.declare_local(expr.ty.clone(), false);
        let merge_block = self.new_block();

        for arm in arms {
            let body_block = self.new_block();
            let next_block = self.new_block();
            self.lower_pattern_branch(
                Operand::Local(scrutinee_local),
                &scrutinee_ty,
                &arm.pattern,
                body_block,
                next_block,
            );

            self.current = body_block;
            // Each arm's own pattern bindings form their own scope,
            // released once the arm's body has produced its value —
            // exactly the same shape as a `let` inside a `Block` (a
            // heap-kind binding here, e.g. `Boxed(d) => ...`'s `d`, would
            // otherwise never be released at all: nothing else tracks
            // it, and `insert_arc` already gave it a bind-time retain via
            // the `VariantField`/`Field` rvalue
            // `lower_pattern_bindings` read it from).
            self.scopes.push(Vec::new());
            let mut bindings = Vec::new();
            self.lower_pattern_bindings(
                Operand::Local(scrutinee_local),
                &scrutinee_ty,
                &arm.pattern,
                &mut bindings,
            );
            for (id, op, ty) in bindings {
                let local = self.as_local(op, ty.clone());
                self.local_map.insert(id, local);
                if self.sigs.has_managed_content(&ty, self.defs) {
                    self.scopes.last_mut().unwrap().push(local);
                }
            }
            // `arm.body` may be a bare expression (`Circle(x) => x`, no
            // braces — language-spec match arms don't require a block),
            // unlike `if`/`while`/`loop` bodies, which are always
            // `Block`s and so always self-credit via their own
            // `lower_block`/`escape`; `prepare_new_binding` is what
            // catches a bare `Circle(_) => self`-style arm here.
            let body_op = self.prepare_new_binding(&arm.body);
            let body_escaping = escaping_local(&body_op);
            self.push_instr(Instr::Assign(Place::local(result), Rvalue::Use(body_op)));
            self.release_scopes(match_scope_depth, body_escaping);
            self.scopes.pop();
            self.terminate_current_to(merge_block);

            self.current = next_block;
        }
        // Every arm chain is exhaustive (`typecheck`'s own invariant) —
        // this final fallthrough block is provably never reached.
        self.terminate_current(Terminator::Unreachable);

        self.scopes.pop();
        self.current = merge_block;
        Operand::Local(result)
    }

    /// Emits the control-flow test for one pattern. Variant payloads are
    /// deliberately visited only from the block reached after their tag
    /// has matched: unused enum payload slots are not initialized, and
    /// eagerly reading a managed payload from (for example) `Option.None`
    /// would make ARC retain an arbitrary pointer.
    fn lower_pattern_branch(
        &mut self,
        scrutinee: Operand,
        scrutinee_ty: &Type,
        pattern: &HirPattern,
        success: BlockId,
        failure: BlockId,
    ) {
        let bool_ty = Type::Primitive(PrimitiveKind::Bool);
        match pattern {
            HirPattern::Wildcard | HirPattern::Binding(_) => {
                self.terminate_current_to(success);
            }
            HirPattern::Literal(lit) => {
                let cond = Operand::Local(self.materialize(
                    Rvalue::Binary(
                        nether_ast::BinaryOp::Eq,
                        scrutinee,
                        Operand::Literal(lit.clone(), scrutinee_ty.clone()),
                    ),
                    bool_ty,
                ));
                self.terminate_current(Terminator::Branch {
                    cond,
                    then_block: success,
                    else_block: failure,
                });
            }
            HirPattern::Tuple(subs) => {
                let elem_tys = match scrutinee_ty {
                    Type::Tuple(tys) => tys.clone(),
                    _ => vec![Type::Error; subs.len()],
                };
                for (i, sub) in subs.iter().enumerate() {
                    let elem_ty = elem_tys.get(i).cloned().unwrap_or(Type::Error);
                    let next = if i + 1 == subs.len() {
                        success
                    } else {
                        self.new_block()
                    };
                    if matches!(sub, HirPattern::Wildcard | HirPattern::Binding(_)) {
                        self.terminate_current_to(next);
                    } else {
                        let field_val = Operand::Local(self.materialize(
                            Rvalue::Field {
                                base: scrutinee.clone(),
                                index: i as u32,
                            },
                            elem_ty.clone(),
                        ));
                        self.lower_pattern_branch(field_val, &elem_ty, sub, next, failure);
                    }
                    if next != success {
                        self.current = next;
                    }
                }
                if subs.is_empty() {
                    self.terminate_current_to(success);
                }
            }
            HirPattern::Variant {
                enum_id: _,
                variant,
                payload,
            } => {
                let usize_ty = Type::Primitive(PrimitiveKind::Usize);
                let tag = Operand::Local(
                    self.materialize(Rvalue::Discriminant(scrutinee.clone()), usize_ty.clone()),
                );
                let tag_matches = Operand::Local(self.materialize(
                    Rvalue::Binary(
                        nether_ast::BinaryOp::Eq,
                        tag,
                        Operand::Literal(nether_ast::Literal::Int(u128::from(*variant)), usize_ty),
                    ),
                    bool_ty.clone(),
                ));
                let payload_block = if payload.is_empty() {
                    success
                } else {
                    self.new_block()
                };
                self.terminate_current(Terminator::Branch {
                    cond: tag_matches,
                    then_block: payload_block,
                    else_block: failure,
                });
                if payload.is_empty() {
                    return;
                }
                self.current = payload_block;
                let payload_tys = self
                    .sigs
                    .enum_payload(scrutinee_ty, *variant)
                    .unwrap_or_default();
                for (i, sub) in payload.iter().enumerate() {
                    let elem_ty = payload_tys.get(i).cloned().unwrap_or(Type::Error);
                    let next = if i + 1 == payload.len() {
                        success
                    } else {
                        self.new_block()
                    };
                    if matches!(sub, HirPattern::Wildcard | HirPattern::Binding(_)) {
                        self.terminate_current_to(next);
                    } else {
                        let field_val = Operand::Local(self.materialize(
                            Rvalue::VariantField {
                                base: scrutinee.clone(),
                                variant: *variant,
                                index: i as u32,
                            },
                            elem_ty.clone(),
                        ));
                        self.lower_pattern_branch(field_val, &elem_ty, sub, next, failure);
                    }
                    if next != success {
                        self.current = next;
                    }
                }
            }
        }
    }

    /// Collects bindings after [`Self::lower_pattern_branch`] has proved
    /// the complete pattern. Every variant payload read here is therefore
    /// active and safe to retain.
    fn lower_pattern_bindings(
        &mut self,
        scrutinee: Operand,
        scrutinee_ty: &Type,
        pattern: &HirPattern,
        bindings: &mut Vec<(HirLocalId, Operand, Type)>,
    ) {
        match pattern {
            HirPattern::Wildcard | HirPattern::Literal(_) => {}
            HirPattern::Binding(id) => {
                bindings.push((*id, scrutinee, scrutinee_ty.clone()));
            }
            HirPattern::Tuple(subs) => {
                let elem_tys = match scrutinee_ty {
                    Type::Tuple(tys) => tys.clone(),
                    _ => vec![Type::Error; subs.len()],
                };
                for (i, sub) in subs.iter().enumerate() {
                    if !pattern_has_bindings(sub) {
                        continue;
                    }
                    let elem_ty = elem_tys.get(i).cloned().unwrap_or(Type::Error);
                    let field = Operand::Local(self.materialize(
                        Rvalue::Field {
                            base: scrutinee.clone(),
                            index: i as u32,
                        },
                        elem_ty.clone(),
                    ));
                    self.lower_pattern_bindings(field, &elem_ty, sub, bindings);
                }
            }
            HirPattern::Variant {
                variant, payload, ..
            } => {
                let payload_tys = self
                    .sigs
                    .enum_payload(scrutinee_ty, *variant)
                    .unwrap_or_default();
                for (i, sub) in payload.iter().enumerate() {
                    if !pattern_has_bindings(sub) {
                        continue;
                    }
                    let elem_ty = payload_tys.get(i).cloned().unwrap_or(Type::Error);
                    let field = Operand::Local(self.materialize(
                        Rvalue::VariantField {
                            base: scrutinee.clone(),
                            variant: *variant,
                            index: i as u32,
                        },
                        elem_ty.clone(),
                    ));
                    self.lower_pattern_bindings(field, &elem_ty, sub, bindings);
                }
            }
        }
    }
}

fn escaping_local(op: &Operand) -> Option<Local> {
    match op {
        Operand::Local(l) => Some(*l),
        _ => None,
    }
}

fn pattern_has_bindings(pattern: &HirPattern) -> bool {
    match pattern {
        HirPattern::Binding(_) => true,
        HirPattern::Tuple(items) => items.iter().any(pattern_has_bindings),
        HirPattern::Variant { payload, .. } => payload.iter().any(pattern_has_bindings),
        HirPattern::Wildcard | HirPattern::Literal(_) => false,
    }
}

/// Whether `expr` is, syntactically, nothing more than a bare name for an
/// *already-bound* local, as opposed to an expression that computes or
/// reads a value of its own. Deliberately does *not* look through a
/// `Block`'s tail even when that tail is itself a bare name: a `Block`
/// always credits its own escaping value correctly on its own
/// (`FnBuilder::lower_block` calls [`FnBuilder::lower_escaping_value`] internally,
/// which already applies this exact same reasoning) — treating the whole
/// `Block` as "still a trivial alias" too would double the retain that
/// already happened inside it. See [`FnBuilder::prepare_new_binding`] for
/// why this distinction is what decides whether binding `expr`'s value to
/// a second name needs a fresh `Retain`.
fn is_trivial_local_alias(expr: &MonoExpr) -> bool {
    matches!(expr.kind, MonoExprKind::Local(_))
}
