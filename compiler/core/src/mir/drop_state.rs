//! Sparse runtime initialization oracle for cleanup. Unlike compile-time facts,
//! these states distinguish incomplete construction from a moved-out subfield.
use super::drop_plan::{Initialization as I, Part};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug)]
struct Node {
    own: I,
    descendants: I,
}
#[derive(Clone, Debug, Default)]
pub struct State {
    nodes: BTreeMap<Vec<Part>, Node>,
}
fn covers(parent: &[Part], child: &[Part]) -> bool {
    child.starts_with(parent)
}
impl State {
    fn node(&self, path: &[Part]) -> Node {
        self.nodes
            .iter()
            .filter(|(prefix, _)| covers(prefix, path))
            .max_by_key(|(prefix, _)| prefix.len())
            .map(|(prefix, node)| {
                if prefix.len() == path.len() {
                    *node
                } else {
                    Node {
                        own: node.descendants,
                        descendants: node.descendants,
                    }
                }
            })
            .unwrap_or(Node {
                own: I::Vacant,
                descendants: I::Vacant,
            })
    }
    pub fn get(&self, path: &[Part]) -> I {
        let own = self.node(path).own;
        if matches!(own, I::Live | I::Vacant)
            && self.nodes.iter().any(|(child, node)| {
                child.len() > path.len() && covers(path, child) && node.own != own
            })
        {
            I::Partial
        } else {
            own
        }
    }

    /// Whole initialization invalidates old move flags and constructor progress.
    pub fn initialize(&mut self, path: &[Part]) -> Result<(), I> {
        if self.get(path) == I::Finalizing {
            return Err(I::Finalizing);
        }
        self.replace(path, I::Live);
        Ok(())
    }
    pub fn constructing(&mut self, path: &[Part]) -> Result<(), I> {
        if self.get(path) == I::Finalizing {
            return Err(I::Finalizing);
        }
        self.replace(path, I::Vacant);
        self.nodes.insert(
            path.into(),
            Node {
                own: I::Constructing,
                descendants: I::Vacant,
            },
        );
        Ok(())
    }
    pub fn forget(&mut self, path: &[Part]) -> Result<(), I> {
        let state = self.get(path);
        if state != I::Live {
            return Err(state);
        }
        self.replace(path, I::Vacant);
        Ok(())
    }
    /// Mark only this lifetime finalizing. Its initialized children remain live
    /// and readable by the body until their own cleanup starts.
    pub fn begin(&mut self, path: &[Part]) -> Result<(), I> {
        let state = self.get(path);
        if matches!(state, I::Vacant | I::Finalizing) {
            return Err(state);
        }
        let old = self.node(path);
        self.nodes.insert(
            path.into(),
            Node {
                own: I::Finalizing,
                descendants: old.descendants,
            },
        );
        Ok(())
    }
    pub fn finish(&mut self, path: &[Part]) {
        self.replace(path, I::Vacant);
    }
    fn replace(&mut self, path: &[Part], state: I) {
        self.nodes.retain(|child, _| !covers(path, child));
        self.nodes.insert(
            path.into(),
            Node {
                own: state,
                descendants: state,
            },
        );
    }
    /// Exposed to reference tests to detect history accumulation on replacement.
    pub fn recorded_paths(&self) -> usize {
        self.nodes.len()
    }
}
