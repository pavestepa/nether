//! Loan conflict domain for resolved origins. Binding aliases must first be
//! normalized to their underlying root places by the ownership inference pass.
use super::{Place, Projection};
use std::collections::{BTreeMap, BTreeSet};
pub type LoanId = usize;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Readonly,
    Mutable,
}
#[derive(Clone, Debug)]
pub struct Loan {
    pub origins: Vec<Place>,
    pub access: Access,
    pub parent: Option<LoanId>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Conflict {
    MissingLoan(LoanId),
    Readonly,
    OutsideOrigin,
    Overlapping(LoanId),
}
#[derive(Default)]
pub struct Loans {
    loans: BTreeMap<LoanId, Loan>,
    live: BTreeSet<LoanId>,
    next: LoanId,
}
impl Loans {
    pub fn set_live(&mut self, live: impl IntoIterator<Item = LoanId>) -> Result<(), Conflict> {
        let live: BTreeSet<_> = live.into_iter().collect();
        for id in &live {
            if !self.loans.contains_key(id) {
                return Err(Conflict::MissingLoan(*id));
            }
        }
        self.live = live;
        Ok(())
    }
    pub fn borrow(
        &mut self,
        origins: Vec<Place>,
        access: Access,
        source_access: Access,
    ) -> Result<LoanId, Conflict> {
        if origins.is_empty() {
            return Err(Conflict::OutsideOrigin);
        }
        if access == Access::Mutable && source_access == Access::Readonly {
            return Err(Conflict::Readonly);
        }
        for place in &origins {
            self.access(place, access, None)?;
        }
        Ok(self.insert(Loan {
            origins,
            access,
            parent: None,
        }))
    }
    pub fn reborrow(
        &mut self,
        parent: LoanId,
        origins: Vec<Place>,
        access: Access,
    ) -> Result<LoanId, Conflict> {
        let source = self.active(parent)?;
        if access == Access::Mutable && source.access == Access::Readonly {
            return Err(Conflict::Readonly);
        }
        if origins.is_empty()
            || origins
                .iter()
                .any(|p| !source.origins.iter().any(|origin| contains(origin, p)))
        {
            return Err(Conflict::OutsideOrigin);
        }
        for place in &origins {
            self.access(place, access, Some(parent))?;
        }
        Ok(self.insert(Loan {
            origins,
            access,
            parent: Some(parent),
        }))
    }
    pub fn access(
        &self,
        place: &Place,
        access: Access,
        through: Option<LoanId>,
    ) -> Result<(), Conflict> {
        let mut ancestors = BTreeSet::new();
        if let Some(id) = through {
            let loan = self.active(id)?;
            if access == Access::Mutable && loan.access == Access::Readonly {
                return Err(Conflict::Readonly);
            }
            if !loan.origins.iter().any(|origin| contains(origin, place)) {
                return Err(Conflict::OutsideOrigin);
            }
            let mut current = Some(id);
            while let Some(id) = current {
                ancestors.insert(id);
                current = self.loans[&id].parent;
            }
        }
        for id in &self.live {
            if ancestors.contains(id) {
                continue;
            }
            let loan = &self.loans[id];
            if (access == Access::Mutable || loan.access == Access::Mutable)
                && loan.origins.iter().any(|origin| overlaps(origin, place))
            {
                return Err(Conflict::Overlapping(*id));
            }
        }
        Ok(())
    }
    /// Ending or replacing an owner invalidates every overlapping view, regardless
    /// of whether the owner binding itself is mutable.
    pub fn end_owner(&self, place: &Place) -> Result<(), Conflict> {
        self.access(place, Access::Mutable, None)
    }
    fn insert(&mut self, loan: Loan) -> LoanId {
        let id = self.next;
        self.next += 1;
        self.loans.insert(id, loan);
        self.live.insert(id);
        id
    }
    fn active(&self, id: LoanId) -> Result<&Loan, Conflict> {
        if !self.live.contains(&id) {
            return Err(Conflict::MissingLoan(id));
        }
        self.loans.get(&id).ok_or(Conflict::MissingLoan(id))
    }
}
pub fn overlaps(a: &Place, b: &Place) -> bool {
    if a.local != b.local {
        return false;
    }
    for (a, b) in a.projections.iter().zip(&b.projections) {
        if a == b {
            continue;
        }
        return match (a, b) {
            (Projection::Field(a), Projection::Field(b)) => a == b,
            (Projection::ConstantIndex(a), Projection::ConstantIndex(b)) => a == b,
            // Different variants share physical payload storage. Dynamic indices may alias.
            _ => true,
        };
    }
    true
}
fn contains(parent: &Place, child: &Place) -> bool {
    parent.local == child.local
        && parent.projections.len() <= child.projections.len()
        && parent
            .projections
            .iter()
            .zip(&child.projections)
            .all(|(a, b)| a == b)
}
