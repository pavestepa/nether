use nether_core::{
    check::check,
    mir::{
        self,
        cleanup::{Plan, State},
    },
};
use nether_frontend::{parser::parse, source::SourceId};

#[test]
fn reinitialization_orders_roots_within_their_owning_scope() {
    let mut plan = Plan::new(4);
    let outer = plan.scope(0);
    let inner = plan.scope(outer);
    plan.assign(0, outer);
    plan.assign(1, outer);
    plan.assign(2, inner);
    plan.assign(3, inner);
    let mut state = State::new(&plan);
    state.initialize(0);
    state.initialize(1);
    state.initialize(2);
    // Replacement executes inside inner but a remains an outer root.
    assert_eq!(state.end(&[0]), vec![0]);
    state.initialize(0);
    assert_eq!(state.end(&[0, 1, 2, 3]), vec![2, 0, 1]);
    assert!(state.end(&[0, 1, 2, 3]).is_empty());
    state.initialize(0);
    state.initialize(1);
    state.forget(0);
    state.initialize(0);
    assert_eq!(state.end(&[1]), vec![1]);
    assert_eq!(state.end(&[0]), vec![0]);
    for _ in 0..100_000 {
        state.initialize(0);
        state.initialize(1);
        state.initialize(0);
    }
    assert_eq!(state.end(&[0, 1]), vec![0, 1]);
}

#[test]
fn lowering_keeps_outer_owners_and_separates_temporary_scopes() {
    let source="struct Cell {value:i32} fn main():i32 {var outer=Cell {value:1};if true {let inner=Cell {value:2};outer=Cell {value:3};let read=inner.value};return outer.value}";
    let parsed = parse(SourceId(0), source);
    assert!(parsed.diagnostics.is_empty());
    let hir = check(&parsed.module.unwrap()).unwrap();
    let cfg = mir::lower(&hir);
    let function = &cfg.functions[cfg.main.unwrap()];
    let outer = function
        .locals
        .iter()
        .position(|v| v.name == "outer")
        .unwrap();
    let inner = function
        .locals
        .iter()
        .position(|v| v.name == "inner")
        .unwrap();
    let outer_scope = function.cleanup.owners[outer];
    let inner_scope = function.cleanup.owners[inner];
    let mut parent = Some(inner_scope);
    while parent.is_some_and(|p| p != outer_scope) {
        parent = function.cleanup.parents[parent.unwrap()];
    }
    assert_eq!(parent, Some(outer_scope));
    assert_ne!(inner_scope, outer_scope);
    let groups = function.cleanup.groups(&[outer, inner]);
    assert_eq!(groups[0].1, vec![inner]);
    assert_eq!(groups[1].1, vec![outer]);
    for block in &function.blocks {
        if let mir::Terminator::Return(mir::Operand::Local(local)) = &block.terminator {
            assert_eq!(function.cleanup.owners[*local], 0);
        }
    }
    assert_eq!(function.cleanup.owners.len(), function.locals.len());
    assert!(mir::dataflow::initialization_errors(function).is_empty());
}
