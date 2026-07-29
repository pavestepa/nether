//! # nether_diagnostics
//!
//! Purpose: the shared error-reporting substrate used by every compiler
//! stage from `lexer` through `typecheck` (see
//! `docs/architecture/crates.md` § `compiler/diagnostics`).
//!
//! Responsibilities:
//! - [`SourceMap`] owns loaded source files and maps byte offsets to
//!   line/column for rendering.
//! - [`Span`] is the only source-location representation any other crate
//!   should carry; it is a plain `(FileId, start, end)` byte-offset range.
//! - [`Diagnostic`] carries a severity, a primary message, [`Label`]s
//!   (span-anchored notes), free-text hints, and machine-applicable
//!   [`Suggestion`]s.
//! - [`render`] turns a [`Diagnostic`] plus a [`SourceMap`] into
//!   rustc-style human-readable output.
//!
//! Input: diagnostics are constructed by upstream crates from [`Span`]s;
//! [`SourceMap`] is populated by the driver as it loads files.
//!
//! Output: rendered diagnostic text, or the structured [`Diagnostic`]
//! values themselves for a future machine-readable output mode.
//!
//! Dependencies: none — this is a leaf crate, alongside `ast`.
//!
//! Invariants: constructing a [`Diagnostic`] never touches the filesystem
//! and never panics; rendering (which needs a [`SourceMap`]) is a
//! deliberately separate step so any crate can build a [`Diagnostic`] from
//! just [`Span`]s without holding a `SourceMap` reference.
//!
//! Future extension points: a machine-readable (e.g. JSON) output format,
//! `#[allow]`-style suppression once attributes exist, LSP-shaped
//! diagnostic export.

mod diagnostic;
mod render;
mod source_map;

pub use diagnostic::{Diagnostic, Label, Severity, Suggestion};
pub use render::render;
pub use source_map::{FileId, LineCol, SourceMap, Span};
