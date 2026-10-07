use nether_core::mir::{
    loans::{overlaps, Access as A, Conflict, Loans},
    Place, Projection as P,
};
fn place(root: usize, path: Vec<P>) -> Place {
    Place {
        local: root,
        projections: path,
    }
}
#[test]
fn overlap_is_symmetric_and_only_proven_projections_are_disjoint() {
    let paths = vec![
        vec![],
        vec![P::Field(0)],
        vec![P::Field(1)],
        vec![P::ConstantIndex(0)],
        vec![P::ConstantIndex(1)],
        vec![P::Index(4)],
        vec![P::Variant(0), P::Field(0)],
        vec![P::Variant(1), P::Field(0)],
    ];
    for a in &paths {
        for b in &paths {
            let a = place(0, a.clone());
            let b = place(0, b.clone());
            assert_eq!(overlaps(&a, &b), overlaps(&b, &a));
            assert!(!overlaps(&a, &place(1, b.projections.clone())));
        }
    }
    assert!(!overlaps(
        &place(0, vec![P::Field(0)]),
        &place(0, vec![P::Field(1)])
    ));
    assert!(!overlaps(
        &place(0, vec![P::ConstantIndex(0)]),
        &place(0, vec![P::ConstantIndex(1)])
    ));
    assert!(overlaps(
        &place(0, vec![P::Index(2)]),
        &place(0, vec![P::ConstantIndex(1)])
    ));
    assert!(overlaps(
        &place(0, vec![P::Variant(0)]),
        &place(0, vec![P::Variant(1)])
    ));
}
#[test]
fn mutable_loans_are_unique_and_shared_views_block_owner_replacement() {
    let mut loans = Loans::default();
    let owner = Place::local(0);
    let first = loans
        .borrow(vec![owner.clone()], A::Readonly, A::Mutable)
        .unwrap();
    let second = loans
        .borrow(vec![owner.clone()], A::Readonly, A::Mutable)
        .unwrap();
    assert!(loans.end_owner(&owner).is_err());
    assert!(loans
        .borrow(vec![owner.clone()], A::Mutable, A::Mutable)
        .is_err());
    loans.set_live([second]).unwrap();
    assert!(loans.end_owner(&owner).is_err());
    loans.set_live([]).unwrap();
    assert!(loans.end_owner(&owner).is_ok());
    let unique = loans
        .borrow(vec![owner.clone()], A::Mutable, A::Mutable)
        .unwrap();
    assert!(loans.access(&owner, A::Readonly, None).is_err());
    assert!(loans.access(&owner, A::Mutable, Some(unique)).is_ok());
    assert_eq!(
        loans.access(&owner, A::Readonly, Some(first)),
        Err(Conflict::MissingLoan(first))
    );
}
#[test]
fn reborrows_suspend_parent_access_and_preserve_readonly_capability() {
    let mut loans = Loans::default();
    let root = Place::local(0);
    let field = place(0, vec![P::Field(0)]);
    let other = place(0, vec![P::Field(1)]);
    let parent = loans
        .borrow(vec![root.clone()], A::Mutable, A::Mutable)
        .unwrap();
    let child = loans
        .reborrow(parent, vec![field.clone()], A::Readonly)
        .unwrap();
    assert!(loans.access(&field, A::Mutable, Some(parent)).is_err());
    assert!(loans.access(&other, A::Mutable, Some(parent)).is_ok());
    assert!(loans.access(&field, A::Readonly, Some(child)).is_ok());
    assert_eq!(
        loans.reborrow(child, vec![field.clone()], A::Mutable),
        Err(Conflict::Readonly)
    );
    assert_eq!(
        loans.access(&other, A::Readonly, Some(child)),
        Err(Conflict::OutsideOrigin)
    );
    loans.set_live([parent]).unwrap();
    assert!(loans.access(&field, A::Mutable, Some(parent)).is_ok());
}
