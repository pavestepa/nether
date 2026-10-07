//! Constraint solving for the approved static storage specializations.
//! These modes are compiler data; they are not per-element runtime tags.

use crate::Layout;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElementMode {
    Owned,
    ReadonlyView,
}

impl ElementMode {
    pub fn representation(self, value: Layout) -> Layout {
        match self {
            Self::Owned => value,
            Self::ReadonlyView => Layout::POINTER,
        }
    }

    pub fn drops_target(self) -> bool {
        self == Self::Owned
    }
    pub fn permits_mutable_target(self) -> bool {
        self == Self::Owned
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModeVar(usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModeRequirement {
    pub mode: ElementMode,
    /// Source-site identity supplied by the frontend's source map.
    pub site: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeError {
    UnknownVariable,
    Conflict {
        first: ModeRequirement,
        second: ModeRequirement,
    },
}

#[derive(Default, Debug)]
pub struct StorageModes {
    nodes: Vec<Node>,
}

#[derive(Debug)]
struct Node {
    parent: usize,
    rank: u32,
    requirement: Option<ModeRequirement>,
}

impl StorageModes {
    pub fn variable(&mut self) -> ModeVar {
        let index = self.nodes.len();
        self.nodes.push(Node {
            parent: index,
            rank: 0,
            requirement: None,
        });
        ModeVar(index)
    }

    pub fn constrain(
        &mut self,
        variable: ModeVar,
        requirement: ModeRequirement,
    ) -> Result<(), ModeError> {
        let root = self.root(variable)?;
        if let Some(first) = self.nodes[root].requirement {
            compatible(first, requirement)?;
        } else {
            self.nodes[root].requirement = Some(requirement);
        }
        Ok(())
    }

    /// Assignment, branch joins and cross-call storage parameters unify modes.
    /// On conflict the equivalence classes stay unchanged.
    pub fn unify(&mut self, left: ModeVar, right: ModeVar) -> Result<(), ModeError> {
        let (mut a, mut b) = (self.root(left)?, self.root(right)?);
        if a == b {
            return Ok(());
        }
        let requirement = match (self.nodes[a].requirement, self.nodes[b].requirement) {
            (Some(first), Some(second)) => {
                compatible(first, second)?;
                Some(first)
            }
            (first, second) => first.or(second),
        };
        if self.nodes[a].rank < self.nodes[b].rank {
            std::mem::swap(&mut a, &mut b);
        }
        if self.nodes[a].rank == self.nodes[b].rank {
            self.nodes[a].rank += 1;
        }
        self.nodes[b].parent = a;
        self.nodes[a].requirement = requirement;
        Ok(())
    }

    /// Querying an unconstrained variable must not prematurely choose owned.
    pub fn requirement(&self, variable: ModeVar) -> Result<Option<ModeRequirement>, ModeError> {
        Ok(self.nodes[self.root(variable)?].requirement)
    }

    /// Consume the complete constraint set, defaulting unconstrained classes.
    pub fn finish(self) -> Vec<ElementMode> {
        (0..self.nodes.len())
            .map(|index| {
                self.requirement(ModeVar(index))
                    .expect("internal variable")
                    .map_or(ElementMode::Owned, |requirement| requirement.mode)
            })
            .collect()
    }

    fn root(&self, variable: ModeVar) -> Result<usize, ModeError> {
        if variable.0 >= self.nodes.len() {
            return Err(ModeError::UnknownVariable);
        }
        let mut current = variable.0;
        while self.nodes[current].parent != current {
            current = self.nodes[current].parent;
        }
        Ok(current)
    }
}

fn compatible(first: ModeRequirement, second: ModeRequirement) -> Result<(), ModeError> {
    if first.mode == second.mode {
        Ok(())
    } else {
        Err(ModeError::Conflict { first, second })
    }
}
