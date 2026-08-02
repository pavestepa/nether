use super::*;

impl<'ctx> ModuleCx<'ctx> {
    pub fn call(&self, f: Func<'ctx>, args: &[Value<'ctx>], name: &str) -> Option<Value<'ctx>> {
        let args: Vec<ArgValue<'ctx>> = args.iter().map(|v| (*v).into()).collect();
        self.builder
            .build_call(f, &args, name)
            .expect("build_call")
            .try_as_basic_value()
            .left()
    }

    pub fn indirect_call(
        &self,
        fn_ty: FnTy<'ctx>,
        callee: Value<'ctx>,
        args: &[Value<'ctx>],
        name: &str,
    ) -> Option<Value<'ctx>> {
        let args: Vec<ArgValue<'ctx>> = args.iter().map(|value| (*value).into()).collect();
        self.builder
            .build_indirect_call(fn_ty, callee.into_pointer_value(), &args, name)
            .expect("build_indirect_call")
            .try_as_basic_value()
            .left()
    }

    pub fn br(&self, target: Block<'ctx>) {
        self.builder
            .build_unconditional_branch(target)
            .expect("build_unconditional_branch");
    }

    pub fn cond_br(&self, cond: Value<'ctx>, then_block: Block<'ctx>, else_block: Block<'ctx>) {
        self.builder
            .build_conditional_branch(cond.into_int_value(), then_block, else_block)
            .expect("build_conditional_branch");
    }

    pub fn ret(&self, value: Option<Value<'ctx>>) {
        match value {
            Some(v) => self.builder.build_return(Some(&v)),
            None => self.builder.build_return(None),
        }
        .expect("build_return");
    }

    pub fn unreachable(&self) {
        self.builder.build_unreachable().expect("build_unreachable");
    }
}
