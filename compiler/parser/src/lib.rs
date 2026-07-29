//! # nether_parser
//!
//! Purpose: token stream → [`nether_ast`] tree (see
//! `docs/architecture/crates.md` § `compiler/parser`).
//!
//! Responsibilities: recursive-descent parsing of items/statements/
//! patterns/types, Pratt (operator-precedence) parsing of expressions,
//! syntactic-only well-formedness checks (e.g. `mut self` must be the
//! first parameter), and error recovery (skip to the next recognizable
//! item keyword on a malformed item, so one syntax error doesn't hide the
//! rest).
//!
//! Input: [`nether_lexer::SpannedToken`]s.
//!
//! Output: an [`nether_ast::Module`] plus any syntax diagnostics —
//! [`parse_module`] is total: it always returns a `Module`, even from
//! malformed input, paired with diagnostics describing what went wrong.
//!
//! Dependencies: `nether_ast`, `nether_lexer`, `nether_diagnostics`.
//!
//! Consumers: `nether_driver`, with the resulting AST then consumed by
//! `nether_resolver`.
//!
//! Invariants: parsing syntactically valid input never produces
//! diagnostics. Module/static/member-access dotted paths are parsed into
//! one generic [`nether_ast::Path`] node regardless of what they'll turn
//! out to mean (language-spec §10) — disambiguating a path into
//! "module segment" vs. "value access" is `resolver`'s job, not this
//! crate's.
//!
//! Future extension points: new grammar (macros, const generics) adds new
//! parse rules producing new `nether_ast` variants; the binary-operator
//! precedence table in `expr.rs` is the extension point for new operators.

mod expr;
mod item;
mod parser;
mod pattern;
mod template;
mod ty;

use nether_ast::{FileId, Module};
use nether_diagnostics::Diagnostic;

use crate::parser::Parser;

/// Parses one source file into a [`Module`].
///
/// Runs the lexer internally (so callers only need a [`SourceMap`]-issued
/// [`FileId`] and the file's text), then parses the resulting token stream.
/// Always returns a `Module` — on malformed input it is simply missing the
/// items that couldn't be parsed, and the returned diagnostics explain why.
///
/// [`SourceMap`]: nether_diagnostics::SourceMap
pub fn parse_module(source: &str, file: FileId) -> (Module, Vec<Diagnostic>) {
    let (module, diagnostics, _) = parse_module_with_node_id_start(source, file, 0);
    (module, diagnostics)
}

/// Parses a module using a caller-provided first NodeId and returns the
/// next unused value. The multi-file driver uses this to keep side-table
/// keys globally unique across every source file in one compilation.
pub fn parse_module_with_node_id_start(
    source: &str,
    file: FileId,
    start: u32,
) -> (Module, Vec<Diagnostic>, u32) {
    let (tokens, lex_diags) = nether_lexer::tokenize(source, file);
    let mut parser = Parser::with_node_id_start(tokens, file, start);
    parser.diagnostics.extend(lex_diags);
    let items = parser.parse_items_until_eof();
    let next = parser.ids.next_value();
    (
        Module {
            file,
            items,
            imports: std::collections::HashMap::new(),
        },
        parser.diagnostics,
        next,
    )
}
