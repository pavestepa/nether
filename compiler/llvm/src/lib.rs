//! # nether_llvm
//!
//! Purpose: the one crate that touches `inkwell`/LLVM directly — the
//! backend abstraction boundary described in `overview.md` §5
//! (`docs/architecture/crates.md` § `compiler/llvm`).
//!
//! Responsibilities:
//! - Own an `inkwell::context::Context` and expose module/function/
//!   instruction-building through a small, purpose-built facade
//!   ([`Codegen`], [`ModuleCx`]) shaped around exactly the operations
//!   `nether_codegen` needs (`docs/architecture/overview.md` §4's MIR
//!   instruction set), rather than exposing `inkwell`'s full surface.
//! - `nether_codegen` never writes `use inkwell::...` or lists `inkwell`
//!   in its own `Cargo.toml` — every `inkwell` type it touches only ever
//!   arrives as one of this crate's re-exported type aliases ([`Value`],
//!   [`Func`], [`Block`], [`Ty`], [`StructTy`], [`FnTy`]). This is the
//!   practical shape of `crates.md`'s stated invariant ("no other crate
//!   in `compiler/` imports `inkwell`"): a thin facade crate, not a
//!   fully-erased `dyn Backend` (the loose sketch in `crates.md`) — every
//!   value here is already a cheap `Copy` handle borrowed from the
//!   `Context`, so an additional newtype-wrapping layer would only add
//!   boilerplate, not safety.
//! - Own LLVM optimization ([`ModuleCx::optimize`]) and target-machine
//!   object emission ([`ModuleCx::emit_object`]) for a driver-selected
//!   LLVM triple.
//!
//! Codegen model: every struct/tuple/enum value (heap *or* stack) is
//! addressed through a pointer — a heap allocation for `AllocKind::Heap`
//! types, a `alloca` stack slot for `AllocKind::Stack` ones — and always
//! read/written through [`ModuleCx::load`]/[`ModuleCx::store`]/
//! [`ModuleCx::struct_gep`]. This is a deliberate simplification (the
//! same "always through a pointer" shape `-O0` output from a mainstream
//! compiler has): it avoids ever needing LLVM's `insertvalue`/
//! `extractvalue` aggregate-SSA-value instructions, at the cost of some
//! `alloca`s LLVM's own `mem2reg`/`sroa` optimization passes (run via
//! [`ModuleCx::optimize`]) clean up.
//!
//! Dependencies: `inkwell` (LLVM Rust bindings, version pinned to LLVM 18
//! — see `../../.cargo/config.toml`) — external, no other compiler crate.
//!
//! Consumers: `nether_codegen`.
//!
//! Invariants: no other crate in `compiler/` imports `inkwell` or
//! `llvm-sys` directly.
//!
//! Future extension points: a second
//! backend (Cranelift/WASM/C) would be a sibling crate exposing the same
//! shape of facade `nether_codegen` could be adapted to target.

use std::path::Path;

use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::targets::{CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine, TargetTriple};
use inkwell::AddressSpace;

pub use inkwell::passes::PassBuilderOptions;
pub use inkwell::types::{BasicMetadataTypeEnum as ParamTy, BasicTypeEnum as Ty, FunctionType as FnTy, StructType as StructTy};
pub use inkwell::values::{BasicMetadataValueEnum as ArgValue, BasicValueEnum as Value, FunctionValue as Func};
pub use inkwell::{FloatPredicate, IntPredicate};
pub type Block<'ctx> = inkwell::basic_block::BasicBlock<'ctx>;

/// Owns the LLVM `Context` every [`ModuleCx`] borrows from. One per
/// compilation — `nether_driver` creates exactly one for the whole
/// program (all functions across the monomorphized program share a
/// single LLVM module, per `docs/architecture/crates.md`'s `codegen`
/// section).
pub struct Codegen {
    context: Context,
    target_triple: String,
    opt_level: u8,
}

impl Default for Codegen {
    fn default() -> Self {
        Self::new()
    }
}

impl Codegen {
    pub fn new() -> Self {
        Self::with_target(&Self::host_triple(), 0).expect("the host LLVM target must be available")
    }

    pub fn host_triple() -> String {
        TargetMachine::get_default_triple().as_str().to_string_lossy().into_owned()
    }

    pub fn with_target(target_triple: &str, opt_level: u8) -> Result<Self, String> {
        if opt_level > 3 {
            return Err(format!("unsupported optimization level O{opt_level}; expected O0..O3"));
        }
        Target::initialize_all(&InitializationConfig::default());
        let triple = TargetTriple::create(target_triple);
        Target::from_triple(&triple).map_err(|error| error.to_string())?;
        Ok(Codegen {
            context: Context::create(),
            target_triple: target_triple.to_string(),
            opt_level,
        })
    }

    pub fn module(&self, name: &str) -> ModuleCx<'_> {
        let module = self.context.create_module(name);
        let machine = create_target_machine(&self.target_triple, self.opt_level)
            .expect("target was validated by Codegen::with_target");
        module.set_triple(&machine.get_triple());
        module.set_data_layout(&machine.get_target_data().get_data_layout());
        let builder = self.context.create_builder();
        ModuleCx {
            context: &self.context,
            module,
            builder,
            target_triple: self.target_triple.clone(),
            opt_level: self.opt_level,
        }
    }
}

/// One LLVM module under construction, plus the single [`Builder`] used
/// to build every function in it (`inkwell`'s builder is repositioned per
/// block via [`ModuleCx::position_at_end`], not recreated).
pub struct ModuleCx<'ctx> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    target_triple: String,
    opt_level: u8,
}

impl<'ctx> ModuleCx<'ctx> {
    // ---- Types ---------------------------------------------------------

    pub fn int_type(&self, bits: u32) -> Ty<'ctx> {
        self.context.custom_width_int_type(bits).into()
    }

    pub fn bool_type(&self) -> Ty<'ctx> {
        self.context.bool_type().into()
    }

    pub fn f32_type(&self) -> Ty<'ctx> {
        self.context.f32_type().into()
    }

    pub fn f64_type(&self) -> Ty<'ctx> {
        self.context.f64_type().into()
    }

    /// A single opaque pointer type — every heap object, `weak T`, and
    /// stack-aggregate address is this one type at the LLVM level
    /// (`AddressSpace` 0 for the currently supported native targets).
    pub fn ptr_type(&self) -> Ty<'ctx> {
        self.context.ptr_type(AddressSpace::default()).into()
    }

    pub fn struct_type(&self, fields: &[Ty<'ctx>]) -> StructTy<'ctx> {
        let fields: Vec<_> = fields.to_vec();
        self.context.struct_type(&fields, false)
    }

    pub fn fn_type(&self, params: &[Ty<'ctx>], ret: Option<Ty<'ctx>>) -> FnTy<'ctx> {
        let params: Vec<ParamTy<'ctx>> = params.iter().map(|t| (*t).into()).collect();
        match ret {
            Some(ret) => param_ty_to_fn(ret, &params),
            None => self.context.void_type().fn_type(&params, false),
        }
    }

    /// `ty`'s size in bytes as an `i64` constant expression (LLVM
    /// resolves it once the target's data layout is known — no
    /// `TargetData` query needed at codegen time) — the size
    /// [`ModuleCx::declare_function`]-adjacent runtime allocation calls
    /// need to hand to `nether_alloc`.
    pub fn size_of(&self, ty: Ty<'ctx>) -> Value<'ctx> {
        let size = match ty {
            Ty::ArrayType(t) => t.size_of().expect("sized array type"),
            Ty::FloatType(t) => t.size_of(),
            Ty::IntType(t) => t.size_of(),
            Ty::PointerType(t) => t.size_of(),
            Ty::StructType(t) => t.size_of().expect("sized struct type"),
            Ty::VectorType(t) => t.size_of().expect("sized vector type"),
        };
        size.into()
    }

    // ---- Functions -------------------------------------------------------

    /// Declares (does not define) an external function — used both for a
    /// function this module will later define ([`Self::append_block`]
    /// gives it a body) and for a `runtime` C-ABI symbol this module only
    /// ever calls (never defines; `docs/architecture/crates.md` §
    /// `compiler/codegen`'s "declare, never define" invariant).
    pub fn declare_function(&self, name: &str, ty: FnTy<'ctx>) -> Func<'ctx> {
        self.module.get_function(name).unwrap_or_else(|| self.module.add_function(name, ty, Some(Linkage::External)))
    }

    pub fn append_block(&self, f: Func<'ctx>, name: &str) -> Block<'ctx> {
        self.context.append_basic_block(f, name)
    }

    pub fn position_at_end(&self, block: Block<'ctx>) {
        self.builder.position_at_end(block);
    }

    /// The block the builder is currently positioned at — used to save
    /// and restore an insertion point around a nested piece of function
    /// building (`nether_codegen`'s per-`Type` retain/drop shims are
    /// generated on demand while another function's body is mid-build;
    /// see its `shims` module).
    pub fn current_block(&self) -> Block<'ctx> {
        self.builder.get_insert_block().expect("builder has an active insertion point")
    }

    pub fn param(&self, f: Func<'ctx>, index: u32) -> Value<'ctx> {
        f.get_nth_param(index).unwrap_or_else(|| panic!("function has no parameter #{index}"))
    }

    // ---- Constants -------------------------------------------------------

    pub fn const_int(&self, ty: Ty<'ctx>, value: u64, sign_extend: bool) -> Value<'ctx> {
        ty.into_int_type().const_int(value, sign_extend).into()
    }

    pub fn const_float(&self, ty: Ty<'ctx>, value: f64) -> Value<'ctx> {
        ty.into_float_type().const_float(value).into()
    }

    pub fn const_bool(&self, value: bool) -> Value<'ctx> {
        self.context.bool_type().const_int(u64::from(value), false).into()
    }

    pub fn const_null_ptr(&self) -> Value<'ctx> {
        self.context.ptr_type(AddressSpace::default()).const_null().into()
    }

    /// A pointer to a private global holding `s` as a NUL-terminated byte
    /// array — the raw material `nether_codegen` hands to a `runtime`
    /// string-construction call for a string literal.
    pub fn global_string_ptr(&self, s: &str, name: &str) -> Value<'ctx> {
        self.builder.build_global_string_ptr(s, name).expect("build_global_string_ptr").as_pointer_value().into()
    }

    // ---- Memory ------------------------------------------------------

    pub fn alloca(&self, ty: Ty<'ctx>, name: &str) -> Value<'ctx> {
        self.builder.build_alloca(ty, name).expect("build_alloca").into()
    }

    pub fn load(&self, ty: Ty<'ctx>, ptr: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder.build_load(ty, ptr.into_pointer_value(), name).expect("build_load")
    }

    pub fn store(&self, ptr: Value<'ctx>, value: Value<'ctx>) {
        self.builder.build_store(ptr.into_pointer_value(), value).expect("build_store");
    }

    pub fn struct_gep(&self, struct_ty: StructTy<'ctx>, ptr: Value<'ctx>, index: u32, name: &str) -> Value<'ctx> {
        self.builder.build_struct_gep(struct_ty, ptr.into_pointer_value(), index, name).expect("build_struct_gep").into()
    }

    /// Byte-offset GEP into an opaque pointer treated as `[N x i8]` — used
    /// for array element addressing (`runtime/array`'s own layout, once
    /// it exists, is what `index` is ultimately relative to).
    pub fn gep_bytes(&self, ptr: Value<'ctx>, byte_offset: Value<'ctx>, name: &str) -> Value<'ctx> {
        let i8_ty = self.context.i8_type();
        unsafe {
            self.builder.build_gep(i8_ty, ptr.into_pointer_value(), &[byte_offset.into_int_value()], name).expect("build_gep").into()
        }
    }

    /// Copies `size` bytes from `src` to `dest` — how a stack-kind
    /// aggregate (`Tuple`/a camelCase `struct`/an `enum`, all addressed
    /// directly through their own `alloca`, never loaded into an LLVM
    /// aggregate SSA value — see this crate's module docs) is moved from
    /// one storage slot to another.
    pub fn memcpy(&self, dest: Value<'ctx>, src: Value<'ctx>, size: Value<'ctx>) {
        self.builder.build_memcpy(dest.into_pointer_value(), 8, src.into_pointer_value(), 8, size.into_int_value()).expect("build_memcpy");
    }

    // ---- Arithmetic / comparison ---------------------------------------

    pub fn int_add(&self, a: Value<'ctx>, b: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder.build_int_add(a.into_int_value(), b.into_int_value(), name).expect("build_int_add").into()
    }

    pub fn int_sub(&self, a: Value<'ctx>, b: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder.build_int_sub(a.into_int_value(), b.into_int_value(), name).expect("build_int_sub").into()
    }

    pub fn int_mul(&self, a: Value<'ctx>, b: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder.build_int_mul(a.into_int_value(), b.into_int_value(), name).expect("build_int_mul").into()
    }

    pub fn int_signed_div(&self, a: Value<'ctx>, b: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder.build_int_signed_div(a.into_int_value(), b.into_int_value(), name).expect("build_int_signed_div").into()
    }

    pub fn int_unsigned_div(&self, a: Value<'ctx>, b: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder.build_int_unsigned_div(a.into_int_value(), b.into_int_value(), name).expect("build_int_unsigned_div").into()
    }

    pub fn int_signed_rem(&self, a: Value<'ctx>, b: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder.build_int_signed_rem(a.into_int_value(), b.into_int_value(), name).expect("build_int_signed_rem").into()
    }

    pub fn int_unsigned_rem(&self, a: Value<'ctx>, b: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder.build_int_unsigned_rem(a.into_int_value(), b.into_int_value(), name).expect("build_int_unsigned_rem").into()
    }

    pub fn int_neg(&self, a: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder.build_int_neg(a.into_int_value(), name).expect("build_int_neg").into()
    }

    pub fn int_not(&self, a: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder.build_not(a.into_int_value(), name).expect("build_not").into()
    }

    /// Sign- or zero-extends/truncates `a` to `target: IntType` per
    /// `signed` — used to widen every integer primitive uniformly to
    /// `i64` before handing it to `runtime`'s single `nether_i64_to_string`
    /// (`ToString`'s codegen, `nether_codegen`).
    pub fn int_cast(&self, a: Value<'ctx>, target: Ty<'ctx>, signed: bool, name: &str) -> Value<'ctx> {
        self.builder.build_int_cast_sign_flag(a.into_int_value(), target.into_int_type(), signed, name).expect("build_int_cast_sign_flag").into()
    }

    pub fn float_ext(&self, a: Value<'ctx>, target: Ty<'ctx>, name: &str) -> Value<'ctx> {
        self.builder.build_float_ext(a.into_float_value(), target.into_float_type(), name).expect("build_float_ext").into()
    }

    pub fn select(&self, cond: Value<'ctx>, then_val: Value<'ctx>, else_val: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder.build_select(cond.into_int_value(), then_val, else_val, name).expect("build_select")
    }

    pub fn int_and(&self, a: Value<'ctx>, b: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder.build_and(a.into_int_value(), b.into_int_value(), name).expect("build_and").into()
    }

    pub fn int_or(&self, a: Value<'ctx>, b: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder.build_or(a.into_int_value(), b.into_int_value(), name).expect("build_or").into()
    }

    pub fn int_compare(&self, pred: IntPredicate, a: Value<'ctx>, b: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder.build_int_compare(pred, a.into_int_value(), b.into_int_value(), name).expect("build_int_compare").into()
    }

    pub fn float_add(&self, a: Value<'ctx>, b: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder.build_float_add(a.into_float_value(), b.into_float_value(), name).expect("build_float_add").into()
    }

    pub fn float_sub(&self, a: Value<'ctx>, b: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder.build_float_sub(a.into_float_value(), b.into_float_value(), name).expect("build_float_sub").into()
    }

    pub fn float_mul(&self, a: Value<'ctx>, b: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder.build_float_mul(a.into_float_value(), b.into_float_value(), name).expect("build_float_mul").into()
    }

    pub fn float_div(&self, a: Value<'ctx>, b: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder.build_float_div(a.into_float_value(), b.into_float_value(), name).expect("build_float_div").into()
    }

    pub fn float_neg(&self, a: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder.build_float_neg(a.into_float_value(), name).expect("build_float_neg").into()
    }

    pub fn float_compare(&self, pred: FloatPredicate, a: Value<'ctx>, b: Value<'ctx>, name: &str) -> Value<'ctx> {
        self.builder.build_float_compare(pred, a.into_float_value(), b.into_float_value(), name).expect("build_float_compare").into()
    }

    // ---- Control flow / calls ------------------------------------------

    pub fn call(&self, f: Func<'ctx>, args: &[Value<'ctx>], name: &str) -> Option<Value<'ctx>> {
        let args: Vec<ArgValue<'ctx>> = args.iter().map(|v| (*v).into()).collect();
        self.builder.build_call(f, &args, name).expect("build_call").try_as_basic_value().left()
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
        self.builder.build_unconditional_branch(target).expect("build_unconditional_branch");
    }

    pub fn cond_br(&self, cond: Value<'ctx>, then_block: Block<'ctx>, else_block: Block<'ctx>) {
        self.builder.build_conditional_branch(cond.into_int_value(), then_block, else_block).expect("build_conditional_branch");
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

    // ---- Module-level ----------------------------------------------------

    /// # Errors
    /// Returns the LLVM verifier's diagnostic text if the constructed
    /// module is malformed — a bug in `nether_codegen`, not a user error
    /// (`crates.md`'s stated invariant that `codegen` performs no
    /// semantic checking of its own).
    pub fn verify(&self) -> Result<(), String> {
        self.module.verify().map_err(|e| e.to_string())
    }

    pub fn print_to_string(&self) -> String {
        self.module.print_to_string().to_string()
    }

    /// Runs LLVM's new pass manager pipeline (`"default<O{level}>"`) over
    /// the module in place.
    pub fn optimize(&self, level: u8) {
        let machine = self.target_machine();
        self.module
            .run_passes(&format!("default<O{level}>"), &machine, PassBuilderOptions::create())
            .expect("run_passes");
    }

    /// # Errors
    /// Propagates any I/O error writing the object file.
    pub fn emit_object(&self, out: &Path) -> std::io::Result<()> {
        if self.opt_level > 0 {
            self.optimize(self.opt_level);
        }
        let machine = self.target_machine();
        machine.write_to_file(&self.module, FileType::Object, out).map_err(|e| std::io::Error::other(e.to_string()))
    }

    fn target_machine(&self) -> TargetMachine {
        create_target_machine(&self.target_triple, self.opt_level)
            .expect("target was validated by Codegen::with_target")
    }
}

fn create_target_machine(target_triple: &str, opt_level: u8) -> Result<TargetMachine, String> {
    let triple = TargetTriple::create(target_triple);
    let target = Target::from_triple(&triple).map_err(|error| error.to_string())?;
    let optimization = match opt_level {
        0 => inkwell::OptimizationLevel::None,
        1 => inkwell::OptimizationLevel::Less,
        2 => inkwell::OptimizationLevel::Default,
        _ => inkwell::OptimizationLevel::Aggressive,
    };
    target
        .create_target_machine(&triple, "generic", "", optimization, RelocMode::PIC, CodeModel::Default)
        .ok_or_else(|| format!("cannot create target machine for `{target_triple}`"))
}

fn param_ty_to_fn<'ctx>(ret: Ty<'ctx>, params: &[ParamTy<'ctx>]) -> FnTy<'ctx> {
    match ret {
        Ty::ArrayType(t) => t.fn_type(params, false),
        Ty::FloatType(t) => t.fn_type(params, false),
        Ty::IntType(t) => t.fn_type(params, false),
        Ty::PointerType(t) => t.fn_type(params, false),
        Ty::StructType(t) => t.fn_type(params, false),
        Ty::VectorType(t) => t.fn_type(params, false),
    }
}
