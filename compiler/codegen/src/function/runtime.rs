use super::*;

impl<'ctx> FnCodegen<'_, 'ctx> {
    pub(super) fn gen_closure(&self, function: MonoFnId, captures: &[Operand]) -> Value<'ctx> {
        let capture_tys: Vec<Type> = captures
            .iter()
            .map(|capture| self.operand_ty(capture))
            .collect();
        let mut fields = vec![self.m.ptr_type()];
        fields.extend(capture_tys.iter().map(|ty| self.layout.llvm_type(ty)));
        let env_ty = self.m.struct_type(&fields);
        let size = self.m.size_of(env_ty.into());
        let drop_fn = self.func_ptr_or_null(self.shims.closure_drop_shim(
            self.m,
            self.layout,
            self.runtime,
            &capture_tys,
        ));
        let env = self
            .m
            .call(self.runtime.alloc, &[size, drop_fn], "closure")
            .expect("nether_rt_arc_alloc returns a closure environment");
        let code_field = self.m.struct_gep(env_ty, env, 0, "code");
        let code = self.funcs[&function]
            .as_global_value()
            .as_pointer_value()
            .into();
        self.m.store(code_field, code);
        for (index, (capture, ty)) in captures.iter().zip(capture_tys.iter()).enumerate() {
            let field = self.m.struct_gep(env_ty, env, index as u32 + 1, "capture");
            self.store_at(field, ty, self.gen_operand(capture));
        }
        env
    }

    pub(super) fn gen_call_builtin(
        &mut self,
        name: &Symbol,
        args: &[Operand],
        dest_ty: &Type,
    ) -> Value<'ctx> {
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
    pub(super) fn gen_weak_upgrade(&self, option_ty: &Type, weak_ptr: Value<'ctx>) -> Value<'ctx> {
        let elem_ty = match option_ty {
            Type::Enum(_, args) => args.first().cloned().unwrap_or(Type::Error),
            other => panic!("a weak read's result must be an Option, found {other:?}"),
        };
        let el = self.layout.enum_layout(option_ty);
        let out_slot = self.m.alloca(self.m.ptr_type(), "upgraded_ptr");
        // `nether_rt_arc_weak_upgrade` returns `i8` (0/1), not LLVM's
        // native `i1` — see `runtime.rs`'s module docs; `select` needs an
        // actual `i1` condition.
        let has_value_i8 = self
            .m
            .call(
                self.runtime.weak_upgrade,
                &[weak_ptr, out_slot],
                "has_value",
            )
            .expect("nether_rt_arc_weak_upgrade returns a value");
        let has_value = self
            .m
            .int_cast(has_value_i8, self.m.bool_type(), false, "has_value");

        let result_slot = self.m.alloca(el.ty.into(), "upgrade_result");
        let tag_ptr = self.m.struct_gep(el.ty, result_slot, 0, "tag_ptr");
        // `Option`'s builtin variant order is `Some = 0, None = 1` (see
        // `nether_resolver::def::builtin_definitions`).
        let some_tag = self.m.const_int(self.m.int_type(64), 0, false);
        let none_tag = self.m.const_int(self.m.int_type(64), 1, false);
        let tag = self.m.select(has_value, some_tag, none_tag, "tag");
        self.m.store(tag_ptr, tag);

        if let Some(&payload_index) = el.field_offsets.get(&(0, 0)) {
            let payload_ptr = self
                .m
                .struct_gep(el.ty, result_slot, payload_index, "payload_ptr");
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

    pub(super) fn gen_call_array_method(
        &mut self,
        receiver: &Operand,
        method: &Symbol,
        args: &[Operand],
        dest_ty: &Type,
    ) -> Value<'ctx> {
        let recv = self.gen_operand(receiver);
        match method.as_str() {
            "len" => self
                .m
                .call(self.runtime.array_len, &[recv], "len")
                .expect("nether_rt_array_len returns a value"),
            "push" => {
                let elem_ty = self.operand_ty(&args[0]);
                let elem_val = self.gen_operand(&args[0]);
                let elem_slot = self.m.alloca(self.layout.llvm_type(&elem_ty), "push_elem");
                self.store_at(elem_slot, &elem_ty, elem_val);
                self.m
                    .call(self.runtime.array_push, &[recv, elem_slot], "")
                    .unwrap_or_else(|| self.gen_unit())
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

    pub(super) fn gen_array_pop(&self, option_ty: &Type, recv: Value<'ctx>) -> Value<'ctx> {
        let elem_ty = match option_ty {
            Type::Enum(_, args) => args.first().cloned().unwrap_or(Type::Error),
            other => panic!("Array::pop's result must be an Option, found {other:?}"),
        };
        let el = self.layout.enum_layout(option_ty);
        let out_slot = self.m.alloca(self.layout.llvm_type(&elem_ty), "pop_elem");
        // `nether_rt_array_pop` returns `i8` (0/1), not LLVM's `i1` — see
        // `runtime.rs`'s module docs on why `bool` crosses this ABI
        // boundary as a byte; `select` needs an actual `i1` condition.
        let has_value_i8 = self
            .m
            .call(self.runtime.array_pop, &[recv, out_slot], "has_value")
            .expect("nether_rt_array_pop returns a value");
        let has_value = self
            .m
            .int_cast(has_value_i8, self.m.bool_type(), false, "has_value");

        let result_slot = self.m.alloca(el.ty.into(), "pop_result");
        let tag_ptr = self.m.struct_gep(el.ty, result_slot, 0, "tag_ptr");
        // `Option`'s builtin variant order is `Some = 0, None = 1` (see
        // `nether_resolver::def::builtin_definitions`).
        let some_tag = self.m.const_int(self.m.int_type(64), 0, false);
        let none_tag = self.m.const_int(self.m.int_type(64), 1, false);
        let tag = self.m.select(has_value, some_tag, none_tag, "tag");
        self.m.store(tag_ptr, tag);

        if let Some(&payload_index) = el.field_offsets.get(&(0, 0)) {
            let payload_ptr = self
                .m
                .struct_gep(el.ty, result_slot, payload_index, "payload_ptr");
            let elem_val = self
                .m
                .load(self.layout.llvm_type(&elem_ty), out_slot, "elem");
            self.store_at(payload_ptr, &elem_ty, elem_val);
        }
        result_slot
    }

    pub(super) fn gen_field_read(&self, base: &Operand, index: u32, dest_ty: &Type) -> Value<'ctx> {
        let base_ty = self.operand_ty(base);
        let base_val = self.gen_operand(base);
        let addr = self.struct_field_address(base_ty.strip_indirection(), base_val, index);
        self.load_value(addr, dest_ty)
    }

    pub(super) fn gen_variant_field_read(
        &self,
        base: &Operand,
        variant: u32,
        index: u32,
        dest_ty: &Type,
    ) -> Value<'ctx> {
        let base_ty = self.operand_ty(base);
        let base_val = self.gen_operand(base);
        let addr = self.variant_field_address(&base_ty, base_val, variant, index);
        self.load_value(addr, dest_ty)
    }

    pub(super) fn load_value(&self, addr: Value<'ctx>, ty: &Type) -> Value<'ctx> {
        if is_aggregate(ty, self.defs()) {
            addr
        } else {
            self.m.load(self.layout.llvm_type(ty), addr, "v")
        }
    }

    pub(super) fn gen_discriminant(&self, base: &Operand) -> Value<'ctx> {
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

    pub(super) fn gen_index_read(
        &self,
        base: &Operand,
        index: &Operand,
        dest_ty: &Type,
    ) -> Value<'ctx> {
        let base_val = self.gen_operand(base);
        let idx_val = self.gen_array_index(index);
        let addr = self
            .m
            .call(self.runtime.array_get, &[base_val, idx_val], "elem_ptr")
            .expect("nether_rt_array_get returns a value");
        self.load_value(addr, dest_ty)
    }

    pub(super) fn gen_array_index(&self, index: &Operand) -> Value<'ctx> {
        let value = self.gen_operand(index);
        match self.operand_ty(index) {
            Type::Primitive(PrimitiveKind::Usize) | Type::Primitive(PrimitiveKind::U64) => value,
            Type::Primitive(kind) if kind.is_integer() => self.m.int_cast(
                value,
                self.m.int_type(64),
                Layout::is_signed(kind),
                "array_index",
            ),
            _ => value,
        }
    }
}
