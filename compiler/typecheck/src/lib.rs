//! # nether_typecheck
//!
//! Purpose: assign a structured [`ty::Type`] to every expression and
//! declaration, and enforce every static rule in the language spec that
//! isn't purely syntactic or name-resolution (see
//! `docs/architecture/crates.md` § `compiler/typecheck`).
//!
//! Responsibilities:
//! - Build [`sig::Signatures`]: struct/tuple-struct/unit field shapes,
//!   enum variant payloads, standalone-`fn` and `impl`/`trait` method
//!   signatures — including inheriting trait default methods an
//!   `impl` doesn't override, and reporting a missing required method.
//! - Lower every [`nether_ast::TypeExpr`] to a [`ty::Type`], reusing
//!   `resolver`'s already-computed path resolutions rather than
//!   re-resolving names.
//! - Type-check every function/method body: `let` inference,
//!   struct-literal/tuple-struct construction, field/method access
//!   (including the language-spec §10 leftover-path-segment
//!   disambiguation that only becomes possible once a base's type is
//!   known), operators, `if`/`match`/loops, `match` exhaustiveness over
//!   enum variants, `mut`-parameter/argument agreement (language-spec
//!   §5.1), `weak T` only wrapping a heap type (§3.4), and trait
//!   bound satisfaction at generic call sites where the concrete type
//!   argument can be inferred from the arguments.
//!
//! Input: a [`nether_ast::Module`] plus [`nether_resolver::ResolvedNames`].
//!
//! Output: [`check::TypedTables`] (`expr_types` keyed by each
//! [`nether_ast::Expr`]'s own `id`, plus [`sig::Signatures`]) and
//! diagnostics.
//!
//! Dependencies: `nether_ast`, `nether_diagnostics`, `nether_resolver`.
//!
//! Consumers: `nether_hir`, `nether_mir`, and `nether_codegen`.
//!
//! Invariants: after a successful check (zero error diagnostics), every
//! expression node has an entry in `expr_types`.
//!
//! Documented simplifications:
//! - No full unification-based inference: numeric literals default to
//!   `i32`/`f64` absent a more specific expected type. Generic inference
//!   structurally matches receiver/parameter/argument types and uses the
//!   expected return type, but remains local to one call.
//! - `loop`'s own type is always `()` — a `break value` inside a `loop` is
//!   type-checked but not unified into the loop expression's result type.
//! - The source grammar permits one trait bound per generic
//!   parameter; there are no where-clauses, associated types, blanket
//!   implementations, or specialization.

mod alloc;
mod check;
mod sig;
mod ty;

pub use alloc::{alloc_kind, AllocKind};
pub use check::{check, TypedTables};
pub use sig::{
    EnumSig, FnSig, GenericBound, ParamSig, ReceiverDomain, ReturnOrigin, Signatures, TypeShape,
};
pub use ty::{PrimitiveKind, Type};

// Re-exported so `sig::Signatures`'s public methods can be used without
// requiring a direct `nether_resolver` dependency for the common case.
pub use nether_resolver::DefId;
