//! # nether_codegen
//!
//! Purpose: translate ARC-annotated MIR into an LLVM module
//! (`docs/architecture/crates.md` § `compiler/codegen`).
//!
//! Responsibilities:
//! - Map every `nether_typecheck::Type` to an LLVM type and, for
//!   `Struct`/`TupleStruct`/`Enum`, a concrete field layout — decisions
//!   `type-system.md` deliberately leaves to codegen (see `layout.rs`'s
//!   module docs).
//! - Emit one LLVM function per [`nether_mir::MirFunction`], one LLVM
//!   basic block per MIR basic block — a close to 1:1 translation, since
//!   `mir` already did the hard work of turning structured control flow
//!   into an explicit CFG.
//! - Emit calls into `runtime`'s C-ABI functions for `Retain`/`Release`,
//!   `String`/`Array` operations, and heap allocation — declaring (never
//!   defining) them, per this crate's own invariant; see `runtime.rs`'s
//!   module docs for the concrete ABI, now backed by real
//!   `runtime/{arc,string,array,io}` crates.
//! - Emit the platform's own C-ABI `main`, calling the program's
//!   `nether_main` — see [`emit_entry_point`] — so `nether_driver` never
//!   needs a separate hand-written entry stub to link this module's
//!   object file into an executable.
//!
//! Codegen model (see `nether_llvm`'s own module docs for the LLVM-level
//! half of this): every MIR `Local` gets its own `alloca` in its
//! function's entry block, regardless of type — read via `load`, written
//! via `store`, exactly mirroring how `mir`'s own `Instr::Assign` already
//! treats locals as storage slots. A local of aggregate type (a stack
//! `Tuple`/camelCase `struct`/`enum`) is *addressed* through that
//! `alloca` directly (GEP into it in place) rather than loaded into an
//! LLVM aggregate SSA value; a heap-kind local's `alloca` just holds a
//! pointer to the actual heap object, which is where GEPs land instead.
//!
//! Input: `Vec<nether_mir::MirFunction>` (post-ARC), plus
//! `nether_resolver::Definitions`/`nether_typecheck::Signatures` (needed
//! for the same reasons `nether_mir::build_mir` needed them — layout
//! information `MonoModule`/`MirFunction` don't carry forward
//! themselves).
//!
//! Output: an LLVM module, built through [`nether_llvm::ModuleCx`].
//!
//! Dependencies: `nether_ast`, `nether_resolver`, `nether_typecheck`,
//! `nether_monomorphization`, `nether_mir`, `nether_llvm`.
//!
//! Consumers: `nether_driver` (which hands the module to `nether_llvm`
//! for optimization/emission).
//!
//! Invariants: performs no semantic checking of its own — by the time
//! MIR reaches this crate, the program is assumed well-typed and
//! ARC-correct; any inconsistency found here (a `panic!`) is an internal
//! compiler error, not a user diagnostic (`overview.md` §2's front-end/
//! back-end split).
//!
//! Documented simplification:
//! - An `enum`'s LLVM layout is a flat, non-overlapping struct (every
//!   variant's fields coexist, unused ones simply wasted) rather than a
//!   real overlapping tagged union — see `layout.rs`'s `EnumLayout` docs.
//!
//! Runtime calls target the implemented C ABI in `runtime/*`; dynamic
//! closure calls use an explicit code-pointer/environment ABI.

mod frame;
mod function;
mod layout;
mod runtime;
mod shims;

use std::collections::HashMap;

use nether_llvm::{Codegen, Func, ModuleCx};
use nether_mir::MirFunction;
use nether_monomorphization::MonoFnId;
use nether_resolver::{DefId, Definitions};
use nether_typecheck::{Signatures, Type};

pub use layout::Layout;
pub use runtime::Runtime;
pub use shims::Shims;

/// Builds one LLVM module named `name` containing every function in
/// `functions`.
pub fn generate<'ctx>(
    cg: &'ctx Codegen,
    name: &str,
    functions: &[MirFunction],
    defs: &Definitions,
    sigs: &Signatures,
) -> ModuleCx<'ctx> {
    let m = cg.module(name);
    let layout = Layout::new(&m, defs, sigs);
    let runtime = Runtime::declare(&m);
    let shims = Shims::new();

    let llvm_fns: Vec<Func<'ctx>> = functions.iter().map(|f| declare(&m, &layout, f)).collect();
    let funcs: HashMap<MonoFnId, Func<'ctx>> = functions
        .iter()
        .map(|f| f.id)
        .zip(llvm_fns.iter().copied())
        .collect();
    // A second LLVM function per `is_async` `MirFunction` — see
    // `function::async_fn`'s module docs: `funcs`'s entry stays the
    // "start" function (same symbol/param convention every caller already
    // uses, only its return type changes), while `poll_funcs` holds the
    // `PollFn`-shaped function that actually carries the translated MIR
    // body.
    let poll_funcs: HashMap<MonoFnId, Func<'ctx>> = functions
        .iter()
        .filter(|f| f.is_async)
        .map(|f| (f.id, declare_poll(&m, f)))
        .collect();
    let mir_functions: HashMap<MonoFnId, &MirFunction> =
        functions.iter().map(|f| (f.id, f)).collect();
    let function_env = function::FunctionEnv {
        m: &m,
        layout: &layout,
        runtime: &runtime,
        shims: &shims,
        funcs: &funcs,
        poll_funcs: &poll_funcs,
        mir_functions: &mir_functions,
    };

    for (f, &llvm_fn) in functions.iter().zip(&llvm_fns) {
        // An `extern "C"` declaration is declared (above, via `declare`'s
        // own `is_extern` branch), never defined — there is no Nether-side
        // body to build (`crates.md`'s own invariant, mirrors how
        // `runtime.rs`'s own C-ABI symbols are declared-only).
        if f.is_extern {
            continue;
        }
        function::build_function(&function_env, f, llvm_fn);
    }

    emit_entry_point(&m, &runtime, functions, &funcs);

    m
}

/// Emits the platform's own C-ABI `main` — a tiny wrapper calling the
/// Nether program's `nether_main` — so the object file this module
/// becomes is directly linkable by the system linker with no separate
/// hand-written entry stub (`nether_driver` only ever has to invoke `cc`
/// against this object plus `runtime`'s static libraries). Only emitted
/// when a top-level `main` is actually present — `nether_driver` only
/// reaches `generate` at all once `monomorphize` has confirmed one exists
/// (see its own module docs), so this is really just "always," but the
/// check is cheap insurance against ever calling `generate` on a
/// `main`-less module in the future.
///
/// An `is_async` `main` needs one more step than an ordinary call: its
/// `nether_main` symbol is now the *start* function (see `declare`'s own
/// `is_async` branch) — calling it only constructs the root frame, it
/// doesn't run anything yet, so this wrapper also has to drive that frame
/// to completion (`nether_rt_task_block_on`, the same runtime entry point
/// an ordinary `await` polls through) and release it once it's done —
/// `main`'s frame has no Nether-level owner of its own to do that for it.
fn emit_entry_point<'ctx>(
    m: &ModuleCx<'ctx>,
    runtime: &Runtime<'ctx>,
    functions: &[MirFunction],
    funcs: &HashMap<MonoFnId, Func<'ctx>>,
) {
    let Some(main_fn) = functions
        .iter()
        .find(|f| f.owner.is_none() && f.name.as_str() == "main")
    else {
        return;
    };
    let nether_main = funcs[&main_fn.id];
    let i32_ty = m.int_type(32);
    let entry = m.declare_function("main", m.fn_type(&[], Some(i32_ty)));
    let block = m.append_block(entry, "entry");
    m.position_at_end(block);
    if main_fn.is_async {
        let frame = m
            .call(nether_main, &[], "main_frame")
            .expect("an async start function always returns the frame pointer");
        m.call(runtime.task_block_on, &[frame], "");
        m.call(runtime.release, &[frame], "");
    } else {
        m.call(nether_main, &[], "");
    }
    m.ret(Some(m.const_int(i32_ty, 0, false)));
}

fn declare<'ctx>(m: &ModuleCx<'ctx>, layout: &Layout<'_, 'ctx>, f: &MirFunction) -> Func<'ctx> {
    if f.is_extern {
        return declare_extern(m, layout, f);
    }
    // A stack aggregate (`Tuple`/camelCase `struct`/`enum`) and every
    // mutable parameter are passed by pointer. See
    // `function::FnCodegen::build`'s matching entry-block handling and
    // `crate`'s module docs on aggregate addressing; the pointer is also
    // what gives a `mut` scalar access to the caller's original slot.
    let mut param_tys: Vec<_> = Vec::new();
    if f.is_closure {
        param_tys.push(m.ptr_type());
    }
    param_tys.extend(f.params.iter().map(|&p| {
        let decl = f.local_decl(p);
        if (decl.mutable && !matches!(decl.ty, Type::Ref(_) | Type::MutRef(_)))
            || function::is_aggregate(&decl.ty, layout.defs)
        {
            m.ptr_type()
        } else {
            layout.llvm_type(&decl.ty)
        }
    }));
    // An `is_async` function's own LLVM symbol becomes its "start"
    // function (see `function::async_fn`) — it only ever constructs and
    // returns a `Task<T>` frame pointer, so its LLVM return type is
    // always `ptr`, regardless of `f.ret` (the *unwrapped* output type,
    // which becomes the frame's own output field instead — see
    // `frame::compute_frame_layout`).
    let ret_ty = if f.is_async {
        Some(m.ptr_type())
    } else if is_unit(&f.ret) {
        None
    } else {
        Some(layout.llvm_type(&f.ret))
    };
    let fn_ty = m.fn_type(&param_tys, ret_ty);
    m.declare_function(&mangled_name(f), fn_ty)
}

/// Declares an `extern "C" { ... }` member under its own real name (the
/// actual C symbol — never mangled, unlike an ordinary Nether function;
/// `extern "C" fn`s are always free functions, so there is no `(owner,
/// name)` ambiguity `mangled_name` exists to resolve) using a direct
/// scalar/pointer C-ABI type mapping, never Nether's own internal
/// "mutable/aggregate parameters pass by pointer" convention — FFI-safety
/// checking (`nether_typecheck::check::is_ffi_safe_type`) already excludes
/// every type that convention would otherwise apply to.
fn declare_extern<'ctx>(m: &ModuleCx<'ctx>, layout: &Layout<'_, 'ctx>, f: &MirFunction) -> Func<'ctx> {
    let param_tys: Vec<_> = f
        .params
        .iter()
        .map(|&p| ffi_type(m, layout, &f.local_decl(p).ty))
        .collect();
    let ret_ty = if is_unit(&f.ret) {
        None
    } else {
        Some(ffi_type(m, layout, &f.ret))
    };
    let fn_ty = m.fn_type(&param_tys, ret_ty);
    m.declare_function(f.name.as_str(), fn_ty)
}

/// `layout.llvm_type`, except `bool` maps to `i8` — the real C ABI width
/// (`runtime.rs`'s own module docs explain why LLVM's native `i1` isn't
/// safe to use across a C-ABI boundary). FFI-safety checking guarantees
/// `ty` is always a primitive or raw pointer here, never an aggregate.
fn ffi_type<'ctx>(m: &ModuleCx<'ctx>, layout: &Layout<'_, 'ctx>, ty: &Type) -> nether_llvm::Ty<'ctx> {
    match ty {
        Type::Primitive(nether_typecheck::PrimitiveKind::Bool) => m.int_type(8),
        _ => layout.llvm_type(ty),
    }
}

/// The second LLVM function an `is_async` `MirFunction` needs — matches
/// `runtime/task`'s `PollFn` ABI (`fn(*mut u8) -> bool`, `bool` crossing
/// as `i8` — see `runtime.rs`'s own module docs) exactly, so it's usable
/// directly as the frame's own `poll` field once its address is stored
/// there (`function::async_fn::build_async_start`).
fn declare_poll<'ctx>(m: &ModuleCx<'ctx>, f: &MirFunction) -> Func<'ctx> {
    let i8_ty = m.int_type(8);
    let fn_ty = m.fn_type(&[m.ptr_type()], Some(i8_ty));
    m.declare_function(&poll_mangled_name(f), fn_ty)
}

fn is_unit(ty: &nether_typecheck::Type) -> bool {
    matches!(ty, nether_typecheck::Type::Tuple(elems) if elems.is_empty())
}

/// LLVM has one flat symbol namespace per module, but two distinct
/// methods (or an inherited default and its override) can share a bare
/// name (`Signatures::methods` disambiguates by `(owner, name)`, not name
/// alone) — so every function's LLVM symbol is qualified by its owner,
/// not just its declared name.
fn mangled_name(f: &MirFunction) -> String {
    if f.owner.is_none() && !f.is_closure && f.name.as_str() == "main" {
        return "nether_main".to_string();
    }
    match f.owner {
        Some(owner) => format!(
            "nether_{}_{}_mono{}",
            owner_tag(owner),
            f.name,
            f.id.index()
        ),
        None => format!("nether_{}_mono{}", f.name, f.id.index()),
    }
}

fn owner_tag(id: DefId) -> String {
    format!("{id:?}").replace(['(', ')'], "_")
}

fn poll_mangled_name(f: &MirFunction) -> String {
    format!("{}_poll", mangled_name(f))
}
