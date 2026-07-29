/// A literal value as written in source. Integer literals carry no written
/// suffix (language-spec §2.3 — the concrete integer type is inferred from
/// context, not annotated at the literal).
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Int(u128),
    Float(f64),
    Bool(bool),
    Char(char),
    /// A plain `"..."` string — never scanned for interpolation. Contrast
    /// with [`crate::expr::ExprKind::StringTemplate`], the backtick form.
    Str(String),
}
