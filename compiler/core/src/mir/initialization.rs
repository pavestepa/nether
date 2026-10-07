//! Sparse initialization facts for move paths, independent of ownership/access.
//! A caller supplies all queried paths when joining control-flow alternatives.
use super::{loans::overlaps, Place};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Initialized,
    Uninitialized,
    Maybe,
}
#[derive(Clone, Debug, Default)]
pub struct State {
    facts: BTreeMap<Place, (u64, Status)>,
    clock: u64,
}
impl State {
    pub fn status(&self, place: &Place) -> Status {
        let covering = self
            .facts
            .iter()
            .filter(|(path, _)| covers(path, place))
            .max_by_key(|(_, fact)| fact.0);
        let (epoch, base) = covering
            .map(|(_, fact)| *fact)
            .unwrap_or((0, Status::Uninitialized));
        if base == Status::Maybe {
            return Status::Maybe;
        }
        if self
            .facts
            .iter()
            .any(|(path, (time, status))| *time > epoch && *status != base && overlaps(path, place))
        {
            Status::Maybe
        } else {
            base
        }
    }
    pub fn initialize(&mut self, place: Place) {
        self.set(place, Status::Initialized);
    }
    pub fn move_out(&mut self, place: Place) -> Result<(), Status> {
        let status = self.status(&place);
        if status != Status::Initialized {
            return Err(status);
        }
        self.set(place, Status::Uninitialized);
        Ok(())
    }
    pub fn set(&mut self, place: Place, status: Status) {
        self.clock += 1;
        self.facts.retain(|path, _| !covers(&place, path));
        self.facts.insert(place, (self.clock, status));
    }
    /// Join over the finite set of observable move paths. Root and prefix facts are
    /// included automatically; explicit descendants retain disjoint field proofs.
    pub fn join(&self, other: &Self, paths: impl IntoIterator<Item = Place>) -> Self {
        let mut universe: std::collections::BTreeSet<_> = paths
            .into_iter()
            .chain(self.facts.keys().cloned())
            .chain(other.facts.keys().cloned())
            .collect();
        for place in universe.clone() {
            for length in 0..place.projections.len() {
                universe.insert(Place {
                    local: place.local,
                    projections: place.projections[..length].to_vec(),
                });
            }
        }
        let mut paths: Vec<_> = universe.into_iter().collect();
        paths.sort_by_key(|p| p.projections.len());
        let mut result = Self::default();
        for path in paths {
            let a = self.status(&path);
            let b = other.status(&path);
            result.set(path, if a == b { a } else { Status::Maybe });
        }
        result
    }
}
fn covers(parent: &Place, child: &Place) -> bool {
    parent.local == child.local
        && parent.projections.len() <= child.projections.len()
        && parent
            .projections
            .iter()
            .zip(&child.projections)
            .all(|(a, b)| a == b)
}
