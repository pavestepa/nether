//! # nether_mir
//!
//! Purpose: lower structured, monomorphized HIR into an explicit control-
//! flow graph with a small, fixed instruction set, and run the ARC
//! insertion pass over it (`docs/architecture/crates.md` §
//! `compiler/mir`, `docs/architecture/arc-model.md`).
//!
//! Responsibilities:
//! - [`build_mir`] lowers `if`/`match`/`while`/`loop`/`return`/`break`/
//!   `continue` into basic blocks with explicit `Goto`/`Branch`/`Return`
//!   terminators (`match` compiles to a chain of discriminant/literal
//!   tests, not a jump table — see [`node::Terminator`]'s docs for why
//!   that's this crate's own simplification of `crates.md`'s looser
//!   `Switch`-terminator sketch).
//! - Every [`node::LocalDecl`] carries its heap-vs-stack classification
//!   (`nether_typecheck::alloc_kind`, read once at declaration time) and
//!   its declared mutability.
//! - [`insert_arc`] is the mandatory second pass: every heap-kind local
//!   gets exactly the `Retain`/`Release` instructions `arc-model.md`
//!   specifies — bind-time retain (§3.1), call-argument passing (§3.3),
//!   and scope exit (§3.2, including RVO, §3.4) — before this crate's
//!   output is handed to `codegen`. See `build.rs`'s module docs for
//!   which of these each pass actually implements and why (a deliberate,
//!   documented split from `crates.md`'s "build_mir produces zero
//!   Retain/Release" sketch).
//!
//! Input: [`nether_monomorphization::MonoModule`], plus
//! `nether_resolver::Definitions` and `nether_typecheck::Signatures`
//! (needed for `self`'s type and enum variant payload types respectively
//! — not carried forward by `MonoModule` itself).
//!
//! Output: `Vec<node::MirFunction>`, indexed by the same
//! `nether_monomorphization::MonoFnId` every `node::CallTarget::Fn` here
//! refers to (this crate mints no separate function-id space).
//!
//! Dependencies: `nether_ast`, `nether_resolver`, `nether_typecheck`,
//! `nether_hir` (for `HirLocalId`/`HirPattern`, reused directly — see
//! `node.rs`), `nether_monomorphization`.
//!
//! Consumers: `codegen`.
//!
//! Invariants: every [`node::BasicBlock`] has exactly one terminator
//! (`build_mir` defaults any block nothing else claims to
//! `node::Terminator::Unreachable` — see `build.rs`'s
//! `FnBuilder::terminate_current`). After [`insert_arc`] runs, every
//! heap-kind local has a statically balanced number of retains/releases
//! across every path through the CFG, within the simplifications
//! documented below.
//!
//! Documented simplifications:
//! - `match` lowers to a linear chain of per-arm `Branch`es, not a jump
//!   table — correct, but not the constant-time dispatch a real `Switch`
//!   terminator would give a large enum; a future optimization pass could
//!   detect an all-`Variant`-pattern arm list and rewrite it into one.
//! - A field/index *store* (`self.name = new_name;`) retaining the new
//!   value and releasing the old one is this crate's own extension of
//!   §3.1's reasoning — `arc-model.md`'s worked examples only cover fresh
//!   `let` bindings, not mutation through an existing place. See
//!   `FnBuilder::lower_assign`'s docs.
//! - Closure environments and weak upgrades are explicit MIR operations;
//!   their ownership is handled by the same temporary/scope cleanup rules
//!   as other managed values.

mod arc;
mod build;
mod node;

pub use arc::insert_arc;
pub use build::build_mir;
pub use node::{
    BasicBlock, BlockId, CallTarget, Instr, Local, LocalDecl, MirFunction, Operand, Place,
    Projection, Rvalue, Terminator,
};
