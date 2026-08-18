use nether_llvm::{Func, ModuleCx};

/// Declares (never defines -- `docs/architecture/crates.md` section
/// `compiler/codegen`'s own invariant) every `runtime` C-ABI symbol this
/// crate calls into. These signatures match the real, implemented
/// `runtime/{arc,string,array,io}` crates exactly -- see each field's doc
/// comment for the corresponding Rust definition.
///
/// `bool` crosses this boundary as `i8` (0/1), not LLVM's native `i1`:
/// the AArch64 C calling convention extends a C `_Bool` (and Rust's
/// `bool`, which matches it) to a full byte, not a single bit, so a
/// declared `i1` parameter/return risks a real ABI mismatch against
/// `extern "C" fn(bool)`/`-> bool` compiled by `rustc`. Every call site
/// that produces or consumes one of these `i8` values does the
/// `zext`/`trunc` to/from Nether's own `i1` bool representation itself
/// (`function.rs`).
pub struct Runtime<'ctx> {
    /// `nether_rt_arc::nether_rt_arc_alloc(size: i64, drop: Option<extern "C" fn(*mut u8)>) -> ptr`
    pub alloc: Func<'ctx>,
    pub unique_alloc: Func<'ctx>,
    pub unique_free: Func<'ctx>,
    pub unique_promote: Func<'ctx>,
    /// `nether_rt_arc::nether_rt_arc_retain(ptr)`
    pub retain: Func<'ctx>,
    /// `nether_rt_arc::nether_rt_arc_release(ptr)`
    pub release: Func<'ctx>,
    /// `nether_rt_arc::nether_rt_arc_weak_retain(ptr)` -- bumps a `weak T`
    /// value's own weak count, never its referent's strong count (spec
    /// §13). Emitted for [`nether_mir::Instr::WeakRetain`].
    pub weak_retain: Func<'ctx>,
    /// `nether_rt_arc::nether_rt_arc_weak_release(ptr)`. Emitted for
    /// [`nether_mir::Instr::WeakRelease`].
    pub weak_release: Func<'ctx>,
    /// `nether_rt_arc::nether_rt_arc_weak_upgrade(ptr, out_ptr: ptr) -> i8`
    /// -- `0` if the referent is gone; otherwise retains it and writes
    /// its pointer through `out_ptr`, returning `1`. The one real
    /// implementation behind the `__weak_upgrade` builtin every `weak
    /// T` read desugars to (`nether_hir`'s own module docs) — shaped
    /// like `nether_rt_array_pop` so an `Option<T>` construction follows
    /// the same pattern.
    pub weak_upgrade: Func<'ctx>,
    /// `nether_rt_io::nether_rt_io_println(s: ptr)`
    pub println: Func<'ctx>,
    /// `nether_rt_io::nether_rt_io_print(s: ptr)`
    pub print: Func<'ctx>,
    /// `nether_rt_string::nether_rt_string_from_utf8(bytes: ptr, len: i64) -> ptr`
    pub string_from_bytes: Func<'ctx>,
    /// `nether_rt_string::nether_rt_string_concat(a: ptr, b: ptr) -> ptr`
    pub string_concat: Func<'ctx>,
    pub i64_to_string: Func<'ctx>,
    pub f64_to_string: Func<'ctx>,
    /// `nether_rt_string::nether_rt_bool_to_string(v: i8) -> ptr`
    pub bool_to_string: Func<'ctx>,
    pub char_to_string: Func<'ctx>,
    /// `nether_rt_array::nether_rt_array_new(elem_size: i64, cap: i64,
    /// elem_retain: Option<extern "C" fn(*mut u8)>, elem_drop: Option<extern
    /// "C" fn(*mut u8)>) -> ptr` -- the retain/drop callbacks operate on
    /// one element's own slot address inside the array's storage (see
    /// `crate::shims`'s module docs); `None` for an element type with no
    /// heap-kind content at all.
    pub array_new: Func<'ctx>,
    /// `nether_rt_array::nether_rt_array_push(arr: ptr, elem: ptr)` --
    /// `elem` points at `elem_size` bytes to copy in; internally calls
    /// `elem_retain` on the copied slot if one was given at `array_new`
    /// time (the array becoming an independent owner, mirroring how a
    /// field *store* retains its new value -- `arc-model.md` section 3.1).
    pub array_push: Func<'ctx>,
    pub array_len: Func<'ctx>,
    /// `nether_rt_array::nether_rt_array_get(arr: ptr, index: i64) -> ptr`
    /// -- pointer to the element's own storage inside the array (the
    /// caller knows `elem_size`/its LLVM type from the array's static
    /// element type, and loads/stores through this pointer directly).
    pub array_get: Func<'ctx>,
    /// `nether_rt_array::nether_rt_array_pop(arr: ptr, out_elem: ptr) -> i8`
    /// -- `0` if empty; otherwise pops the last element into `out_elem`
    /// (`elem_size` bytes) and returns `1`. Calls neither `elem_retain`
    /// nor `elem_drop`: ownership transfers whole to the caller's fresh
    /// binding, matching every other call result's "arrives already
    /// owned" convention (`arc-model.md` section 3.4). Shaped this way so
    /// `nether_codegen` can build an `Option<T>` construction directly
    /// from the boolean result.
    pub array_pop: Func<'ctx>,
}

impl<'ctx> Runtime<'ctx> {
    pub fn declare(m: &ModuleCx<'ctx>) -> Self {
        let ptr = m.ptr_type();
        let i64_ty = m.int_type(64);
        let i8_ty = m.int_type(8);
        let f64_ty = m.f64_type();
        let void_fn = |params: &[nether_llvm::Ty<'ctx>]| m.fn_type(params, None);

        Runtime {
            alloc: m.declare_function("nether_rt_arc_alloc", m.fn_type(&[i64_ty, ptr], Some(ptr))),
            unique_alloc: m.declare_function(
                "nether_rt_unique_alloc",
                m.fn_type(&[i64_ty, ptr], Some(ptr)),
            ),
            unique_free: m.declare_function("nether_rt_unique_free", void_fn(&[ptr])),
            unique_promote: m
                .declare_function("nether_rt_unique_promote", m.fn_type(&[ptr], Some(ptr))),
            retain: m.declare_function("nether_rt_arc_retain", void_fn(&[ptr])),
            release: m.declare_function("nether_rt_arc_release", void_fn(&[ptr])),
            weak_retain: m.declare_function("nether_rt_arc_weak_retain", void_fn(&[ptr])),
            weak_release: m.declare_function("nether_rt_arc_weak_release", void_fn(&[ptr])),
            weak_upgrade: m.declare_function(
                "nether_rt_arc_weak_upgrade",
                m.fn_type(&[ptr, ptr], Some(i8_ty)),
            ),
            println: m.declare_function("nether_rt_io_println", void_fn(&[ptr])),
            print: m.declare_function("nether_rt_io_print", void_fn(&[ptr])),
            string_from_bytes: m.declare_function(
                "nether_rt_string_from_utf8",
                m.fn_type(&[ptr, i64_ty], Some(ptr)),
            ),
            string_concat: m
                .declare_function("nether_rt_string_concat", m.fn_type(&[ptr, ptr], Some(ptr))),
            i64_to_string: m
                .declare_function("nether_rt_i64_to_string", m.fn_type(&[i64_ty], Some(ptr))),
            f64_to_string: m
                .declare_function("nether_rt_f64_to_string", m.fn_type(&[f64_ty], Some(ptr))),
            bool_to_string: m
                .declare_function("nether_rt_bool_to_string", m.fn_type(&[i8_ty], Some(ptr))),
            char_to_string: m.declare_function(
                "nether_rt_char_to_string",
                m.fn_type(&[m.int_type(32)], Some(ptr)),
            ),
            array_new: m.declare_function(
                "nether_rt_array_new",
                m.fn_type(&[i64_ty, i64_ty, ptr, ptr], Some(ptr)),
            ),
            array_push: m.declare_function("nether_rt_array_push", void_fn(&[ptr, ptr])),
            array_len: m.declare_function("nether_rt_array_len", m.fn_type(&[ptr], Some(i64_ty))),
            array_get: m
                .declare_function("nether_rt_array_get", m.fn_type(&[ptr, i64_ty], Some(ptr))),
            array_pop: m
                .declare_function("nether_rt_array_pop", m.fn_type(&[ptr, ptr], Some(i8_ty))),
        }
    }
}
