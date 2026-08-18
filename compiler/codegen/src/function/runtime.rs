use super::*;

impl<'ctx> FnCodegen<'_, 'ctx> {
    pub(super) fn gen_completed_task(&self, output: Value<'ctx>, output_ty: &Type) -> Value<'ctx> {
        let word = self.m.int_type(64);
        let output_size = self.m.size_of(self.layout.llvm_type(output_ty));
        let size = self
            .m
            .int_add(output_size, self.m.const_int(word, 16, false), "task_size");
        let output_drop = self.func_ptr_or_null(self.shims.drop_shim(
            self.m,
            self.layout,
            self.runtime,
            output_ty,
        ));
        let task_drop = self
            .runtime
            .task_drop
            .as_global_value()
            .as_pointer_value()
            .into();
        let task = self
            .m
            .call(self.runtime.alloc, &[size, task_drop], "completed_task")
            .expect("task allocation returns a payload pointer");
        let poll = self
            .runtime
            .task_completed_poll
            .as_global_value()
            .as_pointer_value()
            .into();
        self.m.store(task, poll);
        let drop_slot =
            self.m
                .gep_bytes(task, self.m.const_int(word, 8, false), "task_output_drop");
        self.m.store(drop_slot, output_drop);
        let output_slot = self
            .m
            .gep_bytes(task, self.m.const_int(word, 16, false), "task_output");
        self.store_at(output_slot, output_ty, output);
        task
    }

    pub(super) fn gen_await(&self, task: &Operand, output_ty: &Type) -> Value<'ctx> {
        let task = self.gen_operand(task);
        self.m.call(self.runtime.task_block_on, &[task], "");
        let output = self.m.gep_bytes(
            task,
            self.m.const_int(self.m.int_type(64), 16, false),
            "task_output",
        );
        self.load_value(output, output_ty)
    }

    pub(super) fn gen_pack_existential(&self, methods: &[Operand]) -> Value<'ctx> {
        let word = self.m.int_type(64);
        let size = self
            .m
            .const_int(word, ((methods.len() + 1) * 8) as u64, false);
        let drop = self
            .runtime
            .existential_drop
            .as_global_value()
            .as_pointer_value()
            .into();
        let package = self
            .m
            .call(self.runtime.alloc, &[size, drop], "existential")
            .expect("nether_rt_arc_alloc returns an existential package");
        self.m
            .store(package, self.m.const_int(word, methods.len() as u64, false));
        for (index, method) in methods.iter().enumerate() {
            let offset = self.m.const_int(word, ((index + 1) * 8) as u64, false);
            let slot = self.m.gep_bytes(package, offset, "witness_slot");
            let closure = self.gen_operand(method);
            // `methods` are ordinary MIR locals and are released at the end
            // of the surrounding scope.  The existential package therefore
            // needs its own strong reference for every stored witness closure.
            self.m.call(self.runtime.retain, &[closure], "");
            self.m.store(slot, closure);
        }
        package
    }

    pub(super) fn gen_call_witness(
        &self,
        receiver: &Operand,
        slot: u32,
        function_ty: &Type,
        args: &[Operand],
        dest_ty: &Type,
    ) -> Value<'ctx> {
        let package = self.gen_operand(receiver);
        let offset = self
            .m
            .const_int(self.m.int_type(64), ((slot as usize + 1) * 8) as u64, false);
        let closure_slot = self.m.gep_bytes(package, offset, "witness_slot");
        let closure = self
            .m
            .load(self.m.ptr_type(), closure_slot, "witness_closure");
        let code = self.m.load(self.m.ptr_type(), closure, "witness_fn");
        let Type::Function(params, ret) = function_ty else {
            panic!("witness slot does not carry a function type")
        };
        let mut param_tys = vec![self.m.ptr_type()];
        param_tys.extend(params.iter().map(|ty| {
            if is_aggregate(ty, self.defs()) {
                self.m.ptr_type()
            } else {
                self.layout.llvm_type(ty)
            }
        }));
        let ret_ty = if matches!(ret.as_ref(), Type::Tuple(items) if items.is_empty()) {
            None
        } else {
            Some(self.layout.llvm_type(ret))
        };
        let fn_ty = self.m.fn_type(&param_tys, ret_ty);
        let mut values = Vec::with_capacity(args.len() + 1);
        values.push(closure);
        values.extend(args.iter().map(|argument| self.gen_operand(argument)));
        let result = self
            .m
            .indirect_call(fn_ty, code, &values, "witness_call")
            .unwrap_or_else(|| self.gen_unit());
        if is_aggregate(dest_ty, self.defs()) {
            let spill = self
                .m
                .alloca(self.layout.llvm_type(dest_ty), "witness_result");
            self.m.store(spill, result);
            spill
        } else {
            result
        }
    }

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
            "__task_spawn" => {
                let task = self.gen_operand(&args[0]);
                self.m
                    .call(self.runtime.task_spawn, &[task], "spawned_task")
                    .expect("task spawn returns a task")
            }
            "__timer_sleep" => {
                let millis = self.gen_operand(&args[0]);
                let millis = self
                    .m
                    .int_cast(millis, self.m.int_type(64), false, "sleep_millis");
                self.m
                    .call(self.runtime.timer_sleep, &[millis], "timer_task")
                    .expect("timer sleep returns a task")
            }
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
        let base_ty = self.operand_ty(base);
        let base_val = match (&base_ty, base) {
            (Type::FixedArray(_, _), Operand::Local(local)) => self.base_address(*local, &base_ty),
            _ => self.gen_operand(base),
        };
        let idx_val = self.gen_array_index(index);
        let addr = match base_ty {
            Type::FixedArray(element, _) => {
                let size = self.m.size_of(self.layout.llvm_type(&element));
                let offset = self.m.int_mul(idx_val, size, "fixed_index_offset");
                self.m.gep_bytes(base_val, offset, "fixed_elem_ptr")
            }
            _ => self
                .m
                .call(self.runtime.array_get, &[base_val, idx_val], "elem_ptr")
                .expect("nether_rt_array_get returns a value"),
        };
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
