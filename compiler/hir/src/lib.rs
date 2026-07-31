//! # nether_hir
//!
//! Purpose: a smaller, uniform, fully-typed intermediate representation —
//! the AST after resolution/type-checking, with syntactic sugar
//! desugared, but *before* generics are substituted and *before*
//! structured control flow is flattened into a CFG (see
//! `docs/architecture/crates.md` § `compiler/hir` and
//! `docs/architecture/overview.md` §1/§4).
//!
//! Responsibilities:
//! - Desugar template-string interpolation into
//!   [`node::HirExprKind::Concat`] (wrapping non-`String` pieces in
//!   [`node::HirExprKind::ToString`]), tuple-struct/struct-literal
//!   construction into one unified [`node::HirExprKind::Construct`], and
//!   `for`-`in` loops into an index-based `while` loop over synthesized
//!   temporaries (language-spec/architecture: these are exactly the
//!   desugarings called out as HIR's job) — there is deliberately no
//!   `ForIn` node left in HIR.
//! - Resolve the language-spec §10 "leftover path segment" question
//!   (`self.name`, `dog.set_name(x)`, `Dog.new(x)`) into one of
//!   [`node::HirExprKind::Field`], [`node::HirExprKind::CallStatic`], or
//!   [`node::HirExprKind::CallGenericMethod`] — now that every
//!   expression's type is known, this is the first stage that *can*
//!   finish that disambiguation.
//! - Give every function/method a stable [`node::HirFnId`] — including
//!   one freshly lowered per concrete type that inherits (rather than
//!   overrides) an interface default method, since `self`'s meaning
//!   differs per owner even though the source `fn` body is shared.
//! - Carry a [`nether_typecheck::Type`] on every [`node::HirExpr`]
//!   directly (read from [`nether_typecheck::TypedTables`] at lowering
//!   time), and carry [`nether_typecheck::Signatures`] forward unchanged
//!   into [`node::HirModule`], since it is already the structural
//!   information `monomorphization`/`mir` need.
//! - Preserve structured control flow: `if`/`match`/`loop`/`while` remain
//!   nested [`node::HirExpr`] trees — flattening to a CFG is `mir`'s job.
//! - Still represent generic items generically: [`node::HirFunction`]
//!   keeps its own `generics` list, substituted only by
//!   `monomorphization`.
//!
//! Input: a [`nether_ast::Module`], [`nether_resolver::ResolvedNames`],
//! and [`nether_typecheck::TypedTables`] (consumed, not borrowed — see
//! [`lower::lower`]'s docs).
//!
//! Output: a [`node::HirModule`].
//!
//! Dependencies: `nether_ast`, `nether_resolver`, `nether_typecheck`.
//!
//! Consumers: `nether_monomorphization`.
//!
//! Invariants: lowering does not fail — by construction, if `typecheck`
//! succeeded, lowering to HIR always succeeds (this crate's own fallback
//! paths for "shouldn't happen" cases exist only because Rust's type
//! system requires a value on every path, not because they are expected
//! to trigger). No unresolved names, no missing types anywhere in
//! [`node::HirModule`].
//!
//! Documented simplifications (see `lower.rs` for exactly where each
//! applies):
//! - A method call on a value of still-generic type only supports the
//!   single-bound case, matching `nether_typecheck`'s own limit —
//!   [`node::HirExprKind::CallGenericMethod`] carries one bound interface,
//!   not a general constraint set.
//! - Closures carry explicit capture lists; monomorphization lifts their
//!   bodies and codegen constructs the ARC-managed environments.

mod lower;
mod node;

pub use lower::lower;
pub use node::{
    HirCapture, HirExpr, HirExprKind, HirFnId, HirFunction, HirLocalId, HirMatchArm, HirModule,
    HirParam, HirPattern, HirStmt, HirStmtKind,
};
