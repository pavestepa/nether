use super::*;

impl<'ctx> FnCodegen<'_, 'ctx> {
    pub(super) fn gen_rvalue(&mut self, rvalue: &Rvalue, dest_ty: &Type) -> Value<'ctx> {
        match rvalue {
            Rvalue::Use(op) => self.gen_operand(op),
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
            Rvalue::Unary(op, a) => self.gen_unary(*op, a),
            Rvalue::Binary(op, a, b) => self.gen_binary(*op, a, b),
            Rvalue::Call { target, args } => self.gen_call(target, args, dest_ty),
            Rvalue::CallBuiltin { name, args } => self.gen_call_builtin(name, args, dest_ty),
            Rvalue::CallArrayMethod {
                receiver,
                method,
                args,
            } => self.gen_call_array_method(receiver, method, args, dest_ty),
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
                self.m
                    .call(f, &arg_vals, "call")
                    .unwrap_or_else(|| self.gen_unit())
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
