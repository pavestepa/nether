use nether_diagnostics::Span;

use crate::ids::NodeId;
use crate::symbol::Symbol;

#[derive(Debug, Clone)]
pub struct Ident {
    pub name: Symbol,
    pub span: Span,
}

impl Ident {
    pub fn new(name: impl Into<Symbol>, span: Span) -> Self {
        Ident { name: name.into(), span }
    }

    /// Whether this name is private per language-spec §4: a leading `_`.
    /// (The `private` keyword modifier is tracked separately by whichever
    /// declaration node it modifies, since it is written before the item,
    /// not part of the name itself.)
    pub fn is_underscore_private(&self) -> bool {
        self.name.as_str().starts_with('_')
    }
}

/// A dotted path — `foo`, `foo.bar`, `foo.bar.baz`.
///
/// Per language-spec §10, a path is syntactically one uniform shape whether
/// it turns out to name a module, a type, a static member, or a value/field
/// access chain; `resolver` is the stage that disambiguates, by walking
/// segments against known modules/types/values. This node carries its own
/// [`NodeId`] so `resolver` can key its `path_res` table by it regardless of
/// whether the path appears in expression or type position.
#[derive(Debug, Clone)]
pub struct Path {
    pub id: NodeId,
    pub segments: Vec<Ident>,
    pub span: Span,
}

impl Path {
    pub fn single(id: NodeId, ident: Ident) -> Self {
        let span = ident.span;
        Path { id, segments: vec![ident], span }
    }
}
