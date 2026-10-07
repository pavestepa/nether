use nether_core::mir::{
    initialization::{State, Status as S},
    Place, Projection as P,
};
fn field(index: usize) -> Place {
    Place {
        local: 0,
        projections: vec![P::Field(index)],
    }
}
#[test]
fn partial_moves_preserve_disjoint_fields_and_reinitialization_restores_the_root() {
    let mut state = State::default();
    state.initialize(Place::local(0));
    state.move_out(field(0)).unwrap();
    assert_eq!(state.status(&Place::local(0)), S::Maybe);
    assert_eq!(state.status(&field(0)), S::Uninitialized);
    assert_eq!(state.status(&field(1)), S::Initialized);
    assert_eq!(state.move_out(field(0)), Err(S::Uninitialized));
    state.initialize(field(0));
    assert_eq!(state.status(&Place::local(0)), S::Initialized);
}
#[test]
fn branch_joins_keep_maybe_moved_state_and_proven_disjoint_fields() {
    let mut before = State::default();
    before.initialize(Place::local(0));
    let mut branch = before.clone();
    branch.move_out(field(0)).unwrap();
    let joined = before.join(&branch, [field(0), field(1)]);
    assert_eq!(joined.status(&field(0)), S::Maybe);
    assert_eq!(joined.status(&field(1)), S::Initialized);
    let reversed = branch.join(&before, [field(0), field(1)]);
    for p in [Place::local(0), field(0), field(1)] {
        assert_eq!(joined.status(&p), reversed.status(&p));
    }
}
#[test]
fn unknown_indices_never_prove_other_slots_initialized() {
    let mut state = State::default();
    let root = Place::local(0);
    state.initialize(root.clone());
    let dynamic = Place {
        local: 0,
        projections: vec![P::Index(10)],
    };
    let zero = Place {
        local: 0,
        projections: vec![P::ConstantIndex(0)],
    };
    state.move_out(dynamic.clone()).unwrap();
    assert_eq!(state.status(&zero), S::Maybe);
    assert_eq!(state.status(&dynamic), S::Uninitialized);
    state.initialize(zero.clone());
    assert_eq!(state.status(&zero), S::Initialized);
    assert_eq!(state.status(&root), S::Maybe);
    state.initialize(root.clone());
    assert_eq!(state.status(&dynamic), S::Initialized);
    assert_eq!(state.status(&root), S::Initialized);
}
#[test]
fn generated_field_move_replace_sequences_match_a_leaf_bitmap() {
    for script in 0u32..4096 {
        let mut state = State::default();
        state.initialize(Place::local(0));
        let mut initialized = [true; 3];
        let mut script = script;
        for _ in 0..6 {
            let operation = script % 4;
            script /= 4;
            if operation == 3 {
                for (i, slot) in initialized.iter_mut().enumerate() {
                    state.initialize(field(i));
                    *slot = true;
                }
            } else {
                let index = operation as usize;
                let accepted = state.move_out(field(index)).is_ok();
                assert_eq!(accepted, initialized[index]);
                initialized[index] = false;
            }
            for (i, slot) in initialized.iter().enumerate() {
                assert_eq!(
                    state.status(&field(i)),
                    if *slot {
                        S::Initialized
                    } else {
                        S::Uninitialized
                    }
                );
            }
            assert_eq!(
                state.status(&Place::local(0)) == S::Initialized,
                initialized.iter().all(|v| *v)
            );
        }
    }
}
