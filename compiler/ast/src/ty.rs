use nether_diagnostics::Span;

use crate::ident::Path;

/// A type as written in source — not yet resolved to a `type-system.md`
/// `Type`. Resolution (does `Dog` name a struct? an enum? a generic
/// parameter?) is `resolver`/`typecheck`'s job; this crate only records
/// what the programmer wrote.
#[derive(Debug, Clone)]
pub enum TypeExpr {
    /// `Dog`, `Array<T>`, `user.User`.
    Named { path: Path, generics: Vec<TypeExpr>, span: Span },
    /// `(i32, String)`.
    Tuple(Vec<TypeExpr>, Span),
    /// `[i32]` — sugar the parser desugars to this node; see
    /// language-spec.md §2.3/§3.5 for `Array`'s built-in role.
    Array(Box<TypeExpr>, Span),
    /// `weak T`.
    Weak(Box<TypeExpr>, Span),
    /// A function-typed value, e.g. a closure's inferred/annotated shape.
    Function { params: Vec<TypeExpr>, ret: Box<TypeExpr>, span: Span },
}

impl TypeExpr {
    pub fn span(&self) -> Span {
        match self {
            TypeExpr::Named { span, .. }
            | TypeExpr::Tuple(_, span)
            | TypeExpr::Array(_, span)
            | TypeExpr::Weak(_, span)
            | TypeExpr::Function { span, .. } => *span,
        }
    }
}
