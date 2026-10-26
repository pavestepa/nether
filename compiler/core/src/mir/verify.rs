//! Validation at the public MIR/backend boundary. Run structural checks before
//! analyses, which deliberately assume valid indices. Recompute provenance
//! rather than trusting serialized/caller-supplied result contracts.
use super::*;
use nether_frontend::source::Diagnostic;
use std::collections::BTreeSet;

pub fn verify(program: &Program) -> Result<(), Diagnostic> {
    for f in &program.functions {
        let fail = |message| Diagnostic::new("E0401", message, f.span);
        if f.entry >= f.blocks.len()
            || f.parameters.iter().any(|p| *p >= f.locals.len())
            || f.parameters.iter().collect::<BTreeSet<_>>().len() != f.parameters.len()
            || f.drop_reads.iter().any(|p| *p >= f.locals.len())
        {
            return Err(fail("invalid MIR entry or local index"));
        }
        let plan = &f.cleanup;
        if plan.parents.first() != Some(&None)
            || plan
                .parents
                .iter()
                .enumerate()
                .skip(1)
                .any(|(i, p)| p.is_none_or(|p| p >= i))
            || plan.owners.len() != f.locals.len()
            || plan.owners.iter().any(|s| *s >= plan.parents.len())
            || plan.construction.iter().any(|s| *s >= plan.parents.len())
            || plan.field_order.keys().any(|l| *l >= f.locals.len())
            || f.storage_writes.len() != program.functions.len()
        {
            return Err(fail("invalid MIR cleanup or effect table"));
        }
        for block in &f.blocks {
            let t = &block.terminator;
            if t.successors().iter().any(|b| *b >= f.blocks.len())
                || dataflow::uses(t).iter().any(|l| *l >= f.locals.len())
            {
                return Err(fail("invalid MIR successor or operand index"));
            }
            let mut places = dataflow::required_initialized(t);
            if let Terminator::Evaluate { destination, .. } = t {
                places.push(destination.clone());
            }
            if places.iter().any(|p| place_type(f, p).is_none()) {
                return Err(fail("invalid MIR place projection"));
            }
            if let Terminator::EndStorage { locals, .. } = t {
                if locals.iter().any(|l| *l >= f.locals.len()) {
                    return Err(fail("invalid MIR storage boundary"));
                }
            }
            if let Terminator::Evaluate {
                operation,
                destination,
                ..
            } = t
            {
                if destination.local >= f.locals.len()
                    || destination
                        .projections
                        .iter()
                        .any(|p| matches!(p, Projection::Index(l) if *l >= f.locals.len()))
                {
                    return Err(fail("invalid MIR destination"));
                }
                verify_raw(f, operation, destination).map_err(fail)?;
                // Two uses of the same owner in one aggregate/call would both
                // appear initialized on entry to this block. Check atomically.
                let consumed = dataflow::consumed(f, operation);
                for (i, a) in consumed.iter().enumerate() {
                    if consumed[i + 1..].iter().any(|b| loans::overlaps(a, b)) {
                        return Err(fail("MIR operation duplicates an ownership token"));
                    }
                }
                if let Operation::Copy(p) = operation {
                    if !place_type(f, p).is_some_and(|ty| ty.copyable()) {
                        return Err(fail("MIR Copy duplicates a non-Copy owner"));
                    }
                }
                if let Some((callee, args)) = storage_writes::call(operation) {
                    let Some(target) = program.functions.get(callee) else {
                        return Err(fail("invalid MIR call target"));
                    };
                    if target.parameters.len() != args.len() {
                        return Err(fail("invalid MIR call arity"));
                    }
                    for (parameter, arg) in target.parameters.iter().zip(args) {
                        let Some(parameter) = target.locals.get(*parameter) else {
                            return Err(fail("invalid MIR callee parameter"));
                        };
                        match (parameter.view, arg) {
                            (Some(access), Operand::Address(place)) => {
                                if access == loans::Access::Mutable && !writable(f, place) {
                                    return Err(fail(
                                        "MIR call upgrades readonly access to mutable",
                                    ));
                                }
                            }
                            (None, Operand::Address(_)) | (Some(_), _) => {
                                return Err(fail("MIR call changes ownership/borrow ABI"));
                            }
                            _ => {}
                        }
                    }
                }
                if let Some((args, contract)) = returns::call_view(operation) {
                    if destination.projections.is_empty()
                        && f.locals[destination.local].view != Some(contract.access)
                    {
                        return Err(fail("MIR result slot changes borrowed access"));
                    }
                    if contract.origins.is_empty()
                        || contract.origins.iter().any(|o| o.parameter >= args.len())
                    {
                        return Err(fail("invalid MIR borrowed result origins"));
                    }
                }
                if let Some((args, contract)) = returns::call_storage(operation) {
                    if contract
                        .iter()
                        .any(|c| c.origins.iter().any(|o| o.parameter >= args.len()))
                    {
                        return Err(fail("invalid MIR storage origins"));
                    }
                }
                if let Operation::CallIndirect(_, _, contract) = operation {
                    if contract
                        .targets
                        .iter()
                        .any(|t| *t >= program.functions.len())
                    {
                        return Err(fail("invalid MIR indirect call target"));
                    }
                }
            }
        }
    }
    let views = returns::infer(program)
        .map_err(|e| Diagnostic::new("E0401", e.message, program.functions[e.function].span))?;
    for (f, inferred) in program.functions.iter().zip(&views) {
        if &f.result_view != inferred {
            return Err(Diagnostic::new(
                "E0401",
                "MIR result provenance differs from its contract",
                f.span,
            ));
        }
    }
    let storage = returns::infer_storage(program, &views)
        .map_err(|e| Diagnostic::new("E0401", e.message, program.functions[e.function].span))?;
    let writes = storage_writes::infer(program)
        .map_err(|e| Diagnostic::new("E0401", e.message, program.functions[e.function].span))?;
    for f in &program.functions {
        if *f.storage_writes != writes {
            return Err(Diagnostic::new(
                "E0401",
                "MIR storage effects differ from inferred effects",
                f.span,
            ));
        }
    }
    for f in &program.functions {
        for block in &f.blocks {
            if let Terminator::Evaluate { operation, .. } = &block.terminator {
                let mismatch = match operation {
                    Operation::CallView {
                        function, contract, ..
                    } => views[*function].as_ref() != Some(contract),
                    Operation::CallStorage {
                        function, contract, ..
                    } => &storage[*function] != contract,
                    Operation::Call(function, _) => {
                        views[*function].is_some() || !storage[*function].is_empty()
                    }
                    Operation::CallIndirect(_, _, contract) => {
                        contract.targets.iter().any(|target| {
                            let view_mismatch = match (&views[*target], &contract.view) {
                                (None, None) => false,
                                (Some(actual), Some(declared)) => {
                                    !actual.origins.is_subset(&declared.origins)
                                        || (actual.access == loans::Access::Readonly
                                            && declared.access == loans::Access::Mutable)
                                }
                                _ => true,
                            };
                            view_mismatch
                                || storage[*target].iter().any(|actual| {
                                    !contract.storage.iter().any(|declared| {
                                        actual.path == declared.path
                                            && actual.origins.is_subset(&declared.origins)
                                    })
                                })
                                || writes[*target].iter().any(|actual| {
                                    !contract.writes.iter().any(|declared| {
                                        actual.target == declared.target
                                            && actual.origins.is_subset(&declared.origins)
                                    })
                                })
                        })
                    }
                    _ => false,
                };
                if mismatch {
                    return Err(Diagnostic::new(
                        "E0401",
                        "MIR call erases or changes result provenance",
                        block.span,
                    ));
                }
            }
        }
    }
    for f in &program.functions {
        cleanup_exits(f)?;
        if let Some((block, _)) = dataflow::place_initialization_errors(f).first() {
            return Err(Diagnostic::new(
                "E0401",
                "uninitialized MIR place",
                f.blocks[*block].span,
            ));
        }
        if let Some(error) = borrow_check::check(f).into_iter().next() {
            return Err(borrow_check::diagnostic(f, error, "E0401"));
        }
    }
    Ok(())
}

/// Physical place type, including stored descriptors. A local view has an
/// implicit pointer layer; projections must explicitly cross it.
pub(crate) fn place_type(f: &Function, place: &Place) -> Option<Type> {
    let local = f.locals.get(place.local)?;
    let mut ty = local.ty.clone();
    for (index, projection) in place.projections.iter().enumerate() {
        ty = match (projection, ty) {
            (Projection::Dereference, ty) if index == 0 && local.view.is_some() => ty,
            (Projection::Dereference, Type::StoredView(target)) => *target,
            (Projection::Field(i), Type::Tuple(fields)) => fields.get(*i)?.clone(),
            (Projection::Field(i), Type::Record { fields, .. }) => fields.get(*i)?.ty.clone(),
            (Projection::Field(i), ty @ Type::Class { .. }) => {
                f.classes.get(&ty.class_key())?.fields.get(*i)?.ty.clone()
            }
            (Projection::Variant(i), Type::Enum { variants, .. }) => {
                Type::Tuple(variants.get(*i)?.fields.clone())
            }
            (Projection::ConstantIndex(_), Type::Array(element, _)) => *element,
            (Projection::Index(i), Type::Array(element, _))
                if matches!(f.locals.get(*i)?.ty, Type::Integer(_)) =>
            {
                *element
            }
            _ => return None,
        };
    }
    Some(ty)
}

/// May-live owner roots must reach a storage boundary or the return operand on
/// every exit, including unwind. Partial moves keep their enclosing root live.
fn cleanup_exits(f: &Function) -> Result<(), Diagnostic> {
    use std::collections::VecDeque;
    let owns = |l: usize| f.locals[l].view.is_none() && f.locals[l].ty.owns_storage();
    let mut incoming: Vec<Option<BTreeSet<usize>>> = vec![None; f.blocks.len()];
    incoming[f.entry] = Some(f.parameters.iter().copied().filter(|l| owns(*l)).collect());
    let mut queue = VecDeque::from([f.entry]);
    while let Some(id) = queue.pop_front() {
        let mut live = incoming[id].clone().unwrap();
        let term = &f.blocks[id].terminator;
        if let Terminator::EndStorage { locals, .. } = term {
            for l in locals {
                live.remove(l);
            }
        }
        if let Terminator::Return(Operand::Local(l)) = term {
            live.remove(l);
        }
        if matches!(term, Terminator::Return(_) | Terminator::ResumePanic) && !live.is_empty() {
            return Err(Diagnostic::new(
                "E0401",
                "MIR exit leaks an ownership token without cleanup",
                f.blocks[id].span,
            ));
        }
        for successor in term.successors() {
            let mut next_live = live.clone();
            if let Terminator::Evaluate {
                operation,
                destination,
                next,
                ..
            } = term
            {
                let call = matches!(
                    operation,
                    Operation::Call(..)
                        | Operation::CallIndirect(..)
                        | Operation::CallView { .. }
                        | Operation::CallStorage { .. }
                );
                if successor == *next || call {
                    for p in dataflow::consumed(f, operation) {
                        if p.projections.is_empty() {
                            next_live.remove(&p.local);
                        }
                    }
                }
                if successor == *next && owns(destination.local) {
                    next_live.insert(destination.local);
                }
            }
            match &mut incoming[successor] {
                Some(old) => {
                    let len = old.len();
                    old.extend(next_live);
                    if old.len() != len {
                        queue.push_back(successor);
                    }
                }
                slot @ None => {
                    *slot = Some(next_live);
                    queue.push_back(successor);
                }
            }
        }
    }
    Ok(())
}

fn writable(f: &Function, p: &Place) -> bool {
    let l = &f.locals[p.local];
    let root = if p.projections.first() == Some(&Projection::Dereference) {
        l.view == Some(loans::Access::Mutable)
    } else {
        l.mutable
    };
    root && p.projections.iter().enumerate().all(|(i, projection)| {
        *projection != Projection::Dereference
            || i == 0
            || !matches!(
                place_type(
                    f,
                    &Place {
                        local: p.local,
                        projections: p.projections[..i].to_vec()
                    }
                ),
                Some(Type::StoredView(_))
            )
    })
}

fn operand_type(f: &Function, operand: &Operand) -> Option<Type> {
    match operand {
        Operand::Local(l) => f.locals.get(*l).map(|l| l.ty.clone()),
        Operand::Constant(Value::Integer(i)) => Some(Type::Integer(i.ty())),
        Operand::Constant(Value::Bool(_)) => Some(Type::Bool),
        Operand::Constant(Value::Char(_)) => Some(Type::Char),
        Operand::Constant(Value::F32(_)) => Some(Type::F32),
        Operand::Constant(Value::F64(_)) => Some(Type::F64),
        _ => None,
    }
}
fn verify_raw(f: &Function, op: &Operation, destination: &Place) -> Result<(), &'static str> {
    let result = place_type(f, destination);
    let valid = match op {
        Operation::Unary("storage_allocate", count) => {
            operand_type(f, count) == Some(Type::Integer(nether_semantics::IntegerType::Usize))
                && matches!(&result, Some(Type::Pointer {mutable:true,pointee}) if pointee.raw_scalar() || **pointee==Type::Unit)
        }
        Operation::Unary(
            op @ ("storage_initialize" | "storage_replace" | "storage_take" | "storage_drop"
            | "storage_release"),
            args,
        ) => {
            let Some(Type::Tuple(fields)) = operand_type(f, args) else {
                return Err("invalid MIR storage operands");
            };
            let Some(Type::Pointer {
                mutable: true,
                pointee,
            }) = fields.first()
            else {
                return Err("invalid MIR storage capability");
            };
            let count = if matches!(*op, "storage_initialize" | "storage_replace") {
                3
            } else {
                2
            };
            fields.len() == count
                && fields.get(1) == Some(&Type::Integer(nether_semantics::IntegerType::Usize))
                && (pointee.raw_scalar() || **pointee == Type::Unit)
                && (!matches!(*op, "storage_initialize" | "storage_replace")
                    || fields.get(2) == Some(&**pointee))
                && result
                    == Some(if matches!(*op, "storage_take" | "storage_replace") {
                        (**pointee).clone()
                    } else {
                        Type::Unit
                    })
        }

        Operation::Unary(name @ ("raw_addr" | "raw_addr_mut"), Operand::Address(p)) => {
            let source = place_type(f, p);
            source.as_ref().is_some_and(Type::raw_scalar)
                && result
                    == source.map(|t| Type::Pointer {
                        mutable: *name == "raw_addr_mut",
                        pointee: Box::new(t),
                    })
                && (*name != "raw_addr_mut" || writable(f, p))
        }
        Operation::Unary("raw_addr" | "raw_addr_mut", _) => false,
        Operation::Unary("raw_read", p) => {
            matches!(operand_type(f,p), Some(Type::Pointer { pointee, .. }) if pointee.raw_scalar() && result.as_ref() == Some(&*pointee))
        }
        Operation::Binary("raw_write", p, v) => {
            matches!(operand_type(f,p), Some(Type::Pointer { mutable:true, pointee }) if pointee.raw_scalar() && operand_type(f,v).as_ref() == Some(&*pointee) && result == Some(Type::Unit))
        }
        Operation::Binary("raw_offset", p, n) => {
            matches!(operand_type(f,p), Some(Type::Pointer { pointee, .. }) if pointee.raw_scalar())
                && operand_type(f, p) == result
                && operand_type(f, n) == Some(Type::Integer(nether_semantics::IntegerType::Isize))
        }
        Operation::Cast(p, target)
            if matches!(target, Type::Pointer { .. })
                || matches!(operand_type(f, p), Some(Type::Pointer { .. })) =>
        {
            matches!((operand_type(f,p),target), (Some(Type::Pointer { mutable: a, pointee: x }), Type::Pointer { mutable: b, pointee: y }) if (!*b || a) && x == *y && x.raw_scalar() && result.as_ref() == Some(target))
        }
        _ => true,
    };
    if valid {
        Ok(())
    } else {
        Err("invalid MIR raw pointer type or capability")
    }
}
