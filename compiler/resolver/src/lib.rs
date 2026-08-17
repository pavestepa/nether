//! # nether_resolver
//!
//! Purpose: name resolution — bind every identifier and dotted [`Path`] in
//! a parsed [`nether_ast::Module`] to a concrete declaration, or report an
//! unresolved-name diagnostic (see `docs/architecture/crates.md` §
//! `compiler/resolver`).
//!
//! Responsibilities:
//! - Build the top-level namespace ([`def::Definitions`]): every
//!   primitive, built-in (`String`, `Array`, `println`, `print`), the
//!   bundled prelude (`Option`/`Result` included — ordinary `enum`
//!   declarations in `stdlib/`, not builtins; see
//!   [`def::Definitions::promote_to_prelude`]), and user-declared
//!   `type`/`enum`/`trait`/`fn`, with `impl` block method names
//!   merged into their target's entry.
//! - Build nested local scopes for function/method/closure bodies —
//!   `self`, parameters, `let` bindings, and pattern bindings (match arms,
//!   `for`) — with inner bindings shadowing outer ones of the same name.
//! - Resolve every [`Path`], in both expression and type position,
//!   disambiguating it (language-spec §10) into a local, a bare
//!   definition, an enum variant, or a static member — the whole point of
//!   the generic dotted-path node `nether_parser` produces instead of
//!   deciding this itself.
//!
//! Input: a [`nether_ast::Module`].
//!
//! Output: [`ResolvedNames`] (a [`def::Definitions`] table, a
//! `path_res` side table keyed by each [`Path`]'s own
//! [`nether_ast::NodeId`], and a `locals` side table keyed by each binding
//! site's `NodeId`) plus diagnostics.
//!
//! Dependencies: `nether_ast`, `nether_diagnostics`.
//!
//! Consumers: `nether_typecheck`, `nether_hir`, and `nether_driver`.
//!
//! Invariants: after a successful resolve (zero error diagnostics), every
//! [`Path`] node in the module has an entry in `path_res`.
//!
//! Module-aware resolution uses each node's `FileId`: local declarations
//! and imported aliases live in separate per-file namespaces, while every
//! definition still receives one program-wide `DefId`.
//!
//! Deliberate boundary:
//! - Field existence (`self.name` once `name` is looked past `self`) is
//!   deliberately left to `typecheck`, the first stage with the type
//!   information needed to check it — this crate only resolves the
//!   leading segment(s) it can determine from names alone (a local, or a
//!   type/enum's own static members and variants).

mod def;
mod resolve;
mod scope;

pub use def::{Def, DefId, DefKind, Definitions};
pub use resolve::{resolve, resolve_with_prelude, PathResolution, Resolution, ResolvedNames};
pub use scope::LocalId;
