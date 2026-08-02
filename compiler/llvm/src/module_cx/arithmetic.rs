use super::*;

impl<'ctx> ModuleCx<'ctx> {
    pub fn int_add(&self, a: Value<'ctx>, b: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder
            .build_int_add(a.into_int_value(), b.into_int_value(), name)
            .expect("build_int_add")
            .into()
    }

    pub fn int_sub(&self, a: Value<'ctx>, b: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder
            .build_int_sub(a.into_int_value(), b.into_int_value(), name)
            .expect("build_int_sub")
            .into()
    }

    pub fn int_mul(&self, a: Value<'ctx>, b: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder
            .build_int_mul(a.into_int_value(), b.into_int_value(), name)
            .expect("build_int_mul")
            .into()
    }

    pub fn int_signed_div(&self, a: Value<'ctx>, b: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder
            .build_int_signed_div(a.into_int_value(), b.into_int_value(), name)
            .expect("build_int_signed_div")
            .into()
    }

    pub fn int_unsigned_div(&self, a: Value<'ctx>, b: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder
            .build_int_unsigned_div(a.into_int_value(), b.into_int_value(), name)
            .expect("build_int_unsigned_div")
            .into()
    }

    pub fn int_signed_rem(&self, a: Value<'ctx>, b: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder
            .build_int_signed_rem(a.into_int_value(), b.into_int_value(), name)
            .expect("build_int_signed_rem")
            .into()
    }

    pub fn int_unsigned_rem(&self, a: Value<'ctx>, b: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder
            .build_int_unsigned_rem(a.into_int_value(), b.into_int_value(), name)
            .expect("build_int_unsigned_rem")
            .into()
    }

    pub fn int_neg(&self, a: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder
            .build_int_neg(a.into_int_value(), name)
            .expect("build_int_neg")
            .into()
    }

    pub fn int_not(&self, a: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder
            .build_not(a.into_int_value(), name)
            .expect("build_not")
            .into()
    }

    /// Sign- or zero-extends/truncates `a` to `target: IntType` per
    /// `signed` — used to widen every integer primitive uniformly to
    /// `i64` before handing it to `runtime`'s single `nether_i64_to_string`
    /// (`ToString`'s codegen, `nether_codegen`).
    pub fn int_cast(
        &self,
        a: Value<'ctx>,
        target: Ty<'ctx>,
        signed: bool,
        name: &str,
    ) -> Value<'ctx> {
        self.builder
            .build_int_cast_sign_flag(a.into_int_value(), target.into_int_type(), signed, name)
            .expect("build_int_cast_sign_flag")
            .into()
    }

    pub fn float_ext(&self, a: Value<'ctx>, target: Ty<'ctx>, name: &str) -> Value<'ctx> {
        self.builder
            .build_float_ext(a.into_float_value(), target.into_float_type(), name)
            .expect("build_float_ext")
            .into()
    }

    pub fn select(
        &self,
        cond: Value<'ctx>,
        then_val: Value<'ctx>,
        else_val: Value<'ctx>,
        name: &str,
    ) -> Value<'ctx> {
        self.builder
            .build_select(cond.into_int_value(), then_val, else_val, name)
            .expect("build_select")
    }

    pub fn int_and(&self, a: Value<'ctx>, b: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder
            .build_and(a.into_int_value(), b.into_int_value(), name)
            .expect("build_and")
            .into()
    }

    pub fn int_or(&self, a: Value<'ctx>, b: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder
            .build_or(a.into_int_value(), b.into_int_value(), name)
            .expect("build_or")
            .into()
    }

    pub fn int_compare(
        &self,
        pred: IntPredicate,
        a: Value<'ctx>,
        b: Value<'ctx>,
        name: &str,
    ) -> Value<'ctx> {
        self.builder
            .build_int_compare(pred, a.into_int_value(), b.into_int_value(), name)
            .expect("build_int_compare")
            .into()
    }

    pub fn float_add(&self, a: Value<'ctx>, b: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder
            .build_float_add(a.into_float_value(), b.into_float_value(), name)
            .expect("build_float_add")
            .into()
    }

    pub fn float_sub(&self, a: Value<'ctx>, b: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder
            .build_float_sub(a.into_float_value(), b.into_float_value(), name)
            .expect("build_float_sub")
            .into()
    }

    pub fn float_mul(&self, a: Value<'ctx>, b: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder
            .build_float_mul(a.into_float_value(), b.into_float_value(), name)
            .expect("build_float_mul")
            .into()
    }

    pub fn float_div(&self, a: Value<'ctx>, b: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder
            .build_float_div(a.into_float_value(), b.into_float_value(), name)
            .expect("build_float_div")
            .into()
    }

    pub fn float_neg(&self, a: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder
            .build_float_neg(a.into_float_value(), name)
            .expect("build_float_neg")
            .into()
    }

    pub fn float_compare(
        &self,
        pred: FloatPredicate,
        a: Value<'ctx>,
        b: Value<'ctx>,
        name: &str,
    ) -> Value<'ctx> {
        self.builder
            .build_float_compare(pred, a.into_float_value(), b.into_float_value(), name)
            .expect("build_float_compare")
            .into()
    }
}
