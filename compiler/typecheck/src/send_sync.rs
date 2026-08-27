//! Compiler-derived `Send`/`Sync` (language-spec §19, Stage 6) —
//! structural, auto-derived predicates over a fully monomorphized
//! [`Type`], gating what a `thread.spawn(move () => { ... })` closure
//! may capture (`nether_typecheck::check`'s own call-site checking, the
//! only consumer).
//!
//! Unlike `Eq`/`Hash`/`Clone` (explicit opt-in, checked *downward* from a
//! source-level trait declaration — [`crate::sig::Signatures::can_derive_eq`]
//! and neighbors), `Send`/`Sync` are computed with the opposite polarity:
//! true by default, false only when a field/variant makes it unsafe,
//! overridable in either direction only through an explicit `unsafe impl
//! TypeName Send { }`/`unsafe impl TypeName Sync { }` — an unverifiable
//! promise the compiler cannot check for itself, so it can only be made
//! through `unsafe` ([`crate::sig::Signatures::declares_send`]/
//! [`Signatures::declares_sync`]).
//!
//! # Why `Sync` is far stricter than `Send`
//!
//! Both auto-traits mirror `std::sync::Arc`/`Box`'s real bounds
//! (`Arc<T>: Send` needs `T: Send + Sync`, since other clones may remain
//! live on the origin thread even after one clone moves; `Box<T>: Send`
//! needs only `T: Send`, single ownership). But `Sync` — "is `&T`/a
//! shared handle safe to use concurrently from two threads" — depends on
//! whether the type can ever be *mutated through an aliased handle*, and
//! Nether's ARC domain has no equivalent of Rust's own `&`/`&mut`
//! exclusivity discipline protecting that for nominal types: an ordinary
//! `T` (never `:T`) can have any number of live handles, and any one of
//! them can mutate a field in place (`mut self` methods, `d.field =
//! value` on a `mut` binding) with no cross-alias exclusivity enforced —
//! unlike `std::sync::Arc<T>`, which only ever exposes `&T` and needs an
//! explicit `Mutex`/`Cell` for interior mutability. Proving a given
//! nominal type is *never* mutated anywhere in the program would need
//! real whole-program analysis — exactly what this design deliberately
//! avoids (the resolved design question: "conservative structural, no
//! whole-program mutation analysis"). So every nominal, ARC-domain type
//! (`Struct`/`TupleStruct`/`Enum`, and the builtin heap-backed `Array`)
//! is `Sync` **only** via `unsafe impl` — never auto-derived, regardless
//! of its fields. Purely structural, non-nominal composites (`Tuple`,
//! fixed-size arrays) have no identity/aliasing of their own — sharing
//! one is exactly as safe as sharing its elements individually — so they
//! recurse structurally instead. `String` is a base case despite being
//! heap-represented: `runtime/string`'s entire API is create-only
//! (`concat`/`from_utf8`/`hash` all produce a *new* allocation, never
//! mutate an existing one), so it carries no interior-mutability hazard
//! at all.
//!
//! Tested through real Nether source (`compiler/typecheck/tests/`), not
//! hand-built fixtures here — `nether_resolver::Definitions`'s
//! constructors are crate-private, and every other test in this crate
//! already goes through the real `parse -> resolve -> check` pipeline;
//! `thread.spawn` (Stage 6, alongside this module) is this predicate's
//! only real caller and its own tests exercise the table below.

use nether_resolver::Definitions;

use crate::sig::Signatures;
use crate::ty::Type;

/// Whether a value of `ty` may be moved into a `thread.spawn` closure —
/// used, then owned solely by the new thread, with no further access
/// from the spawning thread to *that* handle. Still unsound to grant
/// unconditionally for an ARC-domain type: other live handles to the
/// same object may remain on the origin thread, so those require the
/// payload to also be [`is_sync`] (mirrors `Arc<T>: Send`).
pub fn is_send(ty: &Type, defs: &Definitions, sigs: &Signatures) -> bool {
    match ty {
        Type::Primitive(_) | Type::Const(_) | Type::String | Type::Never | Type::Error => true,
        Type::Tuple(items) => items.iter().all(|item| is_send(item, defs, sigs)),
        Type::FixedArray(elem, _) => is_send(elem, defs, sigs),
        Type::Struct(..) | Type::TupleStruct(..) | Type::Enum(..) | Type::Array(_) => {
            sigs.declares_send(ty, defs)
                || nominal_fields_all(ty, sigs, |field, sigs| {
                    is_send(field, defs, sigs) && is_sync(field, defs, sigs)
                })
        }
        Type::Unique(inner) => sigs.declares_send(ty, defs) || is_send(inner, defs, sigs),
        Type::Weak(inner) => is_send(inner, defs, sigs) && is_sync(inner, defs, sigs),
        // `Thread<T>` mirrors `std::thread::JoinHandle<T>: Send where T:
        // Send` — a spawned thread's own handle is safe to hand to yet
        // another thread to `.join()` from there, unlike `Task<T>`
        // (permanently tied to the single Tokio `current_thread` runtime
        // it was polled on, per `runtime/task`'s own invariant — never
        // `Send`).
        Type::Thread(output) => is_send(output, defs, sigs),
        Type::Ref(_)
        | Type::MutRef(_)
        | Type::RawConstPtr(_)
        | Type::RawMutPtr(_)
        | Type::Function(_, _)
        | Type::Any(_, _)
        | Type::Some(_, _)
        | Type::Task(_) => false,
        Type::Generic(_) | Type::Associated(_, _) | Type::Trait(_) => {
            unreachable!("{ty:?} never appears as a captured value's type by the time Send/Sync is checked (post-monomorphization only)")
        }
    }
}

/// Whether a shared handle to `ty` (an aliased ARC reference, or a
/// `:&T` borrow) may safely be used concurrently from two different
/// threads at once. See this module's own docs for why every nominal
/// type needs an explicit `unsafe impl ... Sync` — this is deliberately
/// far stricter than [`is_send`].
pub fn is_sync(ty: &Type, defs: &Definitions, sigs: &Signatures) -> bool {
    match ty {
        Type::Primitive(_) | Type::Const(_) | Type::String | Type::Never | Type::Error => true,
        Type::Tuple(items) => items.iter().all(|item| is_sync(item, defs, sigs)),
        Type::FixedArray(elem, _) => is_sync(elem, defs, sigs),
        Type::Struct(..) | Type::TupleStruct(..) | Type::Enum(..) | Type::Array(_) => {
            sigs.declares_sync(ty, defs)
        }
        Type::Unique(inner) => sigs.declares_sync(ty, defs) || is_sync(inner, defs, sigs),
        Type::Weak(inner) => is_send(inner, defs, sigs) && is_sync(inner, defs, sigs),
        // `.join(self)` is `Thread<T>`'s only operation and consumes it —
        // there is nothing useful a shared `&Thread<T>` could do, so this
        // costs nothing in practice; kept `false` rather than modeled as
        // vacuously safe, matching this module's own conservative bias.
        Type::Thread(_) => false,
        Type::Ref(_)
        | Type::MutRef(_)
        | Type::RawConstPtr(_)
        | Type::RawMutPtr(_)
        | Type::Function(_, _)
        | Type::Any(_, _)
        | Type::Some(_, _)
        | Type::Task(_) => false,
        Type::Generic(_) | Type::Associated(_, _) | Type::Trait(_) => {
            unreachable!("{ty:?} never appears as a captured value's type by the time Send/Sync is checked (post-monomorphization only)")
        }
    }
}

/// Applies `field_ok` to every field/variant-payload type of a nominal
/// `Struct`/`TupleStruct`/`Enum`, or the element type of an `Array` —
/// `false` if any field fails, or if `ty`'s own shape can't be resolved
/// (an unknown owner, defensively treated as not-Send rather than
/// panicking).
fn nominal_fields_all(
    ty: &Type,
    sigs: &Signatures,
    field_ok: impl Fn(&Type, &Signatures) -> bool,
) -> bool {
    match ty {
        Type::Struct(..) | Type::TupleStruct(..) => sigs
            .type_fields(ty)
            .is_some_and(|fields| fields.iter().all(|field| field_ok(field, sigs))),
        Type::Enum(id, _) => sigs.enum_sigs.get(id).is_some_and(|enum_sig| {
            (0..enum_sig.variants.len() as u32).all(|variant| {
                sigs.enum_payload(ty, variant)
                    .unwrap_or_default()
                    .iter()
                    .all(|field| field_ok(field, sigs))
            })
        }),
        Type::Array(elem) => field_ok(elem, sigs),
        _ => false,
    }
}
