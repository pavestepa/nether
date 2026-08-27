//! # nether_lexer
//!
//! Purpose: source text → token stream (see
//! `docs/architecture/crates.md` § `compiler/lexer`).
//!
//! Responsibilities: recognize every token kind from language-spec §2
//! (identifiers/keywords, numeric/char/string/template-string literals,
//! punctuation, `//`/`///`/`/* */` comments), track byte offsets for
//! [`Span`] construction, and report lexical errors through
//! `nether_diagnostics`. This crate owns no AST types — see `nether_ast`
//! for those.
//!
//! Input: raw source text plus a [`FileId`] from a
//! [`nether_diagnostics::SourceMap`].
//!
//! Output: a flat `Vec<SpannedToken>` plus any lexical diagnostics.
//!
//! Dependencies: `nether_diagnostics` only.
//!
//! Consumers: `nether_parser`.
//!
//! Invariants: [`tokenize`] never fails outright — an invalid character
//! produces a diagnostic plus a [`Token::Error`] recovery token, so a
//! caller always gets a full token stream back. Template-string
//! interpolation splitting happens here, not in the parser: each `${...}`
//! segment is handed onward as unparsed source text
//! ([`TemplatePartTok::Expr`]) for the parser to re-tokenize and parse with
//! its normal expression entry point.

mod lexer;
#[cfg(test)]
mod lexer_tests;
mod token;

pub use lexer::tokenize;
pub use token::{keyword_from_str, Keyword, Punct, SpannedToken, TemplatePartTok, Token};
