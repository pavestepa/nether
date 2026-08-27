use super::*;
use crate::frame::FrameLayout;

impl<'ctx> FnCodegen<'_, 'ctx> {
    pub(super) fn gen_terminator(&mut self, block_id: nether_mir::BlockId, term: &Terminator) {
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
                if let Some(frame) = self.frame.clone() {
                    self.gen_async_return(&frame, op);
                } else {
                    self.gen_sync_return(op);
                }
            }
            Terminator::Unreachable => self.m.unreachable(),
            Terminator::Await {
                task,
                output_local,
                resume,
            } => self.gen_await_terminator(block_id, task, *output_local, *resume),
        }
    }

    fn gen_sync_return(&self, op: &Operand) {
        let ret_ty = self.mir_fn.ret.clone();
        if matches!(&ret_ty, Type::Tuple(elems) if elems.is_empty()) {
            self.gen_operand(op);
            self.m.ret(None);
        } else if is_aggregate(&ret_ty, self.defs()) {
            // `gen_operand` gives an aggregate's *address* (this crate's
            // uniform representation — see module docs), but a `ret`
            // instruction returns aggregates by value (LLVM's own ABI
            // lowering handles the actual copy) — load the whole
            // aggregate once here, at the one boundary that needs it.
            let addr = self.gen_operand(op);
            let val = self.m.load(self.layout.llvm_type(&ret_ty), addr, "ret_val");
            self.m.ret(Some(val));
        } else {
            let val = self.gen_operand(op);
            self.m.ret(Some(val));
        }
    }

    /// An `is_async` function's poll-function counterpart of
    /// [`Self::gen_sync_return`]: the MIR return value doesn't leave
    /// through an LLVM `ret` at all — it's stored into the frame's own
    /// output field (byte offset 16, matching `runtime/task`'s
    /// `TaskHeader` convention every task payload shares — see
    /// `frame`'s module docs), alongside the type-specific output-drop
    /// shim `nether_rt_task_drop` needs to release it later. `store_at`
    /// already branches on aggregate-ness internally, so — unlike
    /// `gen_sync_return` — this needs no separate unit/aggregate/scalar
    /// cases: a unit output is just a zero-byte `memcpy`. Only the poll
    /// function's own `i8` "ready" return varies, always `1` here.
    fn gen_async_return(&self, frame: &FrameLayout<'ctx>, op: &Operand) {
        let frame_ptr = self
            .frame_ptr
            .expect("gen_async_return runs only inside a poll function, after its frame prologue");
        let ret_ty = self.mir_fn.ret.clone();
        let val = self.gen_operand(op);
        let output_addr = self
            .m
            .struct_gep(frame.ty, frame_ptr, frame.output_field, "output");
        self.store_at(output_addr, &ret_ty, val);

        let output_drop = self.func_ptr_or_null(self.shims.drop_shim(
            self.m,
            self.layout,
            self.runtime,
            &ret_ty,
        ));
        let output_drop_addr =
            self.m
                .struct_gep(frame.ty, frame_ptr, frame.output_drop_field, "output_drop");
        self.m.store(output_drop_addr, output_drop);

        // The escaping RVO value this whole crate's own ARC convention
        // deliberately never retains/releases for a direct or let-bound
        // return (`nether_mir::arc`'s own docs — "already start (or
        // arrive) at refcount 1 for this binding") relies on the
        // function's *own* storage simply ceasing to exist on return — a
        // stack `alloca` nobody ever reads again. A frame field is not
        // discarded that way: it just copied `op`'s value into the
        // output slot above, byte for byte, without an accompanying
        // retain, so the local's own field is now a stale *duplicate* of
        // the same reference, not an independent credit — exactly the
        // shape `Instr::Clear` already nulls for a moved-from unique
        // local, generalized here to any heap-kind escaping return value.
        // Left non-null, `frame::frame_drop_shim`'s later walk would
        // release it a second time on top of the output slot's own
        // eventual release — a real double-free this fixes.
        if let Operand::Local(local) = op {
            if !frame.alias_locals[local.index()] && alloc_kind(&ret_ty, self.defs()) == AllocKind::Heap {
                self.m.store(self.locals[local.index()], self.m.const_null_ptr());
            }
        }

        // Marks this frame permanently done — see
        // `FrameLayout::completed_state`'s own docs: real concurrent
        // scheduling can poll an already-finished task again (a spawned
        // task's own background polling racing an explicit `await` of
        // it), and without this, re-dispatching to whatever await-check
        // block the frame was last suspended at would re-run everything
        // from there, including side effects that already ran once.
        let state_addr = self
            .m
            .struct_gep(frame.ty, frame_ptr, frame.state_field, "state_field");
        self.m.store(
            state_addr,
            self.m
                .const_int(self.m.int_type(64), frame.completed_state(), false),
        );

        self.m.ret(Some(self.m.const_int(self.m.int_type(8), 1, false)));
    }

    /// The real suspend/resume codegen `Terminator::Await` exists for —
    /// replaces the milestone-1 placeholder that still blocked via
    /// `nether_rt_task_block_on`. Polls `task` exactly once via the
    /// generic `nether_rt_task_poll` (works uniformly whether `task` is
    /// another state-machine frame, a completed task, or a native timer
    /// task — every payload shares the same `poll`-fn-pointer-first
    /// header): on `Pending`, persists `block_id` itself — *not*
    /// `resume` — as the frame's own resume state
    /// (`nether_mir::split_await_points`'s own docs on why re-entering
    /// this exact, instruction-free check block, rather than skipping
    /// ahead, is what makes re-polling safe) and returns `Pending` (`i8
    /// 0`) up through this poll function's own caller in turn; on
    /// `Ready`, extracts `task`'s own output from its fixed byte-16 slot
    /// (identical addressing to the pre-split `gen_await`/
    /// `gen_completed_task` this replaces) and falls through to `resume`.
    fn gen_await_terminator(
        &mut self,
        block_id: nether_mir::BlockId,
        task: &Operand,
        output_local: Local,
        resume: nether_mir::BlockId,
    ) {
        let frame = self
            .frame
            .clone()
            .expect("Terminator::Await only ever occurs inside an async fn's poll function");
        let frame_ptr = self
            .frame_ptr
            .expect("gen_await_terminator runs after the frame prologue");

        let task_val = self.gen_operand(task);
        let ready_i8 = self
            .m
            .call(self.runtime.task_poll, &[task_val], "poll_ready")
            .expect("nether_rt_task_poll returns a value");
        let ready = self
            .m
            .int_cast(ready_i8, self.m.bool_type(), false, "poll_ready_bool");

        let ready_bb = self.m.append_block(self.llvm_fn, "await_ready");
        let pending_bb = self.m.append_block(self.llvm_fn, "await_pending");
        self.m.cond_br(ready, ready_bb, pending_bb);

        self.m.position_at_end(pending_bb);
        let state_addr = self
            .m
            .struct_gep(frame.ty, frame_ptr, frame.state_field, "state_field");
        let state = frame.state_of(block_id);
        self.m
            .store(state_addr, self.m.const_int(self.m.int_type(64), state, false));
        self.m.ret(Some(self.m.const_int(self.m.int_type(8), 0, false)));

        self.m.position_at_end(ready_bb);
        let output_ty = self.local_ty(output_local);
        let output_addr = self.m.gep_bytes(
            task_val,
            self.m.const_int(self.m.int_type(64), 16, false),
            "task_output",
        );
        let value = self.load_value(output_addr, &output_ty);
        self.store_value(output_local, &output_ty, value);
        self.m.br(self.blocks[&resume]);
    }
}
