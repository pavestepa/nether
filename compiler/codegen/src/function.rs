use std::collections::HashMap;

use nether_ast::{BinaryOp, Literal, Symbol, UnaryOp};
use nether_llvm::{Block, FloatPredicate, Func, IntPredicate, ModuleCx, Value};
use nether_mir::{
    CallTarget, Instr, Local, MirFunction, Operand, Place, Projection, Rvalue, Terminator,
};
use nether_monomorphization::MonoFnId;
use nether_typecheck::{alloc_kind, AllocKind, CaptureMode, PrimitiveKind, Type};

use crate::frame::FrameLayout;
use crate::layout::Layout;
use crate::runtime::Runtime;
use crate::shims::Shims;

mod aggregate;
mod async_fn;
mod helpers;
mod operation;
mod runtime;
mod terminator;

use helpers::block_num;

/// Whether a value of `ty` is addressed directly through its own
/// `alloca`/heap pointer (an aggregate GEPed into in place) rather than
/// loaded as a scalar SSA value — see `crate` module docs and
/// `nether_llvm`'s own "never `insertvalue`/`extractvalue`" design note.
pub(crate) fn is_aggregate(ty: &Type, defs: &nether_resolver::Definitions) -> bool {
    match ty {
        Type::Tuple(_) | Type::Enum(_, _) | Type::FixedArray(_, _) => true,
        Type::Struct(_, _) | Type::TupleStruct(_, _) => alloc_kind(ty, defs) == AllocKind::Stack,
        _ => false,
    }
}

pub(crate) struct FunctionEnv<'m, 'ctx> {
    pub(crate) m: &'m ModuleCx<'ctx>,
    pub(crate) layout: &'m Layout<'m, 'ctx>,
    pub(crate) runtime: &'m Runtime<'ctx>,
    pub(crate) shims: &'m Shims<'ctx>,
    pub(crate) funcs: &'m HashMap<MonoFnId, Func<'ctx>>,
    /// The second LLVM function every `is_async` `MirFunction` gets — see
    /// `async_fn`'s module docs. Empty entries for every non-async
    /// function.
    pub(crate) poll_funcs: &'m HashMap<MonoFnId, Func<'ctx>>,
    pub(crate) mir_functions: &'m HashMap<MonoFnId, &'m MirFunction>,
}

pub(crate) fn build_function<'m, 'ctx>(
    env: &'m FunctionEnv<'m, 'ctx>,
    mir_fn: &'m MirFunction,
    llvm_fn: Func<'ctx>,
) {
    if mir_fn.is_async {
        async_fn::build_async_function(env, mir_fn, llvm_fn);
        return;
    }
    let mut fc = FnCodegen {
        m: env.m,
        layout: env.layout,
        runtime: env.runtime,
        shims: env.shims,
        funcs: env.funcs,
        mir_functions: env.mir_functions,
        mir_fn,
        llvm_fn,
        locals: Vec::new(),
        blocks: HashMap::new(),
        frame: None,
        frame_ptr: None,
        unit_slot: None,
        entry_block: None,
    };
    fc.build();
}

struct FnCodegen<'m, 'ctx> {
    m: &'m ModuleCx<'ctx>,
    layout: &'m Layout<'m, 'ctx>,
    runtime: &'m Runtime<'ctx>,
    shims: &'m Shims<'ctx>,
    funcs: &'m HashMap<MonoFnId, Func<'ctx>>,
    mir_functions: &'m HashMap<MonoFnId, &'m MirFunction>,
    mir_fn: &'m MirFunction,
    llvm_fn: Func<'ctx>,
    /// One `alloca` per MIR local, indexed by [`Local::index`] — see this
    /// module's own docs on aggregate addressing. When `frame` is
    /// `Some(_)` (building an `is_async` function's *poll* function —
    /// `async_fn::build_async_function`), each of these is instead a GEP
    /// into the frame at that local's own field, or — for an alias-kind
    /// local (`frame::FrameLayout::alias_locals`) — the alias pointer
    /// loaded out of that field. Every other instruction-codegen helper
    /// only ever addresses a local through this vector, agnostic to which
    /// case it came from.
    locals: Vec<Value<'ctx>>,
    blocks: HashMap<nether_mir::BlockId, Block<'ctx>>,
    /// `Some(_)` only while building an `is_async` function's *poll*
    /// function (`async_fn::build_async_function`) — `None` for every
    /// ordinary function and for an async function's own *start*
    /// function (which never goes through `FnCodegen` at all, having no
    /// MIR body of its own to translate).
    frame: Option<FrameLayout<'ctx>>,
    /// The poll function's own single parameter (the frame pointer) —
    /// `Some(_)` exactly when `frame` is.
    frame_ptr: Option<Value<'ctx>>,
    /// One shared `alloca {}` for every `Operand::Unit` this function ever
    /// materializes (`Self::gen_unit`), created once in the entry block by
    /// `Self::build` — `None` only until then. A real, previously unfixed
    /// memory-safety bug: `gen_unit` used to call `self.m.alloca(...)`
    /// directly at whatever block the builder happened to be positioned
    /// in, which is *not* the entry block once codegen has moved past the
    /// prologue. LLVM only treats an `alloca` in the entry block as a
    /// fixed-offset stack slot; anywhere else, it must be lowered as a
    /// *dynamic* (runtime) stack adjustment — for a zero-sized `{}`, that
    /// lowering still emits a spill store to the (unmoved) current stack
    /// pointer, which on this project's own AArch64 target landed exactly
    /// on the bottom word of the function's fixed frame, silently
    /// clobbering whatever local happened to live there (e.g. a cached
    /// `&mut self` receiver address for a second method call). One shared,
    /// entry-block slot sidesteps dynamic allocation entirely — sound
    /// because `{}` carries no actual data for two "instances" to ever
    /// conflict over.
    unit_slot: Option<Value<'ctx>>,
    /// This function's entry block, set once in `Self::build` — `None`
    /// only until then. Every other scratch slot that (like `unit_slot`)
    /// turns out to be needed only partway through building a later
    /// block, but must still be a fixed-offset stack slot, goes through
    /// [`ModuleCx::entry_alloca`] with this handle (see `Self::gen_call`).
    entry_block: Option<Block<'ctx>>,
}

impl<'ctx> FnCodegen<'_, 'ctx> {
    fn defs(&self) -> &nether_resolver::Definitions {
        self.layout.defs
    }

    /// An optionally-present shim [`Func`] as the `ptr` value a runtime
    /// call's nullable callback parameter expects — `nether_rt_arc_alloc`'s
    /// `drop` and `nether_rt_array_new`'s `elem_retain`/`elem_drop`.
    fn func_ptr_or_null(&self, f: Option<Func<'ctx>>) -> Value<'ctx> {
        match f {
            Some(f) => f.as_global_value().as_pointer_value().into(),
            None => self.m.const_null_ptr(),
        }
    }

    fn build(&mut self) {
        let entry = self.m.append_block(self.llvm_fn, "entry");
        self.m.position_at_end(entry);
        self.entry_block = Some(entry);
        let unit_ty = self.m.struct_type(&[]);
        self.unit_slot = Some(self.m.alloca(unit_ty.into(), "unit"));

        if self.frame.is_some() {
            self.build_frame_prologue();
        } else {
            self.build_ordinary_prologue();
        }

        for block in &self.mir_fn.blocks {
            let name = format!("bb{}", block_num(block.id));
            self.blocks
                .insert(block.id, self.m.append_block(self.llvm_fn, &name));
        }

        if let Some(frame) = self.frame.clone() {
            // Replaces the ordinary unconditional `br` into `mir_fn.entry`
            // below: an `is_async` function's poll function can be
            // re-entered at any of several points, not just its own
            // entry — see `frame::FrameLayout`'s own docs.
            self.build_state_dispatch(&frame, entry);
        } else {
            self.m.br(self.blocks[&self.mir_fn.entry]);
        }

        for block in &self.mir_fn.blocks {
            self.m.position_at_end(self.blocks[&block.id]);
            for instr in &block.instrs {
                self.gen_instr(instr);
            }
            self.gen_terminator(block.id, &block.terminator);
        }
    }

    fn build_ordinary_prologue(&mut self) {
        for (i, decl) in self.mir_fn.locals.iter().enumerate() {
            let ty = self.layout.llvm_type(&decl.ty);
            let slot = self.m.alloca(ty, &format!("local{i}"));
            self.locals.push(slot);
        }
        if self.mir_fn.is_closure {
            let env = self.m.param(self.llvm_fn, 0);
            // The environment's own field type differs from `local`'s
            // declared type for a `CaptureMode::ByRef` capture: the field
            // holds the *address* of the outer local's storage (a plain
            // pointer, matching `nether_monomorphization`'s own
            // `Type::MutRef` wrapping at the closure-creation call site),
            // while `local`'s declared type stays the plain element type
            // for the body's benefit (`nether_mir::MonoCapture::ty`'s own
            // docs) — so this must be derived the same way the creation
            // side derives it, not read straight off `local_ty`.
            let capture_tys: Vec<Type> = self
                .mir_fn
                .closure_captures
                .iter()
                .map(|&(local, mode)| match mode {
                    CaptureMode::ByValue => self.local_ty(local),
                    CaptureMode::ByRef => Type::MutRef(Box::new(self.local_ty(local))),
                })
                .collect();
            let mut fields = vec![self.m.ptr_type()];
            fields.extend(capture_tys.iter().map(|ty| self.layout.llvm_type(ty)));
            let env_ty = self.m.struct_type(&fields);
            for (index, (&(local, mode), ty)) in self
                .mir_fn
                .closure_captures
                .iter()
                .zip(capture_tys.iter())
                .enumerate()
            {
                let field = self.m.struct_gep(env_ty, env, index as u32 + 1, "capture");
                match mode {
                    CaptureMode::ByValue => {
                        // A captured binding's storage is the environment
                        // field itself. Rebinding/mutating it therefore
                        // persists across calls to this closure while
                        // remaining independent from the outer variable
                        // (capture-by-value semantics).
                        let _ = ty;
                        self.locals[local.index()] = field;
                    }
                    CaptureMode::ByRef => {
                        // The field holds the outer local's own address —
                        // load it once and use it directly as this
                        // local's storage, so every read/write inside the
                        // body transparently reads/writes the *same*
                        // memory as the outer binding (`mut (...) => {}`,
                        // Stage 7).
                        self.locals[local.index()] =
                            self.m.load(self.m.ptr_type(), field, "captured_ref");
                    }
                }
            }
        }
        let param_offset = usize::from(self.mir_fn.is_closure);
        for (i, &local) in self.mir_fn.params.iter().enumerate() {
            let param_val = self.m.param(self.llvm_fn, (i + param_offset) as u32);
            let ty = self.local_ty(local);
            if (self.mir_fn.local_decl(local).mutable
                && !matches!(ty, Type::Ref(_) | Type::MutRef(_)))
                || is_aggregate(&ty, self.defs())
            {
                // Aggregates and `mut` parameters are passed by pointer
                // (see `crate::declare`'s matching signature choice).
                // In particular, a mutable scalar aliases the caller's
                // slot instead of receiving the previous by-value copy.
                self.locals[local.index()] = param_val;
            } else {
                self.m.store(self.locals[local.index()], param_val);
            }
        }
    }

    /// The poll-function counterpart of [`Self::build_ordinary_prologue`]
    /// — every local's storage is a field inside the frame the *start*
    /// function already allocated and (for parameters) populated
    /// (`async_fn::build_async_start`), addressed via `struct_gep` in
    /// place of an `alloca`. An alias-kind local's field instead holds a
    /// caller-owned pointer that must be loaded once to reconstruct the
    /// exact value `build_ordinary_prologue`'s matching branch would have
    /// assigned directly — see [`FrameLayout::alias_locals`]'s own docs.
    fn build_frame_prologue(&mut self) {
        let frame = self
            .frame
            .clone()
            .expect("build_frame_prologue requires a frame");
        let frame_ptr = self.m.param(self.llvm_fn, 0);
        self.frame_ptr = Some(frame_ptr);
        self.locals = (0..self.mir_fn.locals.len())
            .map(|i| {
                let field_addr =
                    self.m
                        .struct_gep(frame.ty, frame_ptr, frame.local_fields[i], "local_field");
                if frame.alias_locals[i] {
                    self.m.load(self.m.ptr_type(), field_addr, "alias")
                } else {
                    field_addr
                }
            })
            .collect();
    }

    /// Dispatches a poll function to whichever region should run next —
    /// `mir_fn.entry` on the very first poll, or a specific await-check
    /// block on a resumed one — by comparing the frame's own persisted
    /// `state` field against each [`FrameLayout::state_of`] discriminant
    /// in turn, mirroring `crate::shims`'s own chained-comparison enum
    /// dispatch (`emit_enum_variants`) rather than introducing a new
    /// `nether_llvm` switch primitive for what's usually a handful of
    /// cases.
    fn build_state_dispatch(&mut self, frame: &FrameLayout<'ctx>, entry: Block<'ctx>) {
        let frame_ptr = self
            .frame_ptr
            .expect("build_state_dispatch runs after build_frame_prologue");
        let state_ptr = self
            .m
            .struct_gep(frame.ty, frame_ptr, frame.state_field, "state_field");
        let state = self.m.load(self.m.int_type(64), state_ptr, "state");

        // A completed poll function must be idempotent: real concurrent
        // scheduling means it can legitimately be polled again after it
        // already returned `true` once (`FrameLayout::completed_state`'s
        // own docs — a real bug this guards against, not a defensive
        // extra). Checked first, ahead of every real per-state branch.
        let completed = self.m.const_int(
            self.m.int_type(64),
            frame.completed_state(),
            false,
        );
        let is_completed = self
            .m
            .int_compare(IntPredicate::EQ, state, completed, "is_completed");
        let already_done = self.m.append_block(self.llvm_fn, "poll_already_done");
        let dispatch = self.m.append_block(self.llvm_fn, "poll_dispatch_start");
        self.m.position_at_end(entry);
        self.m.cond_br(is_completed, already_done, dispatch);
        self.m.position_at_end(already_done);
        self.m.ret(Some(self.m.const_int(self.m.int_type(8), 1, false)));

        let states = frame.state_blocks();
        let mut check_block = dispatch;
        for (i, &block_id) in states.iter().enumerate() {
            self.m.position_at_end(check_block);
            let target = self.blocks[&block_id];
            if i + 1 == states.len() {
                self.m.br(target);
                break;
            }
            let want = self.m.const_int(self.m.int_type(64), i as u64, false);
            let is_state = self
                .m
                .int_compare(IntPredicate::EQ, state, want, "is_state");
            let next_check = self.m.append_block(self.llvm_fn, "poll_dispatch");
            self.m.cond_br(is_state, target, next_check);
            check_block = next_check;
        }
    }

    fn local_ty(&self, local: Local) -> Type {
        self.mir_fn.local_decl(local).ty.clone()
    }

    fn operand_ty(&self, op: &Operand) -> Type {
        match op {
            Operand::Local(l) => self.local_ty(*l),
            Operand::Literal(_, ty) => ty.clone(),
            Operand::Unit => Type::unit(),
            Operand::Fn(_) => Type::Error,
        }
    }

    // ---- Instructions ---------------------------------------------------

    fn gen_instr(&mut self, instr: &Instr) {
        match instr {
            Instr::Assign(place, rvalue) => self.gen_assign(place, rvalue),
            Instr::Clear(local) => {
                self.m
                    .store(self.locals[local.index()], self.m.const_null_ptr());
            }
            Instr::Retain(local) => self.gen_reference_change(*local, true),
            Instr::Release(local) => {
                self.gen_reference_change(*local, false);
                self.null_frame_field_after_release(*local);
            }
            // A transactional call-argument release (§3.3) — the
            // argument's own ownership is unaffected (see
            // `nether_mir::Instr::TransientRelease`'s own docs), so this
            // must *not* run `null_frame_field_after_release`: the local
            // is very often read again later in this same function.
            Instr::TransientRelease(local) => self.gen_reference_change(*local, false),
            Instr::WeakRetain(local) => {
                let ptr = self.load_scalar(*local);
                self.m.call(self.runtime.weak_retain, &[ptr], "");
            }
            Instr::WeakRelease(local) => {
                let ptr = self.load_scalar(*local);
                self.m.call(self.runtime.weak_release, &[ptr], "");
            }
        }
    }

    fn gen_reference_change(&self, local: Local, retain: bool) {
        let ty = self.local_ty(local);
        if alloc_kind(&ty, self.defs()) == AllocKind::Heap {
            let ptr = self.load_scalar(local);
            let function = if matches!(ty, Type::Unique(_)) {
                debug_assert!(!retain, "unique heap values are never retained");
                self.runtime.unique_free
            } else if retain {
                self.runtime.retain
            } else {
                self.runtime.release
            };
            self.m.call(function, &[ptr], "");
            return;
        }
        let shim = if retain {
            self.shims
                .retain_shim(self.m, self.layout, self.runtime, &ty)
        } else {
            self.shims.drop_shim(self.m, self.layout, self.runtime, &ty)
        };
        if let Some(function) = shim {
            self.m.call(function, &[self.locals[local.index()]], "");
        }
    }

    /// Only meaningful while building an `is_async` function's *poll*
    /// function (`frame.is_some()`) — a no-op for every ordinary
    /// function. Maintains the invariant `frame::frame_drop_shim` relies
    /// on: a directly heap-kind local's frame field is non-null *iff* it
    /// currently holds a live, owned reference. Generalizes
    /// `Instr::Clear`'s own existing null-tolerant-release convention
    /// (`nether_mir::node`'s docs — "later lexical drops remain valid
    /// because ARC release accepts null") from a moved-from unique local
    /// to every heap-kind local a suspended frame might outlive.
    ///
    /// Skipped for an alias-kind local (`FrameLayout::alias_locals`): its
    /// field holds a *pointer to caller-owned storage*, not a value this
    /// frame itself owns — nulling it would corrupt every later read of
    /// that parameter within this same poll call, and ordinary ARC
    /// scope-exit release should never target one anyway (a borrowed
    /// parameter is never released by its callee).
    fn null_frame_field_after_release(&self, local: Local) {
        let Some(frame) = &self.frame else {
            return;
        };
        if frame.alias_locals[local.index()] {
            return;
        }
        self.m.store(self.locals[local.index()], self.m.const_null_ptr());
    }

    fn load_scalar(&self, local: Local) -> Value<'ctx> {
        let ty = self.layout.llvm_type(&self.local_ty(local));
        self.m.load(ty, self.locals[local.index()], "v")
    }

    fn gen_assign(&mut self, place: &Place, rvalue: &Rvalue) {
        if place.projection.is_empty() {
            let dest_ty = self.local_ty(place.local);
            let val = self.gen_rvalue(rvalue, &dest_ty);
            self.store_value(place.local, &dest_ty, val);
            return;
        }
        // Every projected-place `Assign` is a plain store — `nether_mir`
        // never produces any other rvalue kind for one (see
        // `nether_mir::build::FnBuilder::lower_assign`).
        let Rvalue::Use(op) = rvalue else {
            unreachable!("nether_mir only ever builds Rvalue::Use for a projected Assign target");
        };
        let val = self.gen_operand(op);
        let ty = self.operand_ty(op);
        let addr = self.place_address(place);
        self.store_at(addr, &ty, val);
    }

    /// Stores `val` into local `l`'s own slot — a plain scalar `store`
    /// for anything else, or a byte copy for a stack aggregate (whose
    /// "value" is really just the address of some other storage; see
    /// this module's docs).
    fn store_value(&self, l: Local, ty: &Type, val: Value<'ctx>) {
        self.store_at(self.locals[l.index()], ty, val);
    }

    fn store_at(&self, addr: Value<'ctx>, ty: &Type, val: Value<'ctx>) {
        if is_aggregate(ty, self.defs()) {
            let llvm_ty = self.layout.llvm_type(ty);
            let size = self.m.size_of(llvm_ty);
            self.m.memcpy(addr, val, size);
        } else {
            self.m.store(addr, val);
        }
    }

    // ---- Operands ---------------------------------------------------------

    fn gen_operand(&self, op: &Operand) -> Value<'ctx> {
        match op {
            Operand::Local(l) => {
                let ty = self.local_ty(*l);
                let slot = self.locals[l.index()];
                if is_aggregate(&ty, self.defs()) {
                    slot
                } else {
                    self.m.load(self.layout.llvm_type(&ty), slot, "v")
                }
            }
            Operand::Literal(lit, ty) => self.gen_literal(lit, ty),
            Operand::Unit => self.gen_unit(),
            Operand::Fn(id) => {
                let f = self.funcs[id];
                f.as_global_value().as_pointer_value().into()
            }
        }
    }

    fn gen_unit(&self) -> Value<'ctx> {
        self.unit_slot
            .expect("Self::build sets unit_slot before any instruction codegen runs")
    }

    /// A scratch slot needed only partway through building a later block
    /// (a witness/array-runtime-call staging area, an aggregate/variant/
    /// tuple construction slot, ...), but which must still be a
    /// fixed-offset stack slot rather than a genuine dynamic allocation —
    /// see [`nether_llvm::ModuleCx::entry_alloca`]'s own docs for the real
    /// stack-corruption bug an ordinary `self.m.alloca` at the *current*
    /// (non-entry) block caused here. Every helper that needs its own
    /// scratch storage should call this instead of `self.m.alloca`
    /// directly.
    fn entry_alloca(&self, ty: nether_llvm::Ty<'ctx>, name: &str) -> Value<'ctx> {
        let entry = self
            .entry_block
            .expect("Self::build sets entry_block before any instruction codegen runs");
        self.m.entry_alloca(entry, ty, name)
    }

    fn gen_literal(&self, lit: &Literal, ty: &Type) -> Value<'ctx> {
        match lit {
            Literal::Int(v) => {
                let signed = matches!(ty, Type::Primitive(p) if Layout::is_signed(*p));
                self.m
                    .const_int(self.layout.llvm_type(ty), *v as u64, signed)
            }
            Literal::Float(v) => self.m.const_float(self.layout.llvm_type(ty), *v),
            Literal::Bool(b) => self.m.const_bool(*b),
            Literal::Char(c) => self.m.const_int(self.m.int_type(32), u64::from(*c), false),
            Literal::Str(s) => {
                let bytes = self.m.global_string_ptr(s, "str_lit");
                let len = self.m.const_int(self.m.int_type(64), s.len() as u64, false);
                self.m
                    .call(self.runtime.string_from_bytes, &[bytes, len], "str")
                    .expect("nether_rt_string_from_utf8 returns a value")
            }
        }
    }

    // ---- Places (assignment targets) ---------------------------------

    /// The address a projected [`Place`] resolves to — the base local's
    /// own heap pointer (loaded) or its `alloca`'s own address (a stack
    /// aggregate), followed by every field/index projection in order.
    fn place_address(&self, place: &Place) -> Value<'ctx> {
        let mut current_ty = self.local_ty(place.local);
        let mut current_addr = self.base_address(place.local, &current_ty);
        for (position, projection) in place.projection.iter().enumerate() {
            let next_ty = self
                .projection_type(&current_ty, projection)
                .unwrap_or(Type::Error);
            current_addr = match projection {
                Projection::Deref => current_addr,
                Projection::Field(index) => {
                    self.struct_field_address(&current_ty, current_addr, *index)
                }
                Projection::VariantField { variant, index } => {
                    self.variant_field_address(&current_ty, current_addr, *variant, *index)
                }
                Projection::Index(idx_op) => {
                    let idx_val = self.gen_array_index(idx_op);
                    if let Type::FixedArray(element, _) = &current_ty {
                        let size = self.m.size_of(self.layout.llvm_type(element));
                        let offset = self.m.int_mul(idx_val, size, "fixed_index_offset");
                        self.m.gep_bytes(current_addr, offset, "fixed_elem_ptr")
                    } else {
                        self.m
                            .call(self.runtime.array_get, &[current_addr, idx_val], "elem_ptr")
                            .expect("nether_rt_array_get returns a value")
                    }
                }
            };
            current_ty = next_ty;
            if position + 1 < place.projection.len() && !is_aggregate(&current_ty, self.defs()) {
                current_addr = self.m.load(
                    self.layout.llvm_type(&current_ty),
                    current_addr,
                    "projected_base",
                );
            }
        }
        current_addr
    }

    /// Address used to form a Nether reference. Stack values borrow their
    /// alloca directly; heap values already are pointers, so borrowing them
    /// yields the object pointer stored in the local slot.
    fn address_of_place(&self, place: &Place) -> Value<'ctx> {
        if !place.projection.is_empty() {
            return self.place_address(place);
        }
        let ty = self.local_ty(place.local);
        if alloc_kind(&ty, self.defs()) == AllocKind::Heap {
            self.m.load(
                self.m.ptr_type(),
                self.locals[place.local.index()],
                "borrow",
            )
        } else {
            self.locals[place.local.index()]
        }
    }

    fn projection_type(&self, base: &Type, projection: &Projection) -> Option<Type> {
        if matches!(projection, Projection::Deref) {
            return match base {
                Type::Ref(inner) | Type::MutRef(inner) => Some((**inner).clone()),
                _ => None,
            };
        }
        let base = base.strip_indirection();
        match projection {
            Projection::Deref => unreachable!(),
            Projection::Field(index) => match base {
                Type::Tuple(items) => items.get(*index as usize).cloned(),
                _ => self
                    .layout
                    .sigs
                    .type_fields(base)?
                    .get(*index as usize)
                    .cloned(),
            },
            Projection::VariantField { variant, index } => self
                .layout
                .sigs
                .enum_payload(base, *variant)?
                .get(*index as usize)
                .cloned(),
            Projection::Index(_) => match base {
                Type::Array(elem) => Some((**elem).clone()),
                Type::FixedArray(elem, _) => Some((**elem).clone()),
                _ => None,
            },
        }
    }

    /// The address of local `l`'s own object: for a heap type, the
    /// pointer *held in* the local's slot (loaded); for a stack
    /// aggregate, the slot itself, which already *is* the object's
    /// address.
    fn base_address(&self, l: Local, ty: &Type) -> Value<'ctx> {
        let slot = self.locals[l.index()];
        if is_aggregate(ty, self.defs()) {
            slot
        } else {
            self.m.load(self.m.ptr_type(), slot, "base")
        }
    }

    fn struct_field_address(
        &self,
        base_ty: &Type,
        base_addr: Value<'ctx>,
        index: u32,
    ) -> Value<'ctx> {
        let base_ty = base_ty.strip_indirection();
        let struct_ty = match base_ty {
            Type::Struct(_, _) | Type::TupleStruct(_, _) => self.layout.struct_layout(base_ty).ty,
            Type::Tuple(items) => {
                let fields = items
                    .iter()
                    .map(|item| self.layout.llvm_type(item))
                    .collect::<Vec<_>>();
                self.m.struct_type(&fields)
            }
            other => panic!("field projection on non-struct type {other:?}"),
        };
        self.m.struct_gep(struct_ty, base_addr, index, "field")
    }

    fn variant_field_address(
        &self,
        base_ty: &Type,
        base_addr: Value<'ctx>,
        variant: u32,
        index: u32,
    ) -> Value<'ctx> {
        match base_ty {
            Type::Enum(_, _) => {}
            other => panic!("variant-field projection on non-enum type {other:?}"),
        }
        let el = self.layout.enum_layout(base_ty);
        let gep_index = *el
            .field_offsets
            .get(&(variant, index))
            .expect("valid variant/field index (typecheck already validated the pattern)");
        self.m.struct_gep(el.ty, base_addr, gep_index, "payload")
    }
}
