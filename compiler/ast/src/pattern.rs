use nether_diagnostics::Span;

use crate::ident::{Ident, Path};
use crate::ids::NodeId;
use crate::literal::Literal;

/// A `match` pattern (language-spec §9). No guards in the MVP — a
/// [`crate::expr::MatchArm`] is `pattern => body` only.
#[derive(Debug, Clone)]
pub enum Pattern {
    Wildcard(Span),
    /// A binding introduces a new local — the [`NodeId`] identifies this
    /// binding site for `resolver`, the same way [`crate::expr::LetStmt`]'s
    /// `id` does.
    Binding(NodeId, Ident),
    Literal(Literal, Span),
    Tuple(Vec<Pattern>, Span),
    /// `Color.Custom(name)`, `Option.Some(x)`.
    Variant {
        path: Path,
        payload: Vec<Pattern>,
        span: Span,
    },
}

impl Pattern {
    pub fn span(&self) -> Span {
        match self {
            Pattern::Wildcard(span) => *span,
            Pattern::Binding(_, ident) => ident.span,
            Pattern::Literal(_, span) => *span,
            Pattern::Tuple(_, span) => *span,
            Pattern::Variant { span, .. } => *span,
        }
    }
}
