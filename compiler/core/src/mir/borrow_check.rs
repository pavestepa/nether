//! Flow-sensitive origins and last-use loan checking for local safe views.
//! View slots carry pointers, while their logical type remains the target type.
use super::{
    dataflow,
    loans::{overlaps, Access},
    *,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
type Bindings = BTreeMap<Place, BTreeSet<BlockId>>;
#[derive(Clone, Default, PartialEq, Eq)]
struct Origin {
    places: BTreeSet<Place>,
    parents: BTreeSet<BlockId>,
    ancestors: BTreeSet<BlockId>,
}
#[derive(Clone, Debug)]
pub struct Error {
    pub loan: Option<BlockId>,
    pub block: BlockId,
    pub message: &'static str,
}
fn resolve(place: &Place, bindings: &Bindings, origins: &[Origin]) -> Origin {
    let mut result = Origin {
        places: BTreeSet::from([Place::local(place.local)]),
        ..Origin::default()
    };
    for projection in &place.projections {
        if *projection != Projection::Dereference {
            result.places = result
                .places
                .into_iter()
                .map(|mut p| {
                    p.projections.push(projection.clone());
                    p
                })
                .collect();
            continue;
        }
        let mut next = Origin::default();
        let mut ancestors: Option<BTreeSet<BlockId>> = None;
        for source in &result.places {
            for loan in bindings
                .iter()
                .filter(|(p, _)| {
                    p.projections.len() == source.projections.len() && compatible_prefix(source, p)
                })
                .flat_map(|(_, loans)| loans)
            {
                let mut through = origins[*loan].ancestors.clone();
                through.insert(*loan);
                through.extend(&result.ancestors);
                ancestors = Some(match ancestors {
                    Some(old) => old.intersection(&through).copied().collect(),
                    None => through,
                });
                next.parents.insert(*loan);
                next.parents.extend(&origins[*loan].parents);
                next.parents.extend(&result.parents);
                next.places.extend(origins[*loan].places.iter().cloned());
            }
        }
        next.ancestors = ancestors.unwrap_or_default();
        result = next;
    }
    result
}

// A parameter's descriptor components refer to external targets even when the
// wrapper itself is passed by value. The symbolic Dereference separates the
// target lifetime from the lifetime of the wrapper slot.
fn parameter_loans(function: &Function) -> Vec<(Place, Place, Access)> {
    fn components(ty: &Type, slot: Place, out: &mut Vec<(Place, Place, Access)>) {
        match ty {
            Type::StoredView(target) => {
                let mut origin = slot.clone();
                origin.projections.push(Projection::Dereference);
                out.push((slot, origin.clone(), Access::Readonly));
                components(target, origin, out);
            }
            Type::Record { fields, .. }
            | Type::Class {
                storage: Some(fields),
                ..
            } => {
                for (i, field) in fields.iter().enumerate() {
                    let mut place = slot.clone();
                    place.projections.push(Projection::Field(i));
                    components(&field.ty, place, out);
                }
            }
            Type::Tuple(fields) => {
                for (i, ty) in fields.iter().enumerate() {
                    let mut place = slot.clone();
                    place.projections.push(Projection::Field(i));
                    components(ty, place, out);
                }
            }
            Type::Array(ty, n) if ty.contains_views() => {
                for i in 0..*n {
                    let mut place = slot.clone();
                    place.projections.push(Projection::ConstantIndex(i));
                    components(ty, place, out);
                }
            }
            Type::Enum { variants, .. } => {
                for (i, variant) in variants.iter().enumerate() {
                    let mut place = slot.clone();
                    place.projections.push(Projection::Variant(i));
                    components(&Type::Tuple(variant.fields.clone()), place, out);
                }
            }
            _ => (),
        }
    }
    let mut loans = Vec::new();
    for p in &function.parameters {
        if let Some(access) = function.locals[*p].view {
            loans.push((Place::local(*p), Place::local(*p), access));
        }
        components(&function.locals[*p].ty, Place::local(*p), &mut loans);
    }
    loans
}

fn merge(target: &mut Bindings, source: &Bindings) -> bool {
    let mut changed = false;
    for (local, loans) in source {
        let entry = target.entry(local.clone()).or_default();
        let before = entry.len();
        entry.extend(loans);
        changed |= entry.len() != before;
    }
    changed
}
/// Flow-sensitive provenance shared by loan checking and interprocedural effects.
pub(crate) struct Provenance {
    incoming: Vec<Option<Bindings>>,
    origins: Vec<Origin>,
    sites: Vec<Option<BlockId>>,
}
impl Provenance {
    pub(crate) fn places(&self, block: BlockId, place: &Place) -> BTreeSet<Place> {
        self.incoming[block]
            .as_ref()
            .map_or_else(BTreeSet::new, |bindings| {
                resolve(place, bindings, &self.origins).places
            })
    }
    pub(crate) fn components(
        &self,
        block: BlockId,
        source: &Place,
    ) -> BTreeMap<Vec<Projection>, BTreeSet<Place>> {
        let mut result = BTreeMap::<Vec<Projection>, BTreeSet<Place>>::new();
        if let Some(bindings) = &self.incoming[block] {
            for source in resolve(source, bindings, &self.origins).places {
                for (slot, loans) in bindings {
                    if compatible_prefix(&source, slot) {
                        let entry = result
                            .entry(slot.projections[source.projections.len()..].to_vec())
                            .or_default();
                        for loan in loans {
                            entry.extend(self.origins[*loan].places.iter().cloned());
                        }
                    }
                }
            }
        }
        result
    }
    pub(super) fn reachable(&self, block: BlockId) -> bool {
        self.incoming[block].is_some()
    }
}
pub(crate) fn provenance(function: &Function) -> Provenance {
    let flow = reachability::analyze(function);
    let count = function.blocks.len();
    let mut incoming = vec![None::<Bindings>; count];
    let mut initial = Bindings::new();
    let mut origins = vec![Origin::default(); count];
    for (slot, origin, _) in parameter_loans(function) {
        let id = origins.len();
        initial.insert(slot, BTreeSet::from([id]));
        origins.push(Origin {
            places: BTreeSet::from([origin]),
            ..Origin::default()
        });
    }
    let mut sites = (0..count).map(Some).collect::<Vec<_>>();
    sites.resize(origins.len(), None);
    let mut call_loans = BTreeMap::<BlockId, Vec<BlockId>>::new();
    for (block, body) in function.blocks.iter().enumerate() {
        if let Terminator::Evaluate { operation, .. } = &body.terminator {
            if let Some((_, contract)) = returns::call_storage(operation) {
                let ids = (0..contract.len())
                    .map(|_| {
                        let id = origins.len();
                        origins.push(Origin::default());
                        sites.push(Some(block));
                        id
                    })
                    .collect();
                call_loans.insert(block, ids);
            }
        }
    }
    let mut write_loans = BTreeMap::<BlockId, Vec<BlockId>>::new();
    for (block, body) in function.blocks.iter().enumerate() {
        if let Terminator::Evaluate { operation, .. } = &body.terminator {
            if let Some((_, writes)) = storage_writes::effects(function, operation) {
                let ids = writes
                    .iter()
                    .map(|_| {
                        let id = origins.len();
                        origins.push(Origin::default());
                        sites.push(Some(block));
                        id
                    })
                    .collect();
                write_loans.insert(block, ids);
            }
        }
    }
    incoming[function.entry] = Some(initial);
    // Revisit all reachable blocks when a loan's resolved provenance expands.
    loop {
        let mut changed_origins = false;
        let mut queue = VecDeque::from([function.entry]);
        let mut visited = BTreeSet::new();
        while let Some(id) = queue.pop_front() {
            let input = incoming[id].clone().unwrap_or_default();
            let mut success = input.clone();
            let mut panic = input.clone();
            let term = &function.blocks[id].terminator;
            if let Terminator::Evaluate {
                operation,
                destination,
                ..
            } = term
            {
                if let Operation::Borrow(source)
                | Operation::CapturePlace(source)
                | Operation::StoreView(source) = operation
                {
                    let resolved = resolve(source, &input, &origins);
                    if !origins[id].parents.contains(&id) && origins[id] != resolved {
                        origins[id] = resolved;
                        changed_origins = true;
                    }
                    success.insert(destination.clone(), BTreeSet::from([id]));
                } else if let Operation::LoadView(source) = operation {
                    let mut loaded = Bindings::new();
                    transfer(source, destination, &input, &origins, &mut loaded);
                    let loans = loaded.values().flatten().copied().collect();
                    success.insert(destination.clone(), loans);
                } else if let Some((arguments, contract)) = returns::call_storage(operation) {
                    for target in resolve(destination, &input, &origins).places {
                        success.retain(|p, _| !covers(&target, p) || parameter_slot(function, p));
                        for (component, loan) in contract.iter().zip(&call_loans[&id]) {
                            let resolved =
                                call_origin(arguments, &component.origins, &input, &origins);
                            if !origins[*loan].parents.contains(loan) && origins[*loan] != resolved
                            {
                                origins[*loan] = resolved;
                                changed_origins = true;
                            }
                            let mut slot = target.clone();
                            slot.projections.extend_from_slice(&component.path);
                            success.insert(slot, BTreeSet::from([*loan]));
                        }
                    }
                } else if let Some((arguments, contract)) = returns::call_view(operation) {
                    let resolved = call_origin(arguments, &contract.origins, &input, &origins);
                    if !origins[id].parents.contains(&id) && origins[id] != resolved {
                        origins[id] = resolved;
                        changed_origins = true;
                    }
                    success.insert(destination.clone(), BTreeSet::from([id]));
                } else {
                    // Replacing a slot removes only its stored descriptors. Moves
                    // and copies transfer the same loans, never the target owner.
                    let targets = resolve(destination, &input, &origins).places;
                    for target in &targets {
                        success.retain(|p, _| !covers(target, p) || parameter_slot(function, p));
                    }
                    for target in &targets {
                        match operation {
                            Operation::Use(Operand::Local(local))
                            | Operation::CompleteClass(Operand::Local(local)) => transfer(
                                &Place::local(*local),
                                target,
                                &input,
                                &origins,
                                &mut success,
                            ),
                            Operation::Copy(source) | Operation::Move(source) => {
                                transfer(source, target, &input, &origins, &mut success)
                            }
                            Operation::Aggregate(values) | Operation::Enum(_, values) => {
                                for (i, value) in values.iter().enumerate() {
                                    if let Operand::Local(local) = value {
                                        let mut field = target.clone();
                                        if let Operation::Enum(variant, _) = operation {
                                            field.projections.push(Projection::Variant(*variant));
                                        }
                                        field.projections.push(
                                            if matches!(
                                                function.locals[destination.local].ty,
                                                Type::Array(..)
                                            ) {
                                                Projection::ConstantIndex(i as u64)
                                            } else {
                                                Projection::Field(i)
                                            },
                                        );
                                        transfer(
                                            &Place::local(*local),
                                            &field,
                                            &input,
                                            &origins,
                                            &mut success,
                                        );
                                    }
                                }
                            }
                            _ => (),
                        }
                    }
                }
            }
            if let Terminator::Evaluate { operation, .. } = term {
                if let Some((arguments, writes)) = storage_writes::effects(function, operation) {
                    for (write, loan) in writes.iter().zip(&write_loans[&id]) {
                        let resolved = call_origin(arguments, &write.origins, &input, &origins);
                        if !origins[*loan].parents.contains(loan) && origins[*loan] != resolved {
                            origins[*loan] = resolved;
                            changed_origins = true;
                        }
                        let Some(Operand::Address(argument)) =
                            arguments.get(write.target.parameter)
                        else {
                            continue;
                        };
                        let mut slot = argument.clone();
                        slot.projections.extend_from_slice(&write.target.path);
                        for target in resolve(&slot, &input, &origins).places {
                            // A may-write contract preserves old dependencies on
                            // both edges. Calls may panic after publishing a view.
                            success.entry(target.clone()).or_default().insert(*loan);
                            panic.entry(target).or_default().insert(*loan);
                        }
                    }
                }
            }
            if let Terminator::Evaluate { operation, .. } = term {
                for source in dataflow::consumed(function, operation) {
                    let sources = resolve(&source, &input, &origins).places;
                    if sources.len() != 1 {
                        continue;
                    }
                    for source in sources {
                        success.retain(|p, _| {
                            !covers(&source, p)
                                || parameter_slot(function, p)
                                || p.projections.contains(&Projection::Dereference)
                        });
                        if storage_writes::call(operation).is_some()
                            || matches!(operation, Operation::CallIndirect(..))
                            || drop_borrows::stages_on_unwind(operation)
                        {
                            panic.retain(|p, _| {
                                !covers(&source, p)
                                    || parameter_slot(function, p)
                                    || p.projections.contains(&Projection::Dereference)
                            });
                        }
                    }
                }
            }
            if let Terminator::EndStorage { locals, .. } = term {
                for local in locals {
                    success.retain(|p, _| p.local != *local);
                }
            }
            visited.insert(id);
            for next in &flow.successors[id] {
                let next = *next;
                let output = match term {
                    Terminator::Evaluate { next: ok, .. } if *ok == next => &success,
                    Terminator::EndStorage { .. } => &success,
                    _ => &panic,
                };
                let changed = match &mut incoming[next] {
                    Some(state) => merge(state, output),
                    slot @ None => {
                        *slot = Some(output.clone());
                        true
                    }
                };
                if changed || !visited.contains(&next) {
                    queue.push_back(next);
                }
            }
        }
        if !changed_origins {
            break;
        }
    }
    Provenance {
        incoming,
        origins,
        sites,
    }
}
pub fn check(function: &Function) -> Vec<Error> {
    if function
        .locals
        .iter()
        .all(|local| local.view.is_none() && !local.ty.contains_views())
    {
        return Vec::new();
    }
    let count = function.blocks.len();
    let provenance = provenance(function);
    let orders =
        (!function.drop_reads.is_empty()).then(|| drop_borrows::orders(function, &provenance));
    let Provenance {
        incoming,
        origins,
        sites,
    } = provenance;
    let parameter_loans = parameter_loans(function);
    let live = dataflow::liveness(function);
    let mut errors = Vec::new();
    for (id, block) in function.blocks.iter().enumerate() {
        if origins
            .iter()
            .enumerate()
            .any(|(loan, origin)| sites[loan] == Some(id) && origin.parents.contains(&loan))
        {
            errors.push(Error {
                loan: None,
                block: id,
                message: "cyclic reborrowing across loop iterations is not yet supported",
            });
        }
        let Some(bindings) = &incoming[id] else {
            continue;
        };
        let mut holders = BTreeMap::<BlockId, BTreeSet<LocalId>>::new();
        for local in &live.live_in[id] {
            for loan in bindings
                .iter()
                .filter(|(place, _)| place.local == *local)
                .flat_map(|(_, loans)| loans)
            {
                holders.entry(*loan).or_default().insert(*local);
            }
        }
        loop {
            let old = holders.clone();
            for (loan, readers) in &old {
                for origin in &origins[*loan].places {
                    for (_, loans) in bindings
                        .iter()
                        .filter(|(stored, _)| compatible_prefix(origin, stored))
                    {
                        for loan in loans {
                            holders.entry(*loan).or_default().extend(readers);
                        }
                    }
                }
            }
            if holders == old {
                break;
            }
        }
        let active = holders.keys().copied().collect::<BTreeSet<_>>();
        if let Terminator::EndStorage { locals, .. } = &block.terminator {
            for loan in &active {
                if origins[*loan].places.iter().any(|p| {
                    locals.contains(&p.local)
                        && !orders.as_ref().is_some_and(|orders| {
                            holders[loan].iter().all(|holder| {
                                locals.contains(holder)
                                    && function.drop_reads.contains(holder)
                                    && orders.cleans_before(function, id, *holder, p.local)
                            })
                        })
                        && !(function.parameters.contains(&p.local)
                            && (function.locals[p.local].view.is_some()
                                || p.projections.contains(&Projection::Dereference)))
                }) {
                    errors.push(Error {
                        loan: Some(*loan),
                        block: id,
                        message: "view outlives its local target",
                    });
                }
            }
        }
        let mut access = |place: &Place, kind: Access, mutation: bool| {
            let resolved = resolve(place, bindings, &origins);
            if resolved.places.is_empty() {
                errors.push(Error {
                    loan: None,
                    block: id,
                    message: "view has no initialized origin",
                });
                return;
            }
            if mutation {
                let mutable = if matches!(place.projections.first(), Some(Projection::Dereference))
                {
                    function.locals[place.local].view == Some(Access::Mutable)
                } else {
                    function.locals[place.local].mutable
                };
                if !mutable {
                    errors.push(Error {
                        loan: None,
                        block: id,
                        message: "mutable access requires a mutable origin",
                    });
                }
            }
            for loan in &active {
                if resolved.ancestors.contains(loan) {
                    continue;
                }
                let capability = if *loan >= count {
                    Some(
                        parameter_loans
                            .get(*loan - count)
                            .map_or(Access::Readonly, |loan| loan.2),
                    )
                } else {
                    let Terminator::Evaluate {
                        destination,
                        operation,
                        ..
                    } = &function.blocks[*loan].terminator
                    else {
                        unreachable!()
                    };
                    if matches!(operation, Operation::CapturePlace(_)) {
                        continue;
                    }
                    if matches!(operation, Operation::StoreView(_)) {
                        Some(Access::Readonly)
                    } else {
                        function.locals[destination.local].view
                    }
                };
                if (kind == Access::Mutable || capability == Some(Access::Mutable))
                    && resolved
                        .places
                        .iter()
                        .any(|a| origins[*loan].places.iter().any(|b| overlaps(a, b)))
                {
                    errors.push(Error {
                        loan: Some(*loan),
                        block: id,
                        message: "access conflicts with a view that is used later",
                    });
                    break;
                }
            }
        };
        if let Terminator::Evaluate {
            operation,
            destination,
            ..
        } = &block.terminator
        {
            match operation {
                Operation::Identity(a, b) => {
                    access(a, Access::Readonly, false);
                    access(b, Access::Readonly, false);
                }
                Operation::StoreView(source) => access(source, Access::Readonly, false),
                Operation::LoadView(source) => access(source, Access::Readonly, false),
                Operation::Borrow(source) => access(
                    source,
                    function.locals[destination.local]
                        .view
                        .unwrap_or(Access::Readonly),
                    function.locals[destination.local].view == Some(Access::Mutable),
                ),
                Operation::Copy(source) | Operation::TagPlace(source) => {
                    access(source, Access::Readonly, false)
                }
                Operation::Move(source) => access(source, Access::Mutable, false),
                Operation::Unary("raw_addr", Operand::Address(source)) => {
                    access(source, Access::Readonly, false)
                }
                Operation::Unary("raw_addr_mut", Operand::Address(source)) => {
                    access(source, Access::Mutable, true)
                }
                _ => (),
            }
            if matches!(operation, Operation::Move(Place { projections, .. }) if projections.first() == Some(&Projection::Dereference))
            {
                errors.push(Error {
                    loan: None,
                    block: id,
                    message: "cannot move ownership out of a borrowed view",
                });
            }
            let owns_operand = |operand: &Operand| matches!(operand, Operand::Local(local) if function.locals[*local].view.is_some());
            let transfers_view = match operation {
                Operation::Use(value) | Operation::CompleteClass(value) => owns_operand(value),
                Operation::Aggregate(values)
                | Operation::Enum(_, values)
                | Operation::Call(_, values)
                | Operation::CallIndirect(_, values, _)
                | Operation::CallStorage {
                    arguments: values, ..
                }
                | Operation::CallView {
                    arguments: values, ..
                } => values.iter().any(owns_operand),
                _ => false,
            };
            if transfers_view {
                errors.push(Error {
                    loan: None,
                    block: id,
                    message: "cannot transfer a borrowed result into owning storage",
                });
            }
            // Root writes initialize/rebind a slot, not necessarily a mutable binding.
            // They still end the previous owner's lifetime and conflict with live loans.
            if !matches!(
                operation,
                Operation::Borrow(_)
                    | Operation::CapturePlace(_)
                    | Operation::CallView { .. }
                    | Operation::LoadView(_)
                    | Operation::StoreView(_)
            ) {
                let resolved = resolve(destination, bindings, &origins);
                for loan in &active {
                    if *loan < count
                        && matches!(
                            &function.blocks[*loan].terminator,
                            Terminator::Evaluate {
                                operation: Operation::CapturePlace(_),
                                ..
                            }
                        )
                    {
                        continue;
                    }
                    if !resolved.ancestors.contains(loan)
                        && resolved
                            .places
                            .iter()
                            .any(|a| origins[*loan].places.iter().any(|b| overlaps(a, b)))
                    {
                        errors.push(Error {
                            loan: Some(*loan),
                            block: id,
                            message: "replacement conflicts with a view that is used later",
                        });
                        break;
                    }
                }
                if matches!(
                    destination.projections.first(),
                    Some(Projection::Dereference)
                ) && function.locals[destination.local].view != Some(Access::Mutable)
                {
                    errors.push(Error {
                        loan: None,
                        block: id,
                        message: "cannot write through a readonly view",
                    });
                }
            }
        }
    }
    errors
}

/// Attach the target, creation site and a reachable later use to a loan error.
/// Source positions survive imports and specialization; MIR local IDs do not
/// appear in user-facing diagnostics.
pub fn diagnostic(
    function: &Function,
    error: Error,
    code: &'static str,
) -> nether_frontend::source::Diagnostic {
    let mut result = nether_frontend::source::Diagnostic::new(
        code,
        error.message,
        function.blocks[error.block].span,
    );
    let Some(loan) = error.loan else {
        return result;
    };
    let provenance = provenance(function);
    for origin in &provenance.origins[loan].places {
        let local = &function.locals[origin.local];
        result = result.with_note(
            local.span,
            format!("borrowed target '{}' originates here", local.name),
        );
    }
    if let Some(block) = provenance.sites[loan].and_then(|site| function.blocks.get(site)) {
        result = result.with_note(block.span, "this view is created here");
    }
    let mut queue = VecDeque::from(function.blocks[error.block].terminator.successors());
    let mut seen = BTreeSet::from([error.block]);
    while let Some(id) = queue.pop_front() {
        if !seen.insert(id) {
            continue;
        }
        if let Some(bindings) = &provenance.incoming[id] {
            if dataflow::uses(&function.blocks[id].terminator)
                .iter()
                .any(|local| {
                    bindings
                        .iter()
                        .filter(|(place, _)| place.local == *local)
                        .flat_map(|(_, loans)| loans)
                        .any(|source| {
                            *source == loan || provenance.origins[*source].parents.contains(&loan)
                        })
                })
            {
                result = result.with_note(
                    function.blocks[id].span,
                    "this later use keeps the view live",
                );
                break;
            }
        }
        queue.extend(function.blocks[id].terminator.successors());
    }
    result
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
fn compatible_prefix(parent: &Place, child: &Place) -> bool {
    parent.local == child.local
        && parent.projections.len() <= child.projections.len()
        && parent
            .projections
            .iter()
            .zip(&child.projections)
            .all(|(a, b)| {
                a == b
                    || matches!(
                        (a, b),
                        (Projection::Index(_), Projection::ConstantIndex(_))
                            | (Projection::ConstantIndex(_), Projection::Index(_))
                            | (Projection::Index(_), Projection::Index(_))
                    )
            })
}
fn transfer(
    source: &Place,
    target: &Place,
    input: &Bindings,
    origins: &[Origin],
    output: &mut Bindings,
) {
    for source in resolve(source, input, origins).places {
        for (place, loans) in input {
            if compatible_prefix(&source, place) {
                let mut field = target.clone();
                field
                    .projections
                    .extend_from_slice(&place.projections[source.projections.len()..]);
                output.entry(field).or_default().extend(loans);
            }
        }
    }
}

fn call_origin(
    arguments: &[Operand],
    paths: &BTreeSet<returns::Origin>,
    bindings: &Bindings,
    origins: &[Origin],
) -> Origin {
    let mut result = Origin::default();
    let mut ancestors: Option<BTreeSet<BlockId>> = None;
    for path in paths {
        let mut argument = match arguments.get(path.parameter) {
            Some(Operand::Address(p)) => p.clone(),
            Some(Operand::Local(p)) => Place::local(*p),
            _ => continue,
        };
        argument.projections.extend_from_slice(&path.path);
        let source = resolve(&argument, bindings, origins);
        ancestors = Some(match ancestors {
            Some(old) => old.intersection(&source.ancestors).copied().collect(),
            None => source.ancestors.clone(),
        });
        result.parents.extend(source.parents);
        result.places.extend(source.places);
    }
    result.ancestors = ancestors.unwrap_or_default();
    result
}

// The pointer binding of a borrowed parameter survives replacement of its
// pointee. Its component bindings describe external storage in the same map.
fn parameter_slot(function: &Function, place: &Place) -> bool {
    place.projections.is_empty()
        && function.parameters.contains(&place.local)
        && function.locals[place.local].view.is_some()
}
