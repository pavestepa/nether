use std::fmt;
use std::rc::Rc;

/// An identifier name.
///
/// MVP note: backed by a cheaply-cloneable `Rc<str>`, not a true global
/// intern table with integer-compare dedup. Swapping in a real interner
/// later is a self-contained change to this file only — every call site
/// only ever clones, compares, or displays a `Symbol`, never inspects its
/// representation (see `docs/architecture/crates.md`, `ast` future
/// extension points).
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Symbol(Rc<str>);

impl Symbol {
    pub fn new(s: impl AsRef<str>) -> Self {
        Symbol(Rc::from(s.as_ref()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for Symbol {
    fn from(s: &str) -> Self {
        Symbol::new(s)
    }
}

impl From<String> for Symbol {
    fn from(s: String) -> Self {
        Symbol::new(s)
    }
}
