use std::collections::{HashMap, HashSet};

use nether_ast::Symbol;

/// Identifies one local binding (a `let`, a parameter, `self`, or a pattern
/// binding) for the lifetime of one [`resolve`](crate::resolve) call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocalId(u32);

#[derive(Default)]
pub(crate) struct LocalIdGen(u32);

impl LocalIdGen {
    pub(crate) fn next_id(&mut self) -> LocalId {
        let id = LocalId(self.0);
        self.0 += 1;
        id
    }
}

/// A stack of nested block scopes for resolving local variable names.
///
/// Pushed on entry to a function/method/closure body and every nested
/// `{ }` block; popped on exit. Lookup walks innermost-to-outermost so an
/// inner `let` shadows an outer one of the same name, matching the
/// language-spec's Rust-like block semantics (§2.4).
pub(crate) struct Scopes {
    stack: Vec<HashMap<Symbol, LocalId>>,
}

impl Scopes {
    pub(crate) fn new() -> Self {
        Scopes { stack: vec![HashMap::new()] }
    }

    pub(crate) fn push(&mut self) {
        self.stack.push(HashMap::new());
    }

    pub(crate) fn pop(&mut self) {
        self.stack.pop();
        debug_assert!(!self.stack.is_empty(), "popped the outermost scope");
    }

    pub(crate) fn bind(&mut self, name: Symbol, id: LocalId) {
        self.stack.last_mut().expect("at least one scope is always active").insert(name, id);
    }

    pub(crate) fn lookup(&self, name: &Symbol) -> Option<LocalId> {
        self.stack.iter().rev().find_map(|scope| scope.get(name).copied())
    }

    /// Every name currently visible at a value-path use site. Inner
    /// bindings win when a name is shadowed; sorting makes typo
    /// suggestions deterministic despite `HashMap` iteration order.
    pub(crate) fn visible_names(&self) -> Vec<Symbol> {
        let mut seen = HashSet::new();
        let mut names = Vec::new();
        for scope in self.stack.iter().rev() {
            for name in scope.keys() {
                if seen.insert(name.clone()) {
                    names.push(name.clone());
                }
            }
        }
        names.sort();
        names
    }
}
