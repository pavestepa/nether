use std::collections::HashMap;

use nether_ast::{BinaryOp, Literal, Symbol, UnaryOp};
use nether_llvm::{Block, Func, FloatPredicate, IntPredicate, ModuleCx, Value};
use nether_mir::{CallTarget, Instr, Local, MirFunction, Operand, Place, Projection, Rvalue, Terminator};
use nether_monomorphization::MonoFnId;
use nether_typecheck::{alloc_kind, AllocKind, PrimitiveKind, Type};

use crate::layout::Layout;
use crate::runtime::Runtime;
use crate::shims::Shims;

/// Whether a value of `ty` is addressed directly through its own
/// `alloca`/heap pointer (an aggregate GEPed into in place) rather than
/// loaded as a scalar SSA value — see `crate` module docs and
/// `nether_llvm`'s own "never `insertvalue`/`extractvalue`" design note.
pub(crate) fn is_aggregate(ty: &Type, defs: &nether_resolver::Definitions) -> bool {
    match ty {
        Type::Tuple(_) | Type::Enum(_, _) => true,
        Type::Struct(_, _) | Type::TupleStruct(_, _) => alloc_kind(ty, defs) == AllocKind::Stack,
        _ => false,
    }
}

pub fn build_function<'m, 'ctx>(
    m: &'m ModuleCx<'ctx>,
    layout: &'m Layout<'m, 'ctx>,
    runtime: &'m Runtime<'ctx>,
    shims: &'m Shims<'ctx>,
    funcs: &'m HashMap<MonoFnId, Func<'ctx>>,
    mir_fn: &'m MirFunction,
    llvm_fn: Func<'ctx>,
) {
    let mut fc = FnCodegen { m, layout, runtime, shims, funcs, mir_fn, llvm_fn, locals: Vec::new(), blocks: HashMap::new() };
    fc.build();
}

struct FnCodegen<'m, 'ctx> {
    m: &'m ModuleCx<'ctx>,
    layout: &'m Layout<'m, 'ctx>,
    runtime: &'m Runtime<'ctx>,
    shims: &'m Shims<'ctx>,
    funcs: &'m HashMap<MonoFnId, Func<'ctx>>,
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
            if is_aggregate(&ty, self.defs()) {
                // An aggregate parameter is passed by pointer (see
                // `crate::declare`'s matching signature choice) — the
                // incoming value already *is* the address to use as this
                // local's own slot, no separate `alloca`/copy needed.
                // This aliases the caller's own storage rather than
                // copying it, which is exactly `mut`'s documented
                // semantics for a stack-kind parameter (`arc-model.md`
                // §3.6) and is unobservable for a non-`mut` one (never
                // reassigned).
                self.locals[local.index()] = param_val;
            } else {
                self.m.store(self.locals[local.index()], param_val);
            }
        }

        for block in &self.mir_fn.blocks {
            let name = format!("bb{}", block_num(block.id));
            self.blocks.insert(block.id, self.m.append_block(self.llvm_fn, &name));
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
            let function = if retain { self.runtime.retain } else { self.runtime.release };
            self.m.call(function, &[ptr], "");
            return;
        }
        let shim = if retain {
            self.shims.retain_shim(self.m, self.layout, self.runtime, &ty)
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
                self.m.const_int(self.layout.llvm_type(ty), *v as u64, signed)
            }
            Literal::Float(v) => self.m.const_float(self.layout.llvm_type(ty), *v),
            Literal::Bool(b) => self.m.const_bool(*b),
            Literal::Char(c) => self.m.const_int(self.m.int_type(32), u64::from(*c), false),
            Literal::Str(s) => {
                let bytes = self.m.global_string_ptr(s, "str_lit");
                let len = self.m.const_int(self.m.int_type(64), s.len() as u64, false);
                self.m.call(self.runtime.string_from_bytes, &[bytes, len], "str").expect("nether_rt_string_from_utf8 returns a value")
            }
        }
    }

    // ---- Places (assignment targets) ---------------------------------

    /// The address a projected [`Place`] resolves to — the base local's
    /// own heap pointer (loaded) or its `alloca`'s own address (a stack
    /// aggregate), followed by every field/index projection in order.
    fn place_address(&self, place: &Place) -> Value<'ctx> {
        let mut current_ty = self.local_ty(place.local);
        let mut current_addr =
            self.base_address(place.local, &current_ty);
        for (position, projection) in
            place.projection.iter().enumerate()
        {
            let next_ty = self
                .projection_type(&current_ty, projection)
                .unwrap_or(Type::Error);
            current_addr = match projection {
                Projection::Field(index) => self
                    .struct_field_address(
                        &current_ty,
                        current_addr,
                        *index,
                    ),
                Projection::VariantField { variant, index } => self
                    .variant_field_address(
                        &current_ty,
                        current_addr,
                        *variant,
                        *index,
                ),
                Projection::Index(idx_op) => {
                    let idx_val = self.gen_array_index(idx_op);
                    self.m
                        .call(
                            self.runtime.array_get,
                            &[current_addr, idx_val],
                            "elem_ptr",
                        )
                        .expect(
                            "nether_rt_array_get returns a value",
                        )
                }
            };
            current_ty = next_ty;
            if position + 1 < place.projection.len()
                && !is_aggregate(&current_ty, self.defs())
            {
                current_addr = self.m.load(
                    self.layout.llvm_type(&current_ty),
                    current_addr,
                    "projected_base",
                );
            }
        }
        current_addr
    }

    fn projection_type(
        &self,
        base: &Type,
        projection: &Projection,
    ) -> Option<Type> {
        match projection {
            Projection::Field(index) => match base {
                Type::Tuple(items) => {
                    items.get(*index as usize).cloned()
                }
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

    fn struct_field_address(&self, base_ty: &Type, base_addr: Value<'ctx>, index: u32) -> Value<'ctx> {
        let struct_ty = match base_ty {
            Type::Struct(_, _) | Type::TupleStruct(_, _) => {
                self.layout.struct_layout(base_ty).ty
            }
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

    fn variant_field_address(&self, base_ty: &Type, base_addr: Value<'ctx>, variant: u32, index: u32) -> Value<'ctx> {
        match base_ty {
            Type::Enum(_, _) => {}
            other => panic!("variant-field projection on non-enum type {other:?}"),
        }
        let el = self.layout.enum_layout(base_ty);
        let gep_index = *el.field_offsets.get(&(variant, index)).expect("valid variant/field index (typecheck already validated the pattern)");
        self.m.struct_gep(el.ty, base_addr, gep_index, "payload")
    }

    // ---- Rvalues -----------------------------------------------------

    fn gen_rvalue(&mut self, rvalue: &Rvalue, dest_ty: &Type) -> Value<'ctx> {
        match rvalue {
            Rvalue::Use(op) => self.gen_operand(op),
            Rvalue::Unary(op, a) => self.gen_unary(*op, a),
            Rvalue::Binary(op, a, b) => self.gen_binary(*op, a, b),
            Rvalue::Call { target, args } => self.gen_call(target, args, dest_ty),
            Rvalue::CallBuiltin { name, args } => self.gen_call_builtin(name, args, dest_ty),
            Rvalue::CallArrayMethod { receiver, method, args } => self.gen_call_array_method(receiver, method, args, dest_ty),
            Rvalue::Field { base, index } => self.gen_field_read(base, *index, dest_ty),
            Rvalue::VariantField { base, variant, index } => self.gen_variant_field_read(base, *variant, *index, dest_ty),
            Rvalue::Discriminant(base) => self.gen_discriminant(base),
            Rvalue::Index { base, index } => self.gen_index_read(base, index, dest_ty),
            Rvalue::Construct { ty, fields } => self.gen_construct(*ty, fields, dest_ty),
            Rvalue::ConstructVariant { enum_id, variant, payload } => self.gen_construct_variant(*enum_id, *variant, payload, dest_ty),
            Rvalue::Tuple(items) => self.gen_tuple(items, dest_ty),
            Rvalue::Array(items) => self.gen_array_literal(items, dest_ty),
            Rvalue::Concat(items) => self.gen_concat(items),
            Rvalue::ToString(op) => self.gen_to_string(op),
            Rvalue::Closure { function, captures } => self.gen_closure(*function, captures),
        }
    }

    fn gen_unary(&self, op: UnaryOp, a: &Operand) -> Value<'ctx> {
        let ty = self.operand_ty(a);
        let av = self.gen_operand(a);
        match op {
            UnaryOp::Neg => match &ty {
                Type::Primitive(p) if Layout::is_float(*p) => self.m.float_neg(av, "neg"),
                _ => self.m.int_neg(av, "neg"),
            },
            UnaryOp::Not => self.m.int_not(av, "not"),
        }
    }

    fn gen_binary(&self, op: BinaryOp, a: &Operand, b: &Operand) -> Value<'ctx> {
        let ty = self.operand_ty(a);
        let av = self.gen_operand(a);
        let bv = self.gen_operand(b);
        let is_float = matches!(&ty, Type::Primitive(p) if Layout::is_float(*p));
        let signed = matches!(&ty, Type::Primitive(p) if Layout::is_signed(*p));
        if is_float {
            return match op {
                BinaryOp::Add => self.m.float_add(av, bv, "add"),
                BinaryOp::Sub => self.m.float_sub(av, bv, "sub"),
                BinaryOp::Mul => self.m.float_mul(av, bv, "mul"),
                BinaryOp::Div | BinaryOp::Rem => self.m.float_div(av, bv, "div"),
                BinaryOp::Eq => self.m.float_compare(FloatPredicate::OEQ, av, bv, "eq"),
                BinaryOp::Ne => self.m.float_compare(FloatPredicate::ONE, av, bv, "ne"),
                BinaryOp::Lt => self.m.float_compare(FloatPredicate::OLT, av, bv, "lt"),
                BinaryOp::Le => self.m.float_compare(FloatPredicate::OLE, av, bv, "le"),
                BinaryOp::Gt => self.m.float_compare(FloatPredicate::OGT, av, bv, "gt"),
                BinaryOp::Ge => self.m.float_compare(FloatPredicate::OGE, av, bv, "ge"),
                BinaryOp::And | BinaryOp::Or => unreachable!("`&&`/`||` never operate on float operands (typecheck)"),
            };
        }
        match op {
            BinaryOp::Add => self.m.int_add(av, bv, "add"),
            BinaryOp::Sub => self.m.int_sub(av, bv, "sub"),
            BinaryOp::Mul => self.m.int_mul(av, bv, "mul"),
            BinaryOp::Div => {
                if signed {
                    self.m.int_signed_div(av, bv, "div")
                } else {
                    self.m.int_unsigned_div(av, bv, "div")
                }
            }
            BinaryOp::Rem => {
                if signed {
                    self.m.int_signed_rem(av, bv, "rem")
                } else {
                    self.m.int_unsigned_rem(av, bv, "rem")
                }
            }
            BinaryOp::Eq => self.m.int_compare(IntPredicate::EQ, av, bv, "eq"),
            BinaryOp::Ne => self.m.int_compare(IntPredicate::NE, av, bv, "ne"),
            BinaryOp::Lt => self.m.int_compare(if signed { IntPredicate::SLT } else { IntPredicate::ULT }, av, bv, "lt"),
            BinaryOp::Le => self.m.int_compare(if signed { IntPredicate::SLE } else { IntPredicate::ULE }, av, bv, "le"),
            BinaryOp::Gt => self.m.int_compare(if signed { IntPredicate::SGT } else { IntPredicate::UGT }, av, bv, "gt"),
            BinaryOp::Ge => self.m.int_compare(if signed { IntPredicate::SGE } else { IntPredicate::UGE }, av, bv, "ge"),
            BinaryOp::And => self.m.int_and(av, bv, "and"),
            BinaryOp::Or => self.m.int_or(av, bv, "or"),
        }
    }

    fn gen_call(&mut self, target: &CallTarget, args: &[Operand], dest_ty: &Type) -> Value<'ctx> {
        let arg_vals: Vec<Value<'ctx>> = args.iter().map(|a| self.gen_operand(a)).collect();
        let result = match target {
            CallTarget::Fn(id) => {
                let f = *self.funcs.get(id).expect("every CallTarget::Fn refers to a function declared in this same module");
                self.m.call(f, &arg_vals, "call").unwrap_or_else(|| self.gen_unit())
            }
            CallTarget::Dynamic(callee) => {
                let closure = self.gen_operand(callee);
                let code = self.m.load(self.m.ptr_type(), closure, "closure_fn");
                let (params, ret) = match self.operand_ty(callee) {
                    Type::Function(params, ret) => (params, *ret),
                    other => panic!("dynamic call target is not a function: {other:?}"),
                };
                let mut param_tys = vec![self.m.ptr_type()];
                param_tys.extend(params.iter().map(|ty| {
                    if is_aggregate(ty, self.defs()) {
                        self.m.ptr_type()
                    } else {
                        self.layout.llvm_type(ty)
                    }
                }));
                let ret_ty = if matches!(&ret, Type::Tuple(items) if items.is_empty()) {
                    None
                } else {
                    Some(self.layout.llvm_type(&ret))
                };
                let fn_ty = self.m.fn_type(&param_tys, ret_ty);
                let mut dynamic_args = Vec::with_capacity(arg_vals.len() + 1);
                dynamic_args.push(closure);
                dynamic_args.extend(arg_vals);
                self.m.indirect_call(fn_ty, code, &dynamic_args, "closure_call").unwrap_or_else(|| self.gen_unit())
            }
        };
        if is_aggregate(dest_ty, self.defs()) {
            // The callee returns this aggregate by value (see
            // `Self::gen_terminator`'s matching `Return` handling) — spill
            // it into a fresh `alloca` so it rejoins this crate's uniform
            // "an aggregate operand is always an address" representation.
            let slot = self.m.alloca(self.layout.llvm_type(dest_ty), "call_result");
            self.m.store(slot, result);
            slot
        } else {
            result
        }
    }

    fn gen_closure(&self, function: MonoFnId, captures: &[Operand]) -> Value<'ctx> {
        let capture_tys: Vec<Type> = captures.iter().map(|capture| self.operand_ty(capture)).collect();
        let mut fields = vec![self.m.ptr_type()];
        fields.extend(capture_tys.iter().map(|ty| self.layout.llvm_type(ty)));
        let env_ty = self.m.struct_type(&fields);
        let size = self.m.size_of(env_ty.into());
        let drop_fn = self.func_ptr_or_null(
            self.shims
                .closure_drop_shim(self.m, self.layout, self.runtime, &capture_tys),
        );
        let env = self
            .m
            .call(self.runtime.alloc, &[size, drop_fn], "closure")
            .expect("nether_rt_arc_alloc returns a closure environment");
        let code_field = self.m.struct_gep(env_ty, env, 0, "code");
        let code = self.funcs[&function].as_global_value().as_pointer_value().into();
        self.m.store(code_field, code);
        for (index, (capture, ty)) in captures.iter().zip(capture_tys.iter()).enumerate() {
            let field = self.m.struct_gep(env_ty, env, index as u32 + 1, "capture");
            self.store_at(field, ty, self.gen_operand(capture));
        }
        env
    }

    fn gen_call_builtin(&mut self, name: &Symbol, args: &[Operand], dest_ty: &Type) -> Value<'ctx> {
        match name.as_str() {
            "__weak_upgrade" => {
                let weak_ptr = self.gen_operand(&args[0]);
                self.gen_weak_upgrade(dest_ty, weak_ptr)
            }
            _ => {
                if args.is_empty() {
                    if name.as_str() == "println" {
                        let bytes = self.m.global_string_ptr("", "empty_str");
                        let len = self.m.const_int(self.m.int_type(64), 0, false);
                        let empty = self
                            .m
                            .call(self.runtime.string_from_bytes, &[bytes, len], "empty")
                            .expect("string constructor returns a value");
                        self.m.call(self.runtime.println, &[empty], "");
                        self.m.call(self.runtime.release, &[empty], "");
                    }
                    return self.gen_unit();
                }
                for (index, arg) in args.iter().enumerate() {
                    let value = self.gen_operand(arg);
                    let is_last = index + 1 == args.len();
                    let function = if name.as_str() == "println" && is_last {
                        self.runtime.println
                    } else {
                        self.runtime.print
                    };
                    self.m.call(function, &[value], "");
                }
                self.gen_unit()
            }
        }
    }

    /// `weak T`'s only read: `nether_hir` desugars every `weak T` place
    /// read into a call to this builtin (its own module docs), typed as
    /// `Option<T>` — mirrors [`Self::gen_array_pop`]'s own boolean
    /// -success-to-`Option` construction, backed by
    /// `nether_rt_arc_weak_upgrade` instead of `nether_rt_array_pop`.
    fn gen_weak_upgrade(&self, option_ty: &Type, weak_ptr: Value<'ctx>) -> Value<'ctx> {
        let elem_ty = match option_ty {
            Type::Enum(_, args) => args.first().cloned().unwrap_or(Type::Error),
            other => panic!("a weak read's result must be an Option, found {other:?}"),
        };
        let el = self.layout.enum_layout(option_ty);
        let out_slot = self.m.alloca(self.m.ptr_type(), "upgraded_ptr");
        // `nether_rt_arc_weak_upgrade` returns `i8` (0/1), not LLVM's
        // native `i1` — see `runtime.rs`'s module docs; `select` needs an
        // actual `i1` condition.
        let has_value_i8 =
            self.m.call(self.runtime.weak_upgrade, &[weak_ptr, out_slot], "has_value").expect("nether_rt_arc_weak_upgrade returns a value");
        let has_value = self.m.int_cast(has_value_i8, self.m.bool_type(), false, "has_value");

        let result_slot = self.m.alloca(el.ty.into(), "upgrade_result");
        let tag_ptr = self.m.struct_gep(el.ty, result_slot, 0, "tag_ptr");
        // `Option`'s builtin variant order is `Some = 0, None = 1` (see
        // `nether_resolver::def::builtin_definitions`).
        let some_tag = self.m.const_int(self.m.int_type(64), 0, false);
        let none_tag = self.m.const_int(self.m.int_type(64), 1, false);
        let tag = self.m.select(has_value, some_tag, none_tag, "tag");
        self.m.store(tag_ptr, tag);

        if let Some(&payload_index) = el.field_offsets.get(&(0, 0)) {
            let payload_ptr = self.m.struct_gep(el.ty, result_slot, payload_index, "payload_ptr");
            // `elem_ty` is always heap-kind (`weak T` only ever wraps a
            // heap type — `nether_typecheck`'s own validation), so the
            // upgraded pointer `nether_rt_arc_weak_upgrade` wrote into
            // `out_slot` *is* the payload value directly, no further
            // addressing needed.
            let elem_val = self.m.load(self.m.ptr_type(), out_slot, "elem");
            self.store_at(payload_ptr, &elem_ty, elem_val);
        }
        result_slot
    }

    fn gen_call_array_method(&mut self, receiver: &Operand, method: &Symbol, args: &[Operand], dest_ty: &Type) -> Value<'ctx> {
        let recv = self.gen_operand(receiver);
        match method.as_str() {
            "len" => self.m.call(self.runtime.array_len, &[recv], "len").expect("nether_rt_array_len returns a value"),
            "push" => {
                let elem_ty = self.operand_ty(&args[0]);
                let elem_val = self.gen_operand(&args[0]);
                let elem_slot = self.m.alloca(self.layout.llvm_type(&elem_ty), "push_elem");
                self.store_at(elem_slot, &elem_ty, elem_val);
                self.m.call(self.runtime.array_push, &[recv, elem_slot], "").unwrap_or_else(|| self.gen_unit())
            }
            "pop" => {
                // `dest_ty` is `Option<T>` — construct it directly from
                // `nether_array_pop`'s own boolean-success shape rather
                // than routing through a separate `ConstructVariant` step.
                self.gen_array_pop(dest_ty, recv)
            }
            other => panic!("nether_codegen: unknown array method `{other}`"),
        }
    }

    fn gen_array_pop(&self, option_ty: &Type, recv: Value<'ctx>) -> Value<'ctx> {
        let elem_ty = match option_ty {
            Type::Enum(_, args) => args.first().cloned().unwrap_or(Type::Error),
            other => panic!("Array::pop's result must be an Option, found {other:?}"),
        };
        let el = self.layout.enum_layout(option_ty);
        let out_slot = self.m.alloca(self.layout.llvm_type(&elem_ty), "pop_elem");
        // `nether_rt_array_pop` returns `i8` (0/1), not LLVM's `i1` — see
        // `runtime.rs`'s module docs on why `bool` crosses this ABI
        // boundary as a byte; `select` needs an actual `i1` condition.
        let has_value_i8 = self.m.call(self.runtime.array_pop, &[recv, out_slot], "has_value").expect("nether_rt_array_pop returns a value");
        let has_value = self.m.int_cast(has_value_i8, self.m.bool_type(), false, "has_value");

        let result_slot = self.m.alloca(el.ty.into(), "pop_result");
        let tag_ptr = self.m.struct_gep(el.ty, result_slot, 0, "tag_ptr");
        // `Option`'s builtin variant order is `Some = 0, None = 1` (see
        // `nether_resolver::def::builtin_definitions`).
        let some_tag = self.m.const_int(self.m.int_type(64), 0, false);
        let none_tag = self.m.const_int(self.m.int_type(64), 1, false);
        let tag = self.m.select(has_value, some_tag, none_tag, "tag");
        self.m.store(tag_ptr, tag);

        if let Some(&payload_index) = el.field_offsets.get(&(0, 0)) {
            let payload_ptr = self.m.struct_gep(el.ty, result_slot, payload_index, "payload_ptr");
            let elem_val = self.m.load(self.layout.llvm_type(&elem_ty), out_slot, "elem");
            self.store_at(payload_ptr, &elem_ty, elem_val);
        }
        result_slot
    }

    fn gen_field_read(&self, base: &Operand, index: u32, dest_ty: &Type) -> Value<'ctx> {
        let base_ty = self.operand_ty(base);
        let base_val = self.gen_operand(base);
        let addr = self.struct_field_address(&base_ty, base_val, index);
        self.load_value(addr, dest_ty)
    }

    fn gen_variant_field_read(&self, base: &Operand, variant: u32, index: u32, dest_ty: &Type) -> Value<'ctx> {
        let base_ty = self.operand_ty(base);
        let base_val = self.gen_operand(base);
        let addr = self.variant_field_address(&base_ty, base_val, variant, index);
        self.load_value(addr, dest_ty)
    }

    fn load_value(&self, addr: Value<'ctx>, ty: &Type) -> Value<'ctx> {
        if is_aggregate(ty, self.defs()) {
            addr
        } else {
            self.m.load(self.layout.llvm_type(ty), addr, "v")
        }
    }

    fn gen_discriminant(&self, base: &Operand) -> Value<'ctx> {
        let base_ty = self.operand_ty(base);
        match &base_ty {
            Type::Enum(_, _) => {}
            other => panic!("Discriminant on non-enum type {other:?}"),
        }
        let base_val = self.gen_operand(base);
        let el = self.layout.enum_layout(&base_ty);
        let tag_ptr = self.m.struct_gep(el.ty, base_val, 0, "tag_ptr");
        self.m.load(self.m.int_type(64), tag_ptr, "tag")
    }

    fn gen_index_read(&self, base: &Operand, index: &Operand, dest_ty: &Type) -> Value<'ctx> {
        let base_val = self.gen_operand(base);
        let idx_val = self.gen_array_index(index);
        let addr = self.m.call(self.runtime.array_get, &[base_val, idx_val], "elem_ptr").expect("nether_rt_array_get returns a value");
        self.load_value(addr, dest_ty)
    }

    fn gen_array_index(&self, index: &Operand) -> Value<'ctx> {
        let value = self.gen_operand(index);
        match self.operand_ty(index) {
            Type::Primitive(PrimitiveKind::Usize)
            | Type::Primitive(PrimitiveKind::U64) => value,
            Type::Primitive(kind) if kind.is_integer() => self.m.int_cast(
                value,
                self.m.int_type(64),
                Layout::is_signed(kind),
                "array_index",
            ),
            _ => value,
        }
    }

    fn gen_construct(&mut self, _id: nether_resolver::DefId, fields: &[Operand], dest_ty: &Type) -> Value<'ctx> {
        let sl = self.layout.struct_layout(dest_ty);
        let field_vals: Vec<Value<'ctx>> = fields.iter().map(|f| self.gen_operand(f)).collect();
        let field_tys: Vec<Type> = self.layout.sigs.type_fields(dest_ty).unwrap_or_default();

        let heap = alloc_kind(dest_ty, self.defs()) == AllocKind::Heap;
        let addr = if heap {
            let size = self.m.size_of(sl.ty.into());
            let drop_fn = self.func_ptr_or_null(self.shims.own_drop_shim(self.m, self.layout, self.runtime, dest_ty));
            self.m.call(self.runtime.alloc, &[size, drop_fn], "obj").expect("nether_rt_arc_alloc returns a value")
        } else {
            self.m.alloca(sl.ty.into(), "agg")
        };
        for (i, val) in field_vals.into_iter().enumerate() {
            let field_ptr = self.m.struct_gep(sl.ty, addr, i as u32, "field");
            let field_ty = field_tys.get(i).cloned().unwrap_or(Type::Error);
            self.store_at(field_ptr, &field_ty, val);
        }
        addr
    }

    fn gen_construct_variant(&mut self, enum_id: nether_resolver::DefId, variant: u32, payload: &[Operand], dest_ty: &Type) -> Value<'ctx> {
        debug_assert!(matches!(dest_ty, Type::Enum(id, _) if *id == enum_id));
        let el = self.layout.enum_layout(dest_ty);
        let payload_tys = self.layout.sigs.enum_payload(dest_ty, variant).unwrap_or_default();
        let payload_vals: Vec<Value<'ctx>> = payload.iter().map(|p| self.gen_operand(p)).collect();

        let _ = dest_ty;
        let slot = self.m.alloca(el.ty.into(), "variant");
        let tag_ptr = self.m.struct_gep(el.ty, slot, 0, "tag_ptr");
        self.m.store(tag_ptr, self.m.const_int(self.m.int_type(64), u64::from(variant), false));
        for (i, val) in payload_vals.into_iter().enumerate() {
            let gep_index = *el.field_offsets.get(&(variant, i as u32)).expect("valid variant/field index");
            let field_ptr = self.m.struct_gep(el.ty, slot, gep_index, "payload_field");
            let field_ty = payload_tys.get(i).cloned().unwrap_or(Type::Error);
            self.store_at(field_ptr, &field_ty, val);
        }
        slot
    }

    fn gen_tuple(&mut self, items: &[Operand], dest_ty: &Type) -> Value<'ctx> {
        let elem_tys = match dest_ty {
            Type::Tuple(tys) => tys.clone(),
            other => panic!("Rvalue::Tuple with non-tuple destination type {other:?}"),
        };
        let llvm_ty = self.layout.llvm_type(dest_ty);
        let slot = self.m.alloca(llvm_ty, "tuple");
        let struct_ty = match llvm_ty {
            nether_llvm::Ty::StructType(t) => t,
            _ => unreachable!("Tuple always lowers to a struct type"),
        };
        for (i, item) in items.iter().enumerate() {
            let val = self.gen_operand(item);
            let field_ptr = self.m.struct_gep(struct_ty, slot, i as u32, "field");
            let ty = elem_tys.get(i).cloned().unwrap_or(Type::Error);
            self.store_at(field_ptr, &ty, val);
        }
        slot
    }

    fn gen_array_literal(&mut self, items: &[Operand], dest_ty: &Type) -> Value<'ctx> {
        let elem_ty = match dest_ty {
            Type::Array(t) => (**t).clone(),
            other => panic!("Rvalue::Array with non-array destination type {other:?}"),
        };
        let elem_llvm_ty = self.layout.llvm_type(&elem_ty);
        let elem_size = self.m.size_of(elem_llvm_ty);
        let cap = self.m.const_int(self.m.int_type(64), items.len() as u64, false);
        let elem_retain = self.func_ptr_or_null(self.shims.retain_shim(self.m, self.layout, self.runtime, &elem_ty));
        let elem_drop = self.func_ptr_or_null(self.shims.drop_shim(self.m, self.layout, self.runtime, &elem_ty));
        let arr = self
            .m
            .call(self.runtime.array_new, &[elem_size, cap, elem_retain, elem_drop], "arr")
            .expect("nether_rt_array_new returns a value");
        let elem_slot = self.m.alloca(elem_llvm_ty, "elem");
        for item in items {
            let val = self.gen_operand(item);
            self.store_at(elem_slot, &elem_ty, val);
            self.m.call(self.runtime.array_push, &[arr, elem_slot], "");
        }
        arr
    }

    fn gen_concat(&mut self, items: &[Operand]) -> Value<'ctx> {
        let mut iter = items.iter();
        let first = iter.next().expect("Concat always has at least one piece (nether_hir's own desugaring)");
        let mut acc = self.gen_operand(first);
        let Some(second) = iter.next() else {
            // `Concat` is a constructing rvalue and therefore owes its
            // destination an independent +1 even when desugaring
            // produced only one piece.
            self.m.call(self.runtime.retain, &[acc], "");
            return acc;
        };
        acc = self
            .m
            .call(
                self.runtime.string_concat,
                &[acc, self.gen_operand(second)],
                "concat",
            )
            .expect("nether_rt_string_concat returns a value");
        for item in iter {
            let piece = self.gen_operand(item);
            let previous = acc;
            acc = self
                .m
                .call(self.runtime.string_concat, &[previous, piece], "concat")
                .expect("nether_rt_string_concat returns a value");
            self.m.call(self.runtime.release, &[previous], "");
        }
        acc
    }

    fn gen_to_string(&mut self, op: &Operand) -> Value<'ctx> {
        let ty = self.operand_ty(op);
        let val = self.gen_operand(op);
        match &ty {
            Type::Primitive(p) if Layout::is_float(*p) => {
                let widened = if matches!(p, PrimitiveKind::F32) { self.m.float_ext(val, self.m.f64_type(), "widen") } else { val };
                self.m.call(self.runtime.f64_to_string, &[widened], "s").expect("nether_rt_f64_to_string returns a value")
            }
            Type::Primitive(PrimitiveKind::Bool) => {
                // `val` is Nether's own `i1` bool; `nether_rt_bool_to_string`
                // takes `i8` (see `runtime.rs`'s module docs).
                let widened = self.m.int_cast(val, self.m.int_type(8), false, "widen");
                self.m.call(self.runtime.bool_to_string, &[widened], "s").expect("nether_rt_bool_to_string returns a value")
            }
            Type::Primitive(PrimitiveKind::Char) => {
                self.m.call(self.runtime.char_to_string, &[val], "s").expect("nether_rt_char_to_string returns a value")
            }
            Type::Primitive(p) => {
                let widened = self.m.int_cast(val, self.m.int_type(64), Layout::is_signed(*p), "widen");
                self.m.call(self.runtime.i64_to_string, &[widened], "s").expect("nether_rt_i64_to_string returns a value")
            }
            Type::String => val,
            other => panic!("nether_codegen: ToString isn't implemented for {other:?} yet"),
        }
    }

    // ---- Terminators ---------------------------------------------------

    fn gen_terminator(&mut self, term: &Terminator) {
        match term {
            Terminator::Goto(target) => self.m.br(self.blocks[target]),
            Terminator::Branch { cond, then_block, else_block } => {
                let cond_val = self.gen_operand(cond);
                self.m.cond_br(cond_val, self.blocks[then_block], self.blocks[else_block]);
            }
            Terminator::Return(op) => {
                let ret_ty = self.mir_fn.ret.clone();
                if matches!(&ret_ty, Type::Tuple(elems) if elems.is_empty()) {
                    self.gen_operand(op);
                    self.m.ret(None);
                } else if is_aggregate(&ret_ty, self.defs()) {
                    // `gen_operand` gives an aggregate's *address*
                    // (this crate's uniform representation — see module
                    // docs), but a `ret` instruction returns aggregates
                    // by value (LLVM's own ABI lowering handles the
                    // actual copy) — load the whole aggregate once here,
                    // at the one boundary that needs it.
                    let addr = self.gen_operand(op);
                    let val = self.m.load(self.layout.llvm_type(&ret_ty), addr, "ret_val");
                    self.m.ret(Some(val));
                } else {
                    let val = self.gen_operand(op);
                    self.m.ret(Some(val));
                }
            }
            Terminator::Unreachable => self.m.unreachable(),
        }
    }
}

fn block_num(id: nether_mir::BlockId) -> String {
    format!("{id:?}")
}
