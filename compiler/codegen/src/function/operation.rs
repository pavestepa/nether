use super::*;

impl<'ctx> FnCodegen<'_, 'ctx> {
    pub(super) fn gen_rvalue(&mut self, rvalue: &Rvalue, dest_ty: &Type) -> Value<'ctx> {
        match rvalue {
            Rvalue::Use(op) => self.gen_operand(op),
            Rvalue::Await(task) => self.gen_await(task, dest_ty),
            Rvalue::AddressOf(place) => self.address_of_place(place),
            Rvalue::Deref(reference) => {
                let address = self.gen_operand(reference);
                self.load_value(address, dest_ty)
            }
            Rvalue::PromoteUnique(value) => self
                .m
                .call(
                    self.runtime.unique_promote,
                    &[self.gen_operand(value)],
                    "promoted",
                )
                .expect("nether_rt_unique_promote returns a pointer"),
            Rvalue::CloneToUnique(value) => self.gen_clone_to_unique(value, dest_ty),
            Rvalue::Hash(value) => self.gen_hash(value),
            Rvalue::Unary(op, a) => self.gen_unary(*op, a),
            Rvalue::Binary(op, a, b) => self.gen_binary(*op, a, b),
            Rvalue::Call { target, args } => self.gen_call(target, args, dest_ty),
            Rvalue::CallBuiltin { name, args } => self.gen_call_builtin(name, args, dest_ty),
            Rvalue::CallArrayMethod {
                receiver,
                method,
                args,
            } => self.gen_call_array_method(receiver, method, args, dest_ty),
            Rvalue::PackExistential { methods } => self.gen_pack_existential(methods),
            Rvalue::CallWitness {
                receiver,
                slot,
                function_ty,
                args,
            } => self.gen_call_witness(receiver, *slot, function_ty, args, dest_ty),
            Rvalue::Field { base, index } => self.gen_field_read(base, *index, dest_ty),
            Rvalue::VariantField {
                base,
                variant,
                index,
            } => self.gen_variant_field_read(base, *variant, *index, dest_ty),
            Rvalue::Discriminant(base) => self.gen_discriminant(base),
            Rvalue::Index { base, index } => self.gen_index_read(base, index, dest_ty),
            Rvalue::Construct { ty, fields } => self.gen_construct(*ty, fields, dest_ty),
            Rvalue::ConstructVariant {
                enum_id,
                variant,
                payload,
            } => self.gen_construct_variant(*enum_id, *variant, payload, dest_ty),
            Rvalue::Tuple(items) => self.gen_tuple(items, dest_ty),
            Rvalue::Array(items) => self.gen_array_literal(items, dest_ty),
            Rvalue::Concat(items) => self.gen_concat(items),
            Rvalue::ToString(op) => self.gen_to_string(op),
            Rvalue::Closure { function, captures } => self.gen_closure(*function, captures),
        }
    }

    pub(super) fn gen_unary(&self, op: UnaryOp, a: &Operand) -> Value<'ctx> {
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

    pub(super) fn gen_binary(&self, op: BinaryOp, a: &Operand, b: &Operand) -> Value<'ctx> {
        let ty = self.operand_ty(a);
        let av = self.gen_operand(a);
        let bv = self.gen_operand(b);
        if matches!(op, BinaryOp::Eq | BinaryOp::Ne)
            && self.layout.sigs.can_derive_eq(&ty, self.defs())
        {
            let structural_ty = match &ty {
                Type::Unique(inner) if alloc_kind(inner, self.defs()) == AllocKind::Heap => {
                    inner.as_ref()
                }
                _ => &ty,
            };
            let equal = self.gen_structural_eq(av, bv, structural_ty);
            return if op == BinaryOp::Ne {
                self.m.int_not(equal, "ne")
            } else {
                equal
            };
        }
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
                BinaryOp::And | BinaryOp::Or => {
                    unreachable!("`&&`/`||` never operate on float operands (typecheck)")
                }
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
            BinaryOp::Lt => self.m.int_compare(
                if signed {
                    IntPredicate::SLT
                } else {
                    IntPredicate::ULT
                },
                av,
                bv,
                "lt",
            ),
            BinaryOp::Le => self.m.int_compare(
                if signed {
                    IntPredicate::SLE
                } else {
                    IntPredicate::ULE
                },
                av,
                bv,
                "le",
            ),
            BinaryOp::Gt => self.m.int_compare(
                if signed {
                    IntPredicate::SGT
                } else {
                    IntPredicate::UGT
                },
                av,
                bv,
                "gt",
            ),
            BinaryOp::Ge => self.m.int_compare(
                if signed {
                    IntPredicate::SGE
                } else {
                    IntPredicate::UGE
                },
                av,
                bv,
                "ge",
            ),
            BinaryOp::And => self.m.int_and(av, bv, "and"),
            BinaryOp::Or => self.m.int_or(av, bv, "or"),
        }
    }

    /// Compares two addresses containing `ty`. Heap struct operands already
    /// are payload addresses; nested heap/unique fields are loaded once to
    /// obtain their payload addresses before recursion.
    fn gen_structural_eq(&self, left: Value<'ctx>, right: Value<'ctx>, ty: &Type) -> Value<'ctx> {
        match ty {
            Type::Primitive(primitive) => {
                let llvm_ty = self.layout.llvm_type(ty);
                let left = self.m.load(llvm_ty, left, "eq_left");
                let right = self.m.load(llvm_ty, right, "eq_right");
                if Layout::is_float(*primitive) {
                    self.m
                        .float_compare(FloatPredicate::OEQ, left, right, "eq_field")
                } else {
                    self.m
                        .int_compare(IntPredicate::EQ, left, right, "eq_field")
                }
            }
            Type::Unique(inner) => {
                if alloc_kind(inner, self.defs()) == AllocKind::Heap {
                    let left = self.m.load(self.m.ptr_type(), left, "eq_unique_left");
                    let right = self.m.load(self.m.ptr_type(), right, "eq_unique_right");
                    self.gen_structural_eq(left, right, inner)
                } else {
                    self.gen_structural_eq(left, right, inner)
                }
            }
            Type::Struct(_, _) | Type::TupleStruct(_, _) => {
                let layout = self.layout.struct_layout(ty);
                let fields = self.layout.sigs.type_fields(ty).unwrap_or_default();
                fields.iter().enumerate().fold(
                    self.m.const_bool(true),
                    |equal, (index, field_ty)| {
                        let left_field =
                            self.m
                                .struct_gep(layout.ty, left, index as u32, "eq_left_field");
                        let right_field =
                            self.m
                                .struct_gep(layout.ty, right, index as u32, "eq_right_field");
                        let field_equal = if alloc_kind(field_ty, self.defs()) == AllocKind::Heap
                            && !matches!(field_ty, Type::Unique(_))
                        {
                            let left_value =
                                self.m.load(self.m.ptr_type(), left_field, "eq_left_object");
                            let right_value =
                                self.m
                                    .load(self.m.ptr_type(), right_field, "eq_right_object");
                            self.gen_structural_eq(left_value, right_value, field_ty)
                        } else {
                            self.gen_structural_eq(left_field, right_field, field_ty)
                        };
                        self.m.int_and(equal, field_equal, "eq_fields")
                    },
                )
            }
            Type::Tuple(items) => {
                let tuple_ty = match self.layout.llvm_type(ty) {
                    nether_llvm::Ty::StructType(tuple_ty) => tuple_ty,
                    _ => unreachable!("tuple layout must be a struct"),
                };
                items
                    .iter()
                    .enumerate()
                    .fold(self.m.const_bool(true), |equal, (index, item_ty)| {
                        let left_item =
                            self.m
                                .struct_gep(tuple_ty, left, index as u32, "eq_left_item");
                        let right_item =
                            self.m
                                .struct_gep(tuple_ty, right, index as u32, "eq_right_item");
                        let item_equal = if alloc_kind(item_ty, self.defs()) == AllocKind::Heap
                            && !matches!(item_ty, Type::Unique(_))
                        {
                            let left_value =
                                self.m.load(self.m.ptr_type(), left_item, "eq_left_object");
                            let right_value =
                                self.m
                                    .load(self.m.ptr_type(), right_item, "eq_right_object");
                            self.gen_structural_eq(left_value, right_value, item_ty)
                        } else {
                            self.gen_structural_eq(left_item, right_item, item_ty)
                        };
                        self.m.int_and(equal, item_equal, "eq_items")
                    })
            }
            other => unreachable!("non-structural derived Eq field {other:?}"),
        }
    }

    fn gen_hash(&self, value: &Operand) -> Value<'ctx> {
        let ty = self.operand_ty(value);
        let structural_ty = match &ty {
            Type::Unique(inner) if alloc_kind(inner, self.defs()) == AllocKind::Heap => {
                inner.as_ref()
            }
            _ => &ty,
        };
        let address = self.gen_operand(value);
        self.gen_structural_hash(address, structural_ty)
    }

    /// Hashes an address containing `ty` with a stable FNV-1a-style fold.
    /// This is intentionally an unkeyed language hash, not a security hash.
    fn gen_structural_hash(&self, address: Value<'ctx>, ty: &Type) -> Value<'ctx> {
        let u64_ty = self.m.int_type(64);
        match ty {
            Type::String => self
                .m
                .call(self.runtime.string_hash, &[address], "hash_string")
                .expect("nether_rt_string_hash returns u64"),
            Type::Primitive(primitive) => {
                let value = self
                    .m
                    .load(self.layout.llvm_type(ty), address, "hash_field");
                self.m
                    .int_cast(value, u64_ty, Layout::is_signed(*primitive), "hash_bits")
            }
            Type::Unique(inner) => {
                if alloc_kind(inner, self.defs()) == AllocKind::Heap {
                    let value = self.m.load(self.m.ptr_type(), address, "hash_unique");
                    self.gen_structural_hash(value, inner)
                } else {
                    self.gen_structural_hash(address, inner)
                }
            }
            Type::Struct(_, _) | Type::TupleStruct(_, _) => {
                let layout = self.layout.struct_layout(ty);
                let fields = self.layout.sigs.type_fields(ty).unwrap_or_default();
                fields.iter().enumerate().fold(
                    self.m.const_int(u64_ty, 1_469_598_103_934_665_603, false),
                    |hash, (index, field_ty)| {
                        let field =
                            self.m
                                .struct_gep(layout.ty, address, index as u32, "hash_field_ptr");
                        let field_hash = if alloc_kind(field_ty, self.defs()) == AllocKind::Heap
                            && !matches!(field_ty, Type::Unique(_))
                        {
                            let value = self.m.load(self.m.ptr_type(), field, "hash_object");
                            self.gen_structural_hash(value, field_ty)
                        } else {
                            self.gen_structural_hash(field, field_ty)
                        };
                        let mixed = self.m.int_xor(hash, field_hash, "hash_xor");
                        self.m.int_mul(
                            mixed,
                            self.m.const_int(u64_ty, 1_099_511_628_211, false),
                            "hash_mul",
                        )
                    },
                )
            }
            Type::Tuple(items) => {
                let tuple_ty = match self.layout.llvm_type(ty) {
                    nether_llvm::Ty::StructType(tuple_ty) => tuple_ty,
                    _ => unreachable!("tuple layout must be a struct"),
                };
                items.iter().enumerate().fold(
                    self.m.const_int(u64_ty, 1_469_598_103_934_665_603, false),
                    |hash, (index, item_ty)| {
                        let item =
                            self.m
                                .struct_gep(tuple_ty, address, index as u32, "hash_item_ptr");
                        let item_hash = if alloc_kind(item_ty, self.defs()) == AllocKind::Heap
                            && !matches!(item_ty, Type::Unique(_))
                        {
                            let value = self.m.load(self.m.ptr_type(), item, "hash_object");
                            self.gen_structural_hash(value, item_ty)
                        } else {
                            self.gen_structural_hash(item, item_ty)
                        };
                        let mixed = self.m.int_xor(hash, item_hash, "hash_xor");
                        self.m.int_mul(
                            mixed,
                            self.m.const_int(u64_ty, 1_099_511_628_211, false),
                            "hash_mul",
                        )
                    },
                )
            }
            other => unreachable!("non-structural derived Hash field {other:?}"),
        }
    }

    pub(super) fn gen_call(
        &mut self,
        target: &CallTarget,
        args: &[Operand],
        dest_ty: &Type,
    ) -> Value<'ctx> {
        let result = match target {
            CallTarget::Fn(id) => {
                let f = *self.funcs.get(id).expect(
                    "every CallTarget::Fn refers to a function declared in this same module",
                );
                let mir_target = self
                    .mir_functions
                    .get(id)
                    .expect("every CallTarget::Fn refers to MIR in this same module");
                let arg_vals: Vec<Value<'ctx>> = args
                    .iter()
                    .zip(&mir_target.params)
                    .map(|(arg, &param)| {
                        if mir_target.local_decl(param).mutable
                            && !matches!(
                                mir_target.local_decl(param).ty,
                                Type::Ref(_) | Type::MutRef(_)
                            )
                        {
                            let Operand::Local(local) = arg else {
                                panic!("a mutable parameter requires a local argument");
                            };
                            self.locals[local.index()]
                        } else {
                            self.gen_operand(arg)
                        }
                    })
                    .collect();
                let result = self
                    .m
                    .call(f, &arg_vals, "call")
                    .unwrap_or_else(|| self.gen_unit());
                if mir_target.is_async {
                    let output = if is_aggregate(&mir_target.ret, self.defs()) {
                        let slot = self
                            .m
                            .alloca(self.layout.llvm_type(&mir_target.ret), "async_output");
                        self.m.store(slot, result);
                        slot
                    } else {
                        result
                    };
                    return self.gen_completed_task(output, &mir_target.ret);
                }
                result
            }
            CallTarget::Dynamic(callee) => {
                let arg_vals: Vec<Value<'ctx>> = args.iter().map(|a| self.gen_operand(a)).collect();
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
                self.m
                    .indirect_call(fn_ty, code, &dynamic_args, "closure_call")
                    .unwrap_or_else(|| self.gen_unit())
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
}
