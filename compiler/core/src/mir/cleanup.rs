//! Lexical ownership of cleanup records. Initialization order within each scope
//! is dynamic; replacing a root never changes its owning scope.
use super::LocalId;
pub type ScopeId = usize;

#[derive(Clone, Debug)]
pub struct Plan {
    /// Parents precede children. Scope zero owns parameters and return staging.
    pub parents: Vec<Option<ScopeId>>,
    pub owners: Vec<ScopeId>,
    /// Completed fields of an unfinished aggregate clean in declaration order.
    pub construction: std::collections::BTreeSet<ScopeId>,
    pub field_order: std::collections::BTreeMap<LocalId, usize>,
}
impl Default for Plan {
    fn default() -> Self {
        Self::new(0)
    }
}
impl Plan {
    pub fn new(locals: usize) -> Self {
        Self {
            parents: vec![None],
            owners: vec![0; locals],
            construction: Default::default(),
            field_order: Default::default(),
        }
    }
    pub fn scope(&mut self, parent: ScopeId) -> ScopeId {
        assert!(parent < self.parents.len());
        let scope = self.parents.len();
        self.parents.push(Some(parent));
        scope
    }
    pub fn construction_scope(&mut self, parent: ScopeId) -> ScopeId {
        let scope = self.scope(parent);
        self.construction.insert(scope);
        scope
    }
    pub fn add_local(&mut self, scope: ScopeId) {
        assert!(scope < self.parents.len());
        self.owners.push(scope);
    }
    pub fn assign(&mut self, local: LocalId, scope: ScopeId) {
        assert!(scope < self.parents.len());
        self.owners[local] = scope;
    }
    /// Select scopes from innermost to outermost; runtime orders the selected
    /// active roots of each scope by their most recent successful initialization.
    pub fn groups(&self, locals: &[LocalId]) -> Vec<(ScopeId, Vec<LocalId>)> {
        let mut groups = std::collections::BTreeMap::<_, Vec<_>>::new();
        for local in locals {
            let group = groups.entry(self.owners[*local]).or_default();
            if !group.contains(local) {
                group.push(*local);
            }
        }
        groups.into_iter().rev().collect()
    }
}

/// Reference execution uses vectors, deliberately independent of the native
/// intrusive list. Entries are bounded by the number of frame locals.
pub struct State<'a> {
    plan: &'a Plan,
    order: Vec<Vec<LocalId>>,
}
impl<'a> State<'a> {
    pub fn new(plan: &'a Plan) -> Self {
        Self {
            plan,
            order: vec![Vec::new(); plan.parents.len()],
        }
    }
    pub fn initialize(&mut self, local: LocalId) {
        self.forget(local);
        self.order[self.plan.owners[local]].push(local);
    }
    pub fn forget(&mut self, local: LocalId) {
        self.order[self.plan.owners[local]].retain(|id| *id != local);
    }
    pub fn end(&mut self, locals: &[LocalId]) -> Vec<LocalId> {
        let mut result = Vec::new();
        for (scope, selected) in self.plan.groups(locals) {
            let selected = selected
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>();
            if self.plan.construction.contains(&scope) {
                let mut fields: Vec<_> = selected
                    .iter()
                    .filter(|id| self.order[scope].contains(id))
                    .copied()
                    .collect();
                fields.sort_by_key(|id| self.plan.field_order.get(id).copied().unwrap_or(*id));
                result.extend(fields);
            } else {
                result.extend(
                    self.order[scope]
                        .iter()
                        .rev()
                        .filter(|id| selected.contains(id))
                        .copied(),
                );
            }
            self.order[scope].retain(|id| !selected.contains(id));
        }
        result
    }
}
