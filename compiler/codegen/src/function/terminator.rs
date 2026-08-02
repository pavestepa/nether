use super::*;

impl FnCodegen<'_, '_> {
    pub(super) fn gen_terminator(&mut self, term: &Terminator) {
        match term {
            Terminator::Goto(target) => self.m.br(self.blocks[target]),
            Terminator::Branch {
                cond,
                then_block,
                else_block,
            } => {
                let cond_val = self.gen_operand(cond);
                self.m
                    .cond_br(cond_val, self.blocks[then_block], self.blocks[else_block]);
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
