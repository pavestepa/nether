use nether_ast::Expr;
use nether_diagnostics::{Diagnostic, Span};
use nether_lexer::SpannedToken;

use crate::parser::Parser;

impl Parser {
    /// Re-parses one `${...}` interpolation segment of a template string.
    ///
    /// The lexer already extracted `text` as raw source and `span` as its
    /// absolute byte range in the original file (language-spec §2.3;
    /// `nether_lexer`'s module docs). This method re-tokenizes `text` in
    /// isolation — which produces token spans relative to `text`, i.e.
    /// starting at 0 — then shifts every one of those spans by
    /// `span.start` so diagnostics and (eventually) `typecheck` results
    /// for the interpolated expression point at the right place in the
    /// real file.
    ///
    /// Implementation note: rather than constructing a second `Parser`
    /// (which would need its own `NodeIdGen`/diagnostics, defeating the
    /// point of sharing them), this temporarily swaps `self.tokens`/
    /// `self.pos` for the shifted sub-token-stream, parses one expression
    /// from it, and restores the saved buffer/position. This is safe
    /// because the swap is strictly nested and sequential — Nether has no
    /// concurrency, and nothing else observes `self.tokens` mid-swap.
    pub(crate) fn reparse_template_expr(&mut self, text: &str, span: Span) -> Expr {
        let (raw_tokens, lex_diags) = nether_lexer::tokenize(text, self.file);
        self.diagnostics
            .extend(shift_diagnostics(lex_diags, span.start));
        let shifted: Vec<SpannedToken> = raw_tokens
            .into_iter()
            .map(|t| SpannedToken {
                token: t.token,
                span: shift_span(t.span, span.start),
            })
            .collect();

        let saved_tokens = std::mem::replace(&mut self.tokens, shifted);
        let saved_pos = std::mem::replace(&mut self.pos, 0);
        let saved_struct_lit = self.struct_lit_allowed;
        self.struct_lit_allowed = true;

        let expr = self.parse_assign_expr();
        if !self.is_eof() {
            let bad_span = self.peek_span();
            self.error(
                bad_span,
                "unexpected trailing tokens in this string interpolation",
            );
        }

        self.tokens = saved_tokens;
        self.pos = saved_pos;
        self.struct_lit_allowed = saved_struct_lit;
        expr
    }
}

fn shift_span(span: Span, offset: u32) -> Span {
    Span::new(span.file, span.start + offset, span.end + offset)
}

fn shift_diagnostics(diags: Vec<Diagnostic>, offset: u32) -> Vec<Diagnostic> {
    diags
        .into_iter()
        .map(|mut d| {
            for label in &mut d.labels {
                label.span = shift_span(label.span, offset);
            }
            for suggestion in &mut d.suggestions {
                suggestion.span = shift_span(suggestion.span, offset);
            }
            d
        })
        .collect()
}
