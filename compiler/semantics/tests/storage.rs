use nether_semantics::{
    ElementMode as Mode, Layout, ModeError, ModeRequirement as Req, StorageModes,
};

#[test]
fn mode_is_inferred_from_all_uses_and_defaults_only_at_finish() {
    let mut solver = StorageModes::default();
    let a = solver.variable();
    let b = solver.variable();
    let empty = solver.variable();
    solver.unify(a, b).unwrap();
    assert_eq!(solver.requirement(a), Ok(None));
    solver
        .constrain(
            b,
            Req {
                mode: Mode::ReadonlyView,
                site: 7,
            },
        )
        .unwrap();
    assert_eq!(
        solver.requirement(a).unwrap().unwrap().mode,
        Mode::ReadonlyView
    );
    assert_eq!(solver.requirement(empty), Ok(None));
    assert_eq!(
        solver.finish(),
        [Mode::ReadonlyView, Mode::ReadonlyView, Mode::Owned]
    );
}

#[test]
fn conflicting_branch_requirements_report_both_sources_without_merging() {
    let mut solver = StorageModes::default();
    let a = solver.variable();
    let b = solver.variable();
    let first = Req {
        mode: Mode::Owned,
        site: 11,
    };
    let second = Req {
        mode: Mode::ReadonlyView,
        site: 29,
    };
    solver.constrain(a, first).unwrap();
    solver.constrain(b, second).unwrap();
    assert_eq!(
        solver.unify(a, b),
        Err(ModeError::Conflict { first, second })
    );
    assert_eq!(
        solver.constrain(a, second),
        Err(ModeError::Conflict { first, second })
    );
    assert_eq!(solver.finish(), [Mode::Owned, Mode::ReadonlyView]);
}

#[test]
fn recursive_mode_equalities_converge_without_runtime_selection() {
    let mut solver = StorageModes::default();
    let variables: Vec<_> = (0..1000).map(|_| solver.variable()).collect();
    for pair in variables.windows(2) {
        solver.unify(pair[0], pair[1]).unwrap();
    }
    solver.unify(variables[999], variables[0]).unwrap();
    solver
        .constrain(
            variables[541],
            Req {
                mode: Mode::ReadonlyView,
                site: 1,
            },
        )
        .unwrap();
    assert!(solver
        .finish()
        .iter()
        .all(|mode| *mode == Mode::ReadonlyView));
}

#[test]
fn mode_is_independent_of_constraint_visitation_order() {
    for reverse in [false, true] {
        let mut solver = StorageModes::default();
        let a = solver.variable();
        let b = solver.variable();
        let requirement = Req {
            mode: Mode::ReadonlyView,
            site: 10,
        };
        if reverse {
            solver.constrain(b, requirement).unwrap();
        }
        solver.unify(a, b).unwrap();
        if !reverse {
            solver.constrain(b, requirement).unwrap();
        }
        assert_eq!(solver.finish(), [Mode::ReadonlyView, Mode::ReadonlyView]);
    }
}

#[test]
fn borrowed_inline_elements_use_descriptors_without_mutation_or_target_drop() {
    let payload = Layout::new(128, 16).unwrap();
    assert_eq!(Mode::Owned.representation(payload), payload);
    assert_eq!(Mode::ReadonlyView.representation(payload), Layout::POINTER);
    assert!(Mode::Owned.drops_target());
    assert!(!Mode::ReadonlyView.drops_target());
    assert!(!Mode::ReadonlyView.permits_mutable_target());
}
