use super::*;

impl<'ctx> FnCodegen<'_, 'ctx> {
    pub(super) fn gen_clone_to_unique(&mut self, source: &Operand, dest_ty: &Type) -> Value<'ctx> {
        let Type::Unique(inner) = dest_ty else {
            panic!("CloneToUnique requires a unique destination, found {dest_ty:?}");
        };
        let source_ty = (**inner).clone();
        let source = self.gen_operand(source);
        self.gen_structural_clone_to_unique(source, &source_ty)
    }

    /// Clones one heap struct payload. The byte copy is followed by new
    /// ARC/weak credits, then every direct unique heap field is overwritten
    /// with its own recursively cloned allocation.
    fn gen_structural_clone_to_unique(
        &mut self,
        source: Value<'ctx>,
        source_ty: &Type,
    ) -> Value<'ctx> {
        let sl = self.layout.struct_layout(source_ty);
        let size = self.m.size_of(sl.ty.into());
        let drop_fn = self.func_ptr_or_null(self.shims.own_drop_shim(
            self.m,
            self.layout,
            self.runtime,
            source_ty,
        ));
        let clone = self
            .m
            .call(self.runtime.unique_alloc, &[size, drop_fn], "unique_clone")
            .expect("nether_rt_unique_alloc returns a value");
        self.m.memcpy(clone, source, size);
        if let Some(retain_fields) =
            self.shims
                .own_retain_shim(self.m, self.layout, self.runtime, source_ty)
        {
            self.m.call(retain_fields, &[clone], "");
        }
        let field_tys = self.layout.sigs.type_fields(source_ty).unwrap_or_default();
        for (index, field_ty) in field_tys.iter().enumerate() {
            let Type::Unique(inner) = field_ty else {
                continue;
            };
            if alloc_kind(inner, self.defs()) != AllocKind::Heap {
                continue;
            }
            let source_field =
                self.m
                    .struct_gep(sl.ty, source, index as u32, "clone_source_unique");
            let source_value =
                self.m
                    .load(self.m.ptr_type(), source_field, "clone_source_unique_value");
            let nested = self.gen_structural_clone_to_unique(source_value, inner);
            let dest_field = self
                .m
                .struct_gep(sl.ty, clone, index as u32, "clone_dest_unique");
            self.m.store(dest_field, nested);
        }
        clone
    }

    pub(super) fn gen_construct(
        &mut self,
        _id: nether_resolver::DefId,
        fields: &[Operand],
        dest_ty: &Type,
    ) -> Value<'ctx> {
        let sl = self.layout.struct_layout(dest_ty);
        let field_vals: Vec<Value<'ctx>> = fields.iter().map(|f| self.gen_operand(f)).collect();
        let field_tys: Vec<Type> = self.layout.sigs.type_fields(dest_ty).unwrap_or_default();

        let heap = alloc_kind(dest_ty, self.defs()) == AllocKind::Heap;
        let addr = if heap {
            let size = self.m.size_of(sl.ty.into());
            let drop_fn = self.func_ptr_or_null(self.shims.own_drop_shim(
                self.m,
                self.layout,
                self.runtime,
                dest_ty,
            ));
            let alloc = if matches!(dest_ty, Type::Unique(_)) {
                self.runtime.unique_alloc
            } else {
                self.runtime.alloc
            };
            self.m
                .call(alloc, &[size, drop_fn], "obj")
                .expect("nether_rt_arc_alloc returns a value")
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

    pub(super) fn gen_construct_variant(
        &mut self,
        enum_id: nether_resolver::DefId,
        variant: u32,
        payload: &[Operand],
        dest_ty: &Type,
    ) -> Value<'ctx> {
        debug_assert!(matches!(dest_ty, Type::Enum(id, _) if *id == enum_id));
        let el = self.layout.enum_layout(dest_ty);
        let payload_tys = self
            .layout
            .sigs
            .enum_payload(dest_ty, variant)
            .unwrap_or_default();
        let payload_vals: Vec<Value<'ctx>> = payload.iter().map(|p| self.gen_operand(p)).collect();

        let _ = dest_ty;
        let slot = self.m.alloca(el.ty.into(), "variant");
        let tag_ptr = self.m.struct_gep(el.ty, slot, 0, "tag_ptr");
        self.m.store(
            tag_ptr,
            self.m
                .const_int(self.m.int_type(64), u64::from(variant), false),
        );
        for (i, val) in payload_vals.into_iter().enumerate() {
            let gep_index = *el
                .field_offsets
                .get(&(variant, i as u32))
                .expect("valid variant/field index");
            let field_ptr = self.m.struct_gep(el.ty, slot, gep_index, "payload_field");
            let field_ty = payload_tys.get(i).cloned().unwrap_or(Type::Error);
            self.store_at(field_ptr, &field_ty, val);
        }
        slot
    }

    pub(super) fn gen_tuple(&mut self, items: &[Operand], dest_ty: &Type) -> Value<'ctx> {
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

    pub(super) fn gen_array_literal(&mut self, items: &[Operand], dest_ty: &Type) -> Value<'ctx> {
        let elem_ty = match dest_ty {
            Type::Array(t) => (**t).clone(),
            other => panic!("Rvalue::Array with non-array destination type {other:?}"),
        };
        let elem_llvm_ty = self.layout.llvm_type(&elem_ty);
        let elem_size = self.m.size_of(elem_llvm_ty);
        let cap = self
            .m
            .const_int(self.m.int_type(64), items.len() as u64, false);
        let elem_retain = self.func_ptr_or_null(self.shims.retain_shim(
            self.m,
            self.layout,
            self.runtime,
            &elem_ty,
        ));
        let elem_drop = self.func_ptr_or_null(self.shims.drop_shim(
            self.m,
            self.layout,
            self.runtime,
            &elem_ty,
        ));
        let arr = self
            .m
            .call(
                self.runtime.array_new,
                &[elem_size, cap, elem_retain, elem_drop],
                "arr",
            )
            .expect("nether_rt_array_new returns a value");
        let elem_slot = self.m.alloca(elem_llvm_ty, "elem");
        for item in items {
            let val = self.gen_operand(item);
            self.store_at(elem_slot, &elem_ty, val);
            self.m.call(self.runtime.array_push, &[arr, elem_slot], "");
        }
        arr
    }

    pub(super) fn gen_concat(&mut self, items: &[Operand]) -> Value<'ctx> {
        let mut iter = items.iter();
        let first = iter
            .next()
            .expect("Concat always has at least one piece (nether_hir's own desugaring)");
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

    pub(super) fn gen_to_string(&mut self, op: &Operand) -> Value<'ctx> {
        let ty = self.operand_ty(op);
        let val = self.gen_operand(op);
        match &ty {
            Type::Primitive(p) if Layout::is_float(*p) => {
                let widened = if matches!(p, PrimitiveKind::F32) {
                    self.m.float_ext(val, self.m.f64_type(), "widen")
                } else {
                    val
                };
                self.m
                    .call(self.runtime.f64_to_string, &[widened], "s")
                    .expect("nether_rt_f64_to_string returns a value")
            }
            Type::Primitive(PrimitiveKind::Bool) => {
                // `val` is Nether's own `i1` bool; `nether_rt_bool_to_string`
                // takes `i8` (see `runtime.rs`'s module docs).
                let widened = self.m.int_cast(val, self.m.int_type(8), false, "widen");
                self.m
                    .call(self.runtime.bool_to_string, &[widened], "s")
                    .expect("nether_rt_bool_to_string returns a value")
            }
            Type::Primitive(PrimitiveKind::Char) => self
                .m
                .call(self.runtime.char_to_string, &[val], "s")
                .expect("nether_rt_char_to_string returns a value"),
            Type::Primitive(p) => {
                let widened =
                    self.m
                        .int_cast(val, self.m.int_type(64), Layout::is_signed(*p), "widen");
                self.m
                    .call(self.runtime.i64_to_string, &[widened], "s")
                    .expect("nether_rt_i64_to_string returns a value")
            }
            Type::String => val,
            other => panic!("nether_codegen: ToString isn't implemented for {other:?} yet"),
        }
    }
}
