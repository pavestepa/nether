use nether_diagnostics::Span;

use crate::ident::Path;

/// A type as written in source — not yet resolved to a `type-system.md`
/// `Type`. Resolution (does `Dog` name a struct? an enum? a generic
/// parameter?) is `resolver`/`typecheck`'s job; this crate only records
/// what the programmer wrote.
#[derive(Debug, Clone)]
pub enum TypeExpr {
    /// A numeric const-generic argument such as `Buffer<16>`.
    Const(u128, Span),
    /// `Dog`, `Array<T>`, `user.User`.
    Named {
        path: Path,
        generics: Vec<TypeExpr>,
        span: Span,
    },
    /// `(i32, String)`.
    Tuple(Vec<TypeExpr>, Span),
    /// `[i32]` — sugar the parser desugars to this node; see
    /// language-spec.md §2.3/§3.5 for `Array`'s built-in role.
    Array(Box<TypeExpr>, Span),
    /// `{T, N}` — an inline fixed-size array type.
    FixedArray {
        element: Box<TypeExpr>,
        length: Box<TypeExpr>,
        span: Span,
    },
    /// `weak T`.
    Weak(Box<TypeExpr>, Span),
    /// `any Trait` — an existential trait value.
    Any(Box<TypeExpr>, Span),
    /// `some Trait` — an opaque result type.
    Some(Box<TypeExpr>, Span),
    /// `:T` — the uniquely-owned form of a heap/reference-category type, or
    /// `:t` for the uniquely-owned form of an inline-category type
    /// (language-spec §3). The leading `:` is part of the type expression,
    /// not generic punctuation; this node is what carries it structurally
    /// rather than as a string. `Ref`/`MutRef` below are only ever reached
    /// while parsing *inside* a `Unique` type (there is no bare, always-ARC
    /// reference form — language-spec §3.1), so they do not themselves
    /// nest inside another `Unique`.
    Unique(Box<TypeExpr>, Span),
    /// `:&T` — a shared borrow, always within the unique-ownership domain
    /// (language-spec §3.1). Nests structurally for deeper chains
    /// (`:&&T`), never as a string.
    Ref(Box<TypeExpr>, Span),
    /// `:&mut T` — an exclusive borrow, always within the unique-ownership
    /// domain (language-spec §3.1).
    MutRef(Box<TypeExpr>, Span),
    /// `*const T` — a raw, unchecked pointer (language-spec §17, Stage 5).
    /// Unlike `Ref`/`MutRef`, this has no unique-ownership-domain
    /// restriction — it appears bare (`*const T`) as freely as within a
    /// `Unique` wrapper (`:*const T`), since a raw pointer carries no
    /// borrow-tracking to interact with the domain split in the first
    /// place.
    RawConstPtr(Box<TypeExpr>, Span),
    /// `*mut T` — a raw, unchecked mutable pointer (language-spec §17,
    /// Stage 5). See `RawConstPtr`'s own docs.
    RawMutPtr(Box<TypeExpr>, Span),
    /// A function-typed value, e.g. a closure's inferred/annotated shape.
    Function {
        params: Vec<TypeExpr>,
        ret: Box<TypeExpr>,
        span: Span,
    },
}

impl TypeExpr {
    pub fn span(&self) -> Span {
        match self {
            TypeExpr::Const(_, span)
            | TypeExpr::Named { span, .. }
            | TypeExpr::Tuple(_, span)
            | TypeExpr::Array(_, span)
            | TypeExpr::FixedArray { span, .. }
            | TypeExpr::Weak(_, span)
            | TypeExpr::Any(_, span)
            | TypeExpr::Some(_, span)
            | TypeExpr::Unique(_, span)
            | TypeExpr::Ref(_, span)
            | TypeExpr::MutRef(_, span)
            | TypeExpr::RawConstPtr(_, span)
            | TypeExpr::RawMutPtr(_, span)
            | TypeExpr::Function { span, .. } => *span,
        }
    }
}
