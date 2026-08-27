//! Per-`Type` retain/drop shims — small generated LLVM functions that
//! walk a value's statically-known heap-kind content and call into
//! `runtime/arc`, for the two places `nether_codegen` needs one but has
//! no MIR-level instruction to emit instead:
//!
//! - **A heap struct/tuple-struct's own construction** ([`Shims::own_drop_shim`]):
//!   `nether_rt_arc_alloc`'s `drop` callback, called once when that
//!   object's refcount reaches zero, with its own base address — walks
//!   its declared fields directly, releasing whichever ones are
//!   themselves heap-kind (recursing through any `Tuple`/`Enum`-typed
//!   field, since those aren't ARC-managed themselves but can still
//!   contain a heap-kind value — `arc-model.md` §1).
//! - **`Array<T>`'s element storage** ([`Shims::retain_shim`]/[`Shims::drop_shim`]):
//!   `nether_rt_array_new`'s `elem_retain`/`elem_drop` callbacks, called
//!   on every element's own slot inside the array's buffer at push
//!   (retain — the array becomes an independent owner, mirroring how a
//!   field *store* retains its new value) and at final drop (release
//!   every remaining element) — see `runtime/array`'s own module docs.
//!   `pop` deliberately calls neither: ownership transfers whole to the
//!   caller's fresh binding, matching every other call result's "arrives
//!   already owned" convention (`arc-model.md` §3.4).
//!
//! If `T` is itself heap-kind (`String`/`Array<_>`/a heap `struct`), the
//! "shim" a reference to it needs is nothing more than
//! `nether_rt_arc_retain`/`_release` directly — its slot holds a single
//! pointer, and that pointer's own further contents are that object's
//! *own* drop shim's problem, set up once at *its* construction, never
//! re-examined here (this is what keeps this walk from ever recursing
//! into a cycle, since a heap-kind boundary is exactly where Nether
//! requires an indirection for a type to be well-founded in the first
//! place). Only `Tuple`/`Enum`/stack-kind `struct` values (which aren't
//! ARC'd themselves but can still contain a heap-kind leaf) ever need an
//! actually-generated wrapper function — generated once per concrete
//! `Type` and memoized, since two `Array<Dog>`s (or two fields of the
//! same tuple shape) share one shim.
//!
//! A generated shim always takes one `ptr` parameter: the address where
//! its `Type`'s in-memory representation begins (an array element slot,
//! or — for [`Shims::own_drop_shim`] specifically — a heap object's own
//! payload address, which is exactly where its declared fields start).

use std::cell::RefCell;
use std::collections::HashMap;

use nether_llvm::{Func, IntPredicate, ModuleCx, Ty};
use nether_resolver::Definitions;
use nether_typecheck::{alloc_kind, AllocKind, Signatures, Type};

use crate::layout::Layout;
use crate::runtime::Runtime;

pub struct Shims<'ctx> {
    /// Keyed by `(Type, is_retain)` — a `Tuple`/`Enum`/stack-`struct`
    /// value nested inside a container (array element or another
    /// container's field).
    reference: RefCell<HashMap<(Type, bool), Func<'ctx>>>,
    /// Keyed by the heap struct/tuple-struct's own `Type` — its
    /// construction-time drop callback (see module docs' contrast with
    /// `reference`).
    own_drop: RefCell<HashMap<Type, Func<'ctx>>>,
    /// Retains the fields of an already byte-copied heap payload. Used by
    /// structural `Clone` after allocating a distinct outer object.
    own_retain: RefCell<HashMap<Type, Func<'ctx>>>,
    closure_drop: RefCell<HashMap<Vec<Type>, Func<'ctx>>>,
    counter: RefCell<u32>,
}

impl Default for Shims<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'ctx> Shims<'ctx> {
    pub fn new() -> Self {
        Shims {
            reference: RefCell::new(HashMap::new()),
            own_drop: RefCell::new(HashMap::new()),
            own_retain: RefCell::new(HashMap::new()),
            closure_drop: RefCell::new(HashMap::new()),
            counter: RefCell::new(0),
        }
    }

    /// A retain shim usable as `Array<ty>`'s `elem_retain` callback, or
    /// `None` if `ty` never owns any heap-kind content (nothing to
    /// retain).
    pub fn retain_shim(
        &self,
        m: &ModuleCx<'ctx>,
        layout: &Layout<'_, 'ctx>,
        runtime: &Runtime<'ctx>,
        ty: &Type,
    ) -> Option<Func<'ctx>> {
        self.reference_shim(m, layout, runtime, ty, true)
    }

    /// The drop-shim counterpart of [`Self::retain_shim`] — usable as
    /// `Array<ty>`'s `elem_drop` callback.
    pub fn drop_shim(
        &self,
        m: &ModuleCx<'ctx>,
        layout: &Layout<'_, 'ctx>,
        runtime: &Runtime<'ctx>,
        ty: &Type,
    ) -> Option<Func<'ctx>> {
        self.reference_shim(m, layout, runtime, ty, false)
    }

    fn reference_shim(
        &self,
        m: &ModuleCx<'ctx>,
        layout: &Layout<'_, 'ctx>,
        runtime: &Runtime<'ctx>,
        ty: &Type,
        retain: bool,
    ) -> Option<Func<'ctx>> {
        if !has_heap_content(ty, layout.defs, layout.sigs) {
            return None;
        }
        if let Some(&f) = self.reference.borrow().get(&(ty.clone(), retain)) {
            return Some(f);
        }
        let saved_block = m.current_block();
        let f = self.declare_shim(m, retain);
        self.reference.borrow_mut().insert((ty.clone(), retain), f);
        let entry = m.append_block(f, "entry");
        m.position_at_end(entry);
        let base = m.param(f, 0);
        let cx = ShimCx {
            m,
            layout,
            runtime,
            f,
            retain,
        };
        cx.emit_walk(base, ty);
        m.ret(None);
        m.position_at_end(saved_block);
        Some(f)
    }

    /// The `nether_rt_arc_alloc` drop callback for constructing a fresh
    /// heap-kind `ty` (a `Struct`/`TupleStruct` with `alloc_kind ==
    /// Heap`) — see module docs for why this can't just be
    /// [`Self::drop_shim`] applied to the same `ty`.
    pub fn own_drop_shim(
        &self,
        m: &ModuleCx<'ctx>,
        layout: &Layout<'_, 'ctx>,
        runtime: &Runtime<'ctx>,
        ty: &Type,
    ) -> Option<Func<'ctx>> {
        let field_tys = fields_of(ty, layout.sigs);
        if !field_tys
            .iter()
            .any(|t| has_heap_content(t, layout.defs, layout.sigs))
        {
            return None;
        }
        if let Some(&f) = self.own_drop.borrow().get(ty) {
            return Some(f);
        }
        let saved_block = m.current_block();
        let f = self.declare_shim(m, false);
        self.own_drop.borrow_mut().insert(ty.clone(), f);
        let entry = m.append_block(f, "entry");
        m.position_at_end(entry);
        let base = m.param(f, 0);
        let cx = ShimCx {
            m,
            layout,
            runtime,
            f,
            retain: false,
        };
        cx.emit_struct_fields(base, ty);
        m.ret(None);
        m.position_at_end(saved_block);
        Some(f)
    }

    /// Retain counterpart of [`Self::own_drop_shim`], walking the fields
    /// directly rather than retaining the outer object pointer. A structural
    /// clone memcpy's the payload first, then uses this to establish the new
    /// payload's independent ARC/weak ownership credits.
    pub fn own_retain_shim(
        &self,
        m: &ModuleCx<'ctx>,
        layout: &Layout<'_, 'ctx>,
        runtime: &Runtime<'ctx>,
        ty: &Type,
    ) -> Option<Func<'ctx>> {
        let field_tys = fields_of(ty, layout.sigs);
        if !field_tys
            .iter()
            .any(|t| has_heap_content(t, layout.defs, layout.sigs))
        {
            return None;
        }
        if let Some(&f) = self.own_retain.borrow().get(ty) {
            return Some(f);
        }
        let saved_block = m.current_block();
        let f = self.declare_shim(m, true);
        self.own_retain.borrow_mut().insert(ty.clone(), f);
        let entry = m.append_block(f, "entry");
        m.position_at_end(entry);
        let base = m.param(f, 0);
        let cx = ShimCx {
            m,
            layout,
            runtime,
            f,
            retain: true,
        };
        cx.emit_struct_fields(base, ty);
        m.ret(None);
        m.position_at_end(saved_block);
        Some(f)
    }

    /// Drop callback for a closure environment. Field zero is the code
    /// pointer; captures start at field one and follow `capture_tys`.
    pub fn closure_drop_shim(
        &self,
        m: &ModuleCx<'ctx>,
        layout: &Layout<'_, 'ctx>,
        runtime: &Runtime<'ctx>,
        capture_tys: &[Type],
    ) -> Option<Func<'ctx>> {
        if !capture_tys
            .iter()
            .any(|ty| has_heap_content(ty, layout.defs, layout.sigs))
        {
            return None;
        }
        if let Some(&f) = self.closure_drop.borrow().get(capture_tys) {
            return Some(f);
        }
        let saved_block = m.current_block();
        let f = self.declare_shim(m, false);
        self.closure_drop
            .borrow_mut()
            .insert(capture_tys.to_vec(), f);
        let entry = m.append_block(f, "entry");
        m.position_at_end(entry);
        let base = m.param(f, 0);
        let mut fields = vec![m.ptr_type()];
        fields.extend(capture_tys.iter().map(|ty| layout.llvm_type(ty)));
        let env_ty = m.struct_type(&fields);
        let cx = ShimCx {
            m,
            layout,
            runtime,
            f,
            retain: false,
        };
        for (index, capture_ty) in capture_tys.iter().enumerate() {
            if has_heap_content(capture_ty, layout.defs, layout.sigs) {
                let field = m.struct_gep(env_ty, base, index as u32 + 1, "capture");
                cx.emit_walk(field, capture_ty);
            }
        }
        m.ret(None);
        m.position_at_end(saved_block);
        Some(f)
    }

    fn declare_shim(&self, m: &ModuleCx<'ctx>, retain: bool) -> Func<'ctx> {
        let mut c = self.counter.borrow_mut();
        *c += 1;
        let name = format!(
            "nether_shim_{}_{}",
            if retain { "retain" } else { "drop" },
            *c
        );
        let fn_ty = m.fn_type(&[m.ptr_type()], None);
        m.declare_function(&name, fn_ty)
    }
}

/// Bundles everything a shim's body-emitting code needs, purely so the
/// recursive walk below doesn't have to thread each of these five values
/// through every call individually.
struct ShimCx<'a, 'ctx> {
    m: &'a ModuleCx<'ctx>,
    layout: &'a Layout<'a, 'ctx>,
    runtime: &'a Runtime<'ctx>,
    f: Func<'ctx>,
    retain: bool,
}

impl<'ctx> ShimCx<'_, 'ctx> {
    /// Walks a *reference* to a value of type `ty` stored at `base` — a
    /// single pointer to call `retain`/`release` on directly if `ty` is
    /// heap-kind, otherwise `ty`'s own inline structure.
    fn emit_walk(&self, base: nether_llvm::Value<'ctx>, ty: &Type) {
        let (m, layout) = (self.m, self.layout);
        if matches!(ty, Type::Unique(inner) if alloc_kind(inner, layout.defs) == AllocKind::Heap) {
            if !self.retain {
                let ptr = m.load(m.ptr_type(), base, "unique_leaf");
                m.call(self.runtime.unique_free, &[ptr], "");
            }
            return;
        }
        if let Type::Weak(_) = ty {
            // A `weak T` slot's own weak count, never its referent's
            // strong count (spec §13) — see `runtime/arc`'s own module
            // docs on why the two counts are tracked separately.
            let ptr = m.load(m.ptr_type(), base, "weak_leaf");
            let leaf_fn = if self.retain {
                self.runtime.weak_retain
            } else {
                self.runtime.weak_release
            };
            m.call(leaf_fn, &[ptr], "");
            return;
        }
        if is_heap_leaf(ty, layout.defs) {
            let ptr = m.load(m.ptr_type(), base, "leaf");
            let leaf_fn = if self.retain {
                self.runtime.retain
            } else {
                self.runtime.release
            };
            m.call(leaf_fn, &[ptr], "");
            return;
        }
        match ty {
            Type::Tuple(elems) => {
                let tuple_ty = match layout.llvm_type(ty) {
                    Ty::StructType(t) => t,
                    _ => unreachable!("Tuple always lowers to a struct type"),
                };
                for (i, elem_ty) in elems.iter().enumerate() {
                    if !has_heap_content(elem_ty, layout.defs, layout.sigs) {
                        continue;
                    }
                    let field_ptr = m.struct_gep(tuple_ty, base, i as u32, "field");
                    self.emit_walk(field_ptr, elem_ty);
                }
            }
            Type::FixedArray(element, length) => {
                let Type::Const(length) = length.as_ref() else {
                    unreachable!("fixed-array length must be concrete before shim generation")
                };
                if has_heap_content(element, layout.defs, layout.sigs) {
                    let array_ty = match layout.llvm_type(ty) {
                        Ty::StructType(t) => t,
                        _ => unreachable!("FixedArray always lowers to a struct type"),
                    };
                    for index in 0..*length as u32 {
                        let field_ptr = m.struct_gep(array_ty, base, index, "element");
                        self.emit_walk(field_ptr, element);
                    }
                }
            }
            Type::Struct(_, _) | Type::TupleStruct(_, _) => self.emit_struct_fields(base, ty),
            Type::Enum(_, _) => self.emit_enum_variants(base, ty),
            _ => {}
        }
    }

    /// Shared by [`Self::emit_walk`]'s `Struct`/`TupleStruct` case (a
    /// stack-kind struct nested in a container) and
    /// [`Shims::own_drop_shim`] (a heap-kind struct's own fields, at its
    /// own base address) — the GEP-and-recurse mechanics are identical
    /// either way, only the caller's reason for wanting them differs (see
    /// this module's doc comment).
    fn emit_struct_fields(&self, base: nether_llvm::Value<'ctx>, ty: &Type) {
        match ty {
            Type::Struct(_, _) | Type::TupleStruct(_, _) => {}
            other => unreachable!("emit_struct_fields called with non-struct type {other:?}"),
        }
        let sl = self.layout.struct_layout(ty);
        let field_tys = fields_of(ty, self.layout.sigs);
        for (i, field_ty) in field_tys.iter().enumerate() {
            if !has_heap_content(field_ty, self.layout.defs, self.layout.sigs) {
                continue;
            }
            let field_ptr = self.m.struct_gep(sl.ty, base, i as u32, "field");
            self.emit_walk(field_ptr, field_ty);
        }
    }

    fn emit_enum_variants(&self, base: nether_llvm::Value<'ctx>, enum_ty: &Type) {
        let (m, layout) = (self.m, self.layout);
        let Type::Enum(id, _) = enum_ty else {
            unreachable!()
        };
        let el = layout.enum_layout(enum_ty);
        let variants_with_content: Vec<(u32, Vec<(u32, Type)>)> =
            match layout.sigs.enum_sigs.get(id) {
                Some(sig) => sig
                    .variants
                    .iter()
                    .enumerate()
                    .filter_map(|(vi, _)| {
                        let payload = layout
                            .sigs
                            .enum_payload(enum_ty, vi as u32)
                            .unwrap_or_default();
                        let fields: Vec<(u32, Type)> = payload
                            .iter()
                            .enumerate()
                            .filter(|(_, t)| has_heap_content(t, layout.defs, layout.sigs))
                            .map(|(fi, t)| (fi as u32, t.clone()))
                            .collect();
                        if fields.is_empty() {
                            None
                        } else {
                            Some((vi as u32, fields))
                        }
                    })
                    .collect(),
                None => Vec::new(),
            };
        if variants_with_content.is_empty() {
            return;
        }
        let tag_ptr = m.struct_gep(el.ty, base, 0, "tag_ptr");
        let tag = m.load(m.int_type(64), tag_ptr, "tag");
        let merge = m.append_block(self.f, "shim_merge");
        let mut check_block = m.append_block(self.f, "shim_check");
        m.br(check_block);
        for (i, (variant, fields)) in variants_with_content.iter().enumerate() {
            m.position_at_end(check_block);
            let variant_tag = m.const_int(m.int_type(64), u64::from(*variant), false);
            let is_variant = m.int_compare(IntPredicate::EQ, tag, variant_tag, "is_variant");
            let body = m.append_block(self.f, "shim_body");
            let next = if i + 1 < variants_with_content.len() {
                m.append_block(self.f, "shim_check")
            } else {
                merge
            };
            m.cond_br(is_variant, body, next);
            m.position_at_end(body);
            for (field_idx, field_ty) in fields {
                let gep_index = *el
                    .field_offsets
                    .get(&(*variant, *field_idx))
                    .expect("valid variant/field index");
                let field_ptr = m.struct_gep(el.ty, base, gep_index, "payload_field");
                self.emit_walk(field_ptr, field_ty);
            }
            m.br(merge);
            check_block = next;
        }
        m.position_at_end(merge);
    }
}

/// Whether `ty` has any heap-kind value reachable through its own
/// structure without crossing a heap-kind boundary — i.e. whether a
/// retain/drop shim needs to do anything for it at all. A heap-kind type
/// itself counts (a single pointer to retain/release), but this does not
/// look *inside* one — see this module's doc comment.
fn has_heap_content(ty: &Type, defs: &Definitions, sigs: &Signatures) -> bool {
    match ty {
        Type::Unique(inner) => alloc_kind(inner, defs) == AllocKind::Heap,
        Type::String
        | Type::Array(_)
        | Type::Function(_, _)
        | Type::Weak(_)
        | Type::Any(_, _)
        | Type::Some(_, _) => true,
        Type::Task(_) | Type::Thread(_) => true,
        Type::Struct(_, _) | Type::TupleStruct(_, _) => match alloc_kind(ty, defs) {
            AllocKind::Heap => true,
            AllocKind::Stack => fields_of(ty, sigs)
                .iter()
                .any(|t| has_heap_content(t, defs, sigs)),
        },
        Type::Tuple(elems) => elems.iter().any(|t| has_heap_content(t, defs, sigs)),
        Type::FixedArray(element, _) => has_heap_content(element, defs, sigs),
        Type::Enum(_, _) => sigs
            .enum_sigs
            .get(match ty {
                Type::Enum(id, _) => id,
                _ => unreachable!(),
            })
            .map(|sig| {
                sig.variants.iter().enumerate().any(|(variant, _)| {
                    sigs.enum_payload(ty, variant as u32)
                        .unwrap_or_default()
                        .iter()
                        .any(|field| has_heap_content(field, defs, sigs))
                })
            })
            .unwrap_or(false),
        _ => false,
    }
}

fn is_heap_leaf(ty: &Type, defs: &Definitions) -> bool {
    match ty {
        Type::String
        | Type::Array(_)
        | Type::Function(_, _)
        | Type::Any(_, _)
        | Type::Some(_, _) => true,
        Type::Task(_) | Type::Thread(_) => true,
        Type::Struct(_, _) | Type::TupleStruct(_, _) => alloc_kind(ty, defs) == AllocKind::Heap,
        _ => false,
    }
}

fn fields_of(ty: &Type, sigs: &Signatures) -> Vec<Type> {
    sigs.type_fields(ty).unwrap_or_default()
}
