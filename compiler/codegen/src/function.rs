use std::collections::HashMap;

use nether_ast::{BinaryOp, Literal, Symbol, UnaryOp};
use nether_llvm::{Block, FloatPredicate, Func, IntPredicate, ModuleCx, Value};
use nether_mir::{
    CallTarget, Instr, Local, MirFunction, Operand, Place, Projection, Rvalue, Terminator,
};
use nether_monomorphization::MonoFnId;
use nether_typecheck::{alloc_kind, AllocKind, PrimitiveKind, Type};

use crate::layout::Layout;
use crate::runtime::Runtime;
use crate::shims::Shims;

mod aggregate;
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
    pub(crate) mir_functions: &'m HashMap<MonoFnId, &'m MirFunction>,
}

pub(crate) fn build_function<'m, 'ctx>(
    env: &'m FunctionEnv<'m, 'ctx>,
    mir_fn: &'m MirFunction,
    llvm_fn: Func<'ctx>,
) {
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
    /// module's own docs on aggregate addressing.
    locals: Vec<Value<'ctx>>,
    blocks: HashMap<nether_mir::BlockId, Block<'ctx>>,
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

        for (i, decl) in self.mir_fn.locals.iter().enumerate() {
            let ty = self.layout.llvm_type(&decl.ty);
            let slot = self.m.alloca(ty, &format!("local{i}"));
            self.locals.push(slot);
        }
        if self.mir_fn.is_closure {
            let env = self.m.param(self.llvm_fn, 0);
            let capture_tys: Vec<Type> = self
                .mir_fn
                .closure_captures
                .iter()
                .map(|&local| self.local_ty(local))
                .collect();
            let mut fields = vec![self.m.ptr_type()];
            fields.extend(capture_tys.iter().map(|ty| self.layout.llvm_type(ty)));
            let env_ty = self.m.struct_type(&fields);
            for (index, (&local, ty)) in self
                .mir_fn
                .closure_captures
                .iter()
                .zip(capture_tys.iter())
                .enumerate()
            {
                let field = self.m.struct_gep(env_ty, env, index as u32 + 1, "capture");
                // A captured binding's storage is the environment field
                // itself. Rebinding/mutating it therefore persists across
                // calls to this closure while remaining independent from
                // the outer variable (capture-by-value semantics).
                let _ = ty;
                self.locals[local.index()] = field;
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

        for block in &self.mir_fn.blocks {
            let name = format!("bb{}", block_num(block.id));
            self.blocks
                .insert(block.id, self.m.append_block(self.llvm_fn, &name));
        }
        self.m.br(self.blocks[&self.mir_fn.entry]);

        for block in &self.mir_fn.blocks {
            self.m.position_at_end(self.blocks[&block.id]);
            for instr in &block.instrs {
                self.gen_instr(instr);
            }
            self.gen_terminator(&block.terminator);
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
            Instr::Release(local) => self.gen_reference_change(*local, false),
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
        let unit_ty = self.m.struct_type(&[]);
        self.m.alloca(unit_ty.into(), "unit")
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
