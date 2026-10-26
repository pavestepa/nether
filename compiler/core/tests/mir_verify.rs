//! Corrupt checked MIR to ensure the backend does not trust frontend invariants.
use nether_core::{
    check::check,
    mir::{self, Operand, Operation, Place, Terminator},
    rust,
};
use nether_frontend::{parser::parse, source::SourceId};
use nether_semantics::OverflowChecks;

fn program(source: &str) -> mir::Program {
    let parsed = parse(SourceId(0), source);
    assert!(parsed.diagnostics.is_empty());
    let p = mir::lower(&check(&parsed.module.unwrap()).unwrap());
    mir::verify::verify(&p).unwrap();
    p
}
fn rejected(p: &mir::Program, message: &str) {
    let error = mir::verify::verify(p).expect_err("invalid MIR accepted");
    assert!(error.message.contains(message), "{error:?}");
    assert_eq!(
        rust::emit_mir(p, OverflowChecks::Checked).unwrap_err().code,
        "E0401"
    );
}
const OWNER: &str = "struct R {value:i32} struct H {x:R} fn main():i32 {let r=R {value:3};let h=H {x:r};return h.x.value}";

#[test]
fn rejects_invalid_cfg_and_cleanup_before_dataflow() {
    let p = program(OWNER);
    let id = p.main.unwrap();
    let mut bad = p.clone();
    bad.functions[id].entry = usize::MAX;
    rejected(&bad, "entry");
    let mut bad = p.clone();
    bad.functions[id].blocks[0].terminator = Terminator::Goto(usize::MAX);
    rejected(&bad, "successor");
    let mut bad = p.clone();
    bad.functions[id].cleanup.owners.pop();
    rejected(&bad, "cleanup");
    let mut bad = p.clone();
    bad.functions[id].cleanup.parents[0] = Some(0);
    rejected(&bad, "cleanup");
    let mut bad = p;
    bad.functions[id].blocks[0].terminator = Terminator::Return(Operand::Local(usize::MAX));
    rejected(&bad, "operand");
}

#[test]
fn rejects_copy_of_non_copy_and_duplicate_owner_in_one_operation() {
    let p = program(OWNER);
    let id = p.main.unwrap();
    let mut bad = p.clone();
    let operation = bad.functions[id]
        .blocks
        .iter_mut()
        .find_map(|b| match &mut b.terminator {
            Terminator::Evaluate {
                operation: op @ Operation::Move(_),
                ..
            } => Some(op),
            _ => None,
        })
        .unwrap();
    let Operation::Move(place) = operation else {
        unreachable!()
    };
    *operation = Operation::Copy(place.clone());
    rejected(&bad, "Copy");
    let mut bad = p;
    let f = &mut bad.functions[id];
    let owner = f.locals.iter().position(|l| l.name == "r").unwrap();
    let operation = f
        .blocks
        .iter_mut()
        .find_map(|b| match &mut b.terminator {
            Terminator::Evaluate {
                operation: op @ Operation::Move(_),
                ..
            } => Some(op),
            _ => None,
        })
        .unwrap();
    *operation = Operation::Aggregate(vec![Operand::Local(owner), Operand::Local(owner)]);
    rejected(&bad, "duplicates an ownership");
}

#[test]
fn rejects_uninitialized_return() {
    let mut p = program(OWNER);
    let f = &mut p.functions[p.main.unwrap()];
    let owner = f.locals.iter().position(|l| l.name == "r").unwrap();
    f.blocks[f.entry].terminator = Terminator::Return(Operand::Local(owner));
    rejected(&p, "uninitialized");
}

#[test]
fn rejects_erased_result_provenance_and_call_contract() {
    let p = program("struct R {value:i32} fn id(x:R):R {return x} fn main():i32 {let r=R {value:3};let a=id(r);return a.value}");
    let mut bad = p.clone();
    bad.functions
        .iter_mut()
        .find(|f| f.result_view.is_some())
        .unwrap()
        .result_view = None;
    rejected(&bad, "result provenance");
    let mut bad = p;
    for f in &mut bad.functions {
        for b in &mut f.blocks {
            if let Terminator::Evaluate { operation: op, .. } = &mut b.terminator {
                if let Operation::CallView {
                    function,
                    arguments,
                    ..
                } = op
                {
                    *op = Operation::Call(*function, arguments.clone());
                }
            }
        }
    }
    rejected(&bad, "provenance");
}

#[test]
fn rejects_readonly_to_write_in_mir() {
    let mut p = program(
        "struct R {value:i32} fn main():i32 {var r=R {value:3};var a=r;a.value+=1;return r.value}",
    );
    let f = &mut p.functions[p.main.unwrap()];
    let owner = f.locals.iter().position(|l| l.name == "r").unwrap();
    f.locals[owner].mutable = false;
    rejected(&p, "mutable");
}

#[test]
fn rejects_move_from_borrowed_parameter() {
    let mut p = program("struct R {value:i32} fn id(x:R):R {return x} fn main():i32 {let r=R {value:3};let a=id(r);return a.value}");
    let f = p
        .functions
        .iter_mut()
        .find(|f| f.result_view.is_some())
        .unwrap();
    let parameter = f.parameters[0];
    let op = f
        .blocks
        .iter_mut()
        .find_map(|b| match &mut b.terminator {
            Terminator::Evaluate {
                operation: op @ Operation::Borrow(_),
                ..
            } => Some(op),
            _ => None,
        })
        .unwrap();
    *op = Operation::Move(Place {
        local: parameter,
        projections: vec![mir::Projection::Dereference],
    });
    assert!(mir::verify::verify(&p).is_err());
    assert!(rust::emit_mir(&p, OverflowChecks::Checked).is_err());
}

#[test]
fn rejects_missing_cleanup_on_normal_and_panic_edges() {
    let p = program(OWNER);
    let mut bad = p;
    let f = &mut bad.functions[bad.main.unwrap()];
    for b in &mut f.blocks {
        if let Terminator::EndStorage { locals, .. } = &mut b.terminator {
            locals.clear();
        }
    }
    rejected(&bad, "cleanup");
    let mut bad = program("struct R {value:i32} fn main():i32 {let r=R {value:3};return 1/0}");
    let f = &mut bad.functions[bad.main.unwrap()];
    let resume = f
        .blocks
        .iter()
        .position(|b| matches!(b.terminator, Terminator::ResumePanic))
        .unwrap();
    for b in &mut f.blocks {
        if let Terminator::EndStorage { locals, next } = &mut b.terminator {
            if *next == resume {
                locals.clear();
            }
        }
    }
    rejected(&bad, "cleanup");
}

#[test]
fn rejects_invalid_projection() {
    let mut p = program(OWNER);
    let f = &mut p.functions[p.main.unwrap()];
    for b in &mut f.blocks {
        if let Terminator::Evaluate {
            operation: Operation::Copy(place),
            ..
        } = &mut b.terminator
        {
            place.projections.push(mir::Projection::Field(usize::MAX));
            break;
        }
    }
    rejected(&p, "projection");
}

#[test]
fn rejects_call_abi_and_storage_effect_tampering() {
    let p = program("struct R {value:i32} fn id(x:R):R {return x} fn main():i32 {let r=R {value:3};let a=id(r);return a.value}");
    let mut bad = p;
    let f = &mut bad.functions[bad.main.unwrap()];
    for b in &mut f.blocks {
        if let Terminator::Evaluate {
            operation: Operation::CallView { arguments, .. },
            ..
        } = &mut b.terminator
        {
            let Operand::Address(place) = &arguments[0] else {
                panic!()
            };
            arguments[0] = Operand::Local(place.local);
        }
    }
    rejected(&bad, "ABI");

    let mut bad = program("struct R {value:i32} struct H {x:R} fn set(var h:H,r:R):() {let a=r;h.x=a} fn main():i32 {let r=R {value:3};let a=r;var h=H {x:a};set(h,r);return h.x.value}");
    for f in &mut bad.functions {
        f.storage_writes = std::sync::Arc::new(vec![vec![]; f.storage_writes.len()]);
    }
    rejected(&bad, "effects");
}

#[test]
fn rejects_erased_indirect_result_contract() {
    let mut p = program("struct R {value:i32} fn id(x:R):R {return x} fn main():i32 {let r=R {value:3};let f=id;let a=f(r);return a.value}");
    let mut changed = false;
    for f in &mut p.functions {
        for b in &mut f.blocks {
            if let Terminator::Evaluate {
                operation: Operation::CallIndirect(_, _, contract),
                ..
            } = &mut b.terminator
            {
                contract.view = None;
                changed = true;
            }
        }
    }
    assert!(changed);
    rejected(&p, "provenance");
}

#[test]
fn rejects_forged_raw_capabilities_and_pointee_types() {
    let p = program("fn main():i32 {var n=1;let p=&var n;unsafe {*p=7;return *p}}");
    let mut bad = p.clone();
    let f = &mut bad.functions[bad.main.unwrap()];
    let n = f.locals.iter().position(|l| l.name == "n").unwrap();
    f.locals[n].mutable = false;
    rejected(&bad, "raw pointer");
    let mut bad = p.clone();
    let f = &mut bad.functions[bad.main.unwrap()];
    for b in &f.blocks {
        if let Terminator::Evaluate {
            operation: Operation::Binary("raw_write", Operand::Local(id), _),
            ..
        } = &b.terminator
        {
            let nether_core::hir::Type::Pointer { mutable, .. } = &mut f.locals[*id].ty else {
                panic!()
            };
            *mutable = false;
        }
    }
    rejected(&bad, "raw pointer");
    let mut bad = p;
    let f = &mut bad.functions[bad.main.unwrap()];
    for b in &f.blocks {
        if let Terminator::Evaluate {
            operation: Operation::Unary("raw_read", _),
            destination,
            ..
        } = &b.terminator
        {
            f.locals[destination.local].ty = nether_core::hir::Type::Bool;
        }
    }
    rejected(&bad, "raw pointer");
}
