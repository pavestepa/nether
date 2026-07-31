//! # nether_ast
//!
//! Purpose: the syntactic data model for a parsed Nether program — what
//! source looks like structurally, before any name resolution or type
//! checking (see `docs/architecture/crates.md` § `compiler/ast`).
//!
//! Responsibilities:
//! - Node types for items ([`item::Item`]), statements/expressions
//!   ([`expr::Expr`]), patterns ([`pattern::Pattern`]), and as-written type
//!   annotations ([`ty::TypeExpr`]).
//! - Every node carries a [`Span`] from `nether_diagnostics`.
//! - The two string literal forms from language-spec §2.3 are distinct
//!   node shapes: [`literal::Literal::Str`] (plain) vs.
//!   [`expr::ExprKind::StringTemplate`] (backtick, interpolating).
//! - Module/static/member access is one generic dotted-path node
//!   ([`ident::Path`]) — this crate does not decide what a path segment
//!   means (language-spec §10); that is `resolver`'s job.
//!
//! Input: none — this crate defines data only.
//!
//! Output: the types `parser` builds and every downstream crate consumes,
//! directly (`resolver`, `typecheck`) or by reference (`hir`, which defines
//! its own separate node types rather than mutating these in place).
//!
//! Dependencies: `nether_diagnostics` (for [`Span`]) only.
//!
//! Invariants: nodes are immutable once built; `resolver`/`typecheck`
//! record results in side tables keyed by [`NodeId`], never by mutating a
//! node in place, so the AST stays a stable reference for diagnostics even
//! after later passes have run. No node type here encodes a resolved or
//! checked fact — that is what HIR is for.
//!
//! Future extension points: new expression/item kinds are added as new
//! enum variants; every `match` elsewhere over [`expr::ExprKind`] /
//! [`item::Item`] is expected to be exhaustive, so the compiler fails to
//! build until each pass handles the addition.

mod expr;
mod ident;
mod ids;
mod item;
mod literal;
mod pattern;
mod symbol;
mod ty;

pub use nether_diagnostics::{FileId, Span};

pub use expr::{BinaryOp, UnaryOp};
pub use expr::{Block, Expr, ExprKind, FieldAccessor, LetStmt, MatchArm, Stmt, TemplatePart};
pub use ident::{Ident, Path};
pub use ids::{NodeId, NodeIdGen};
pub use item::{
    EnumDecl, EnumVariant, Field, FnDecl, GenericParam, ImplBlock, InterfaceDecl, Item, ModDecl,
    Module, Param, SelfParam, TypeDecl, TypeDeclKind, UseDecl,
};
pub use literal::Literal;
pub use pattern::Pattern;
pub use symbol::Symbol;
pub use ty::TypeExpr;
