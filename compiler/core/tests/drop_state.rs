use nether_core::mir::{
    drop_plan::{Initialization as I, Part},
    drop_state::State,
};
#[test]
fn finalizing_parent_keeps_fields_accessible_and_reinitialization_forgets_old_moves() {
    let mut state = State::default();
    let a = [Part::Field(0)];
    let b = [Part::Field(1)];
    assert_eq!(state.get(&[]), I::Vacant);
    state.initialize(&[]).unwrap();
    state.forget(&a).unwrap();
    assert_eq!(state.get(&[]), I::Partial);
    assert_eq!(state.get(&b), I::Live);
    state.initialize(&a).unwrap();
    assert_eq!(state.get(&[]), I::Live);
    state.begin(&[]).unwrap();
    assert_eq!(state.get(&[]), I::Finalizing);
    assert_eq!(state.get(&a), I::Live);
    assert_eq!(state.forget(&[]), Err(I::Finalizing));
    state.begin(&a).unwrap();
    state.finish(&a);
    assert_eq!(state.get(&b), I::Live);
    state.finish(&[]);
    assert_eq!(state.get(&b), I::Vacant);
    state.initialize(&[]).unwrap();
    assert_eq!(state.get(&a), I::Live);
    for i in 0..100_000 {
        state.forget(&[Part::Element(i)]).unwrap();
        state.initialize(&[]).unwrap();
    }
    assert_eq!(state.recorded_paths(), 1);
}
#[test]
fn construction_progress_does_not_make_unwritten_fields_live() {
    let mut state = State::default();
    let first = [Part::Field(0)];
    let second = [Part::Field(1)];
    state.constructing(&[]).unwrap();
    state.initialize(&first).unwrap();
    assert_eq!(state.get(&[]), I::Constructing);
    assert_eq!(state.get(&first), I::Live);
    assert_eq!(state.get(&second), I::Vacant);
    state.begin(&[]).unwrap();
    assert_eq!(state.get(&first), I::Live);
    assert_eq!(state.get(&second), I::Vacant);
    state.finish(&first);
    state.finish(&[]);
    assert_eq!(state.recorded_paths(), 1);
}
#[test]
fn nested_partial_moves_preserve_sibling_payloads() {
    let mut state = State::default();
    state.initialize(&[]).unwrap();
    let moved = [Part::Variant(1), Part::Field(0), Part::Element(3)];
    state.forget(&moved).unwrap();
    assert_eq!(state.get(&[Part::Variant(1), Part::Field(0)]), I::Partial);
    assert_eq!(state.get(&[Part::Variant(1), Part::Field(1)]), I::Live);
    assert_eq!(
        state.get(&[Part::Variant(1), Part::Field(0), Part::Element(2)]),
        I::Live
    );
    state
        .initialize(&[Part::Variant(1), Part::Field(0)])
        .unwrap();
    assert_eq!(state.get(&[]), I::Live);
}

#[test]
fn a_live_child_of_vacant_storage_requires_partial_cleanup() {
    let mut state = State::default();
    state.initialize(&[Part::Field(0)]).unwrap();
    assert_eq!(state.get(&[]), I::Partial);
    assert_eq!(state.get(&[Part::Field(1)]), I::Vacant);
    state.begin(&[]).unwrap();
    assert_eq!(state.initialize(&[]), Err(I::Finalizing));
    assert_eq!(state.constructing(&[]), Err(I::Finalizing));
    state.initialize(&[Part::Field(1)]).unwrap();
    assert_eq!(state.get(&[Part::Field(1)]), I::Live);
}
