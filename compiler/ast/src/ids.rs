/// Identifies one AST node (an [`crate::expr::Expr`] or a
/// [`crate::ident::Path`]) for the lifetime of one compilation.
///
/// `resolver` and `typecheck` key their result side-tables
/// (`path_res: HashMap<NodeId, Resolution>`, `expr_types: HashMap<NodeId,
/// Type>`) by `NodeId` rather than mutating AST nodes in place — see
/// `docs/architecture/crates.md` (`ast` invariants) for why the AST stays
/// immutable once built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(u32);

/// Hands out fresh, never-repeating [`NodeId`]s while parsing one
/// compilation unit.
#[derive(Debug, Default)]
pub struct NodeIdGen(u32);

impl NodeIdGen {
    pub fn new() -> Self {
        NodeIdGen(0)
    }

    pub fn with_start(start: u32) -> Self {
        NodeIdGen(start)
    }

    pub fn next_value(&self) -> u32 {
        self.0
    }

    pub fn next_id(&mut self) -> NodeId {
        let id = NodeId(self.0);
        self.0 += 1;
        id
    }
}
