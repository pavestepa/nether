//! Static result provenance. Facts flow through value temporaries before final
//! ownership validation, allowing a borrowed entry to return its input view.
use super::*;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Origin {
    pub parameter: usize,
    pub path: Vec<Projection>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contract {
    pub access: loans::Access,
    pub origins: BTreeSet<Origin>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Component {
    pub path: Vec<Projection>,
    pub origins: BTreeSet<Origin>,
}
pub type StorageContract = Vec<Component>;

pub(crate) fn call_view(op: &Operation) -> Option<(&[Operand], &Contract)> {
    match op {
        Operation::CallView {
            arguments,
            contract,
            ..
        } => Some((arguments, contract)),
        Operation::CallIndirect(
            _,
            args,
            h::CallContract {
                view: Some(view), ..
            },
        ) => Some((args, view)),
        _ => None,
    }
}
pub(super) fn call_storage(op: &Operation) -> Option<(&[Operand], &StorageContract)> {
    match op {
        Operation::CallStorage {
            arguments,
            contract,
            ..
        } => Some((arguments, contract)),
        Operation::CallIndirect(_, args, h::CallContract { storage, .. })
            if !storage.is_empty() =>
        {
            Some((args, storage))
        }
        _ => None,
    }
}
/// Dependencies of descriptors carried inside an owned aggregate result.
/// The wrapper owns its storage but never the targets reached through these paths.
pub fn infer_storage(
    program: &Program,
    views: &[Option<Contract>],
) -> Result<Vec<StorageContract>, Error> {
    program.functions.iter().enumerate().map(|(id, f)| {
        if !f.result.contains_views() || views[id].is_some() {
            return Ok(Vec::new());
        }
        let provenance = borrow_check::provenance(f);
        let mut components = BTreeMap::<Vec<Projection>, BTreeSet<Origin>>::new();
        for (block, body) in f.blocks.iter().enumerate() {
            let Terminator::Return(Operand::Local(local)) = &body.terminator else { continue };
            for (path, origins) in provenance.components(block, &Place::local(*local)) {
                for mut origin in origins {
                    if let Some(index) = origin.projections.iter().position(|p| matches!(p, Projection::Index(_))) {
                        origin.projections.truncate(index);
                    }
                    if origin.projections.len() > 32 {
                        return Err(Error {origin:None,function:id,message:"aggregate result provenance exceeds the supported path limit"});
                    }
                    let parameter = f.parameters.iter().position(|p| *p == origin.local);
                    let Some(parameter) = parameter.filter(|_| f.locals[origin.local].view.is_some()
                        || origin.projections.contains(&Projection::Dereference)) else {
                        return Err(Error {origin:f.locals.get(origin.local).map(|l| l.span),function:id,
                            message:"returned aggregate contains a view of an owner destroyed by this function"});
                    };
                    for path in concrete_paths(&f.result, &path, &f.classes) {
                        components.entry(path).or_default().insert(Origin {parameter,path:origin.projections.clone()});
                    }
                }
            }
        }
        Ok(components.into_iter().map(|(path, origins)| Component {path,origins}).collect())
    }).collect()
}
// Result storage paths must not contain callee-local dynamic index IDs.
// Expand a dynamic write to every potentially affected fixed-array component.
pub(super) fn concrete_paths(
    ty: &Type,
    path: &[Projection],
    classes: &h::Classes,
) -> Vec<Vec<Projection>> {
    let Some((head, tail)) = path.split_first() else {
        return vec![Vec::new()];
    };
    let choices = match (head, ty) {
        (Projection::Index(_), Type::Array(element, count)) => (0..*count)
            .map(|i| (Projection::ConstantIndex(i), (**element).clone()))
            .collect(),
        (Projection::ConstantIndex(i), Type::Array(element, _)) => {
            vec![(Projection::ConstantIndex(*i), (**element).clone())]
        }
        (Projection::Field(i), Type::Record { fields, .. }) => {
            vec![(head.clone(), fields[*i].ty.clone())]
        }
        (Projection::Field(i), Type::Class { .. }) => {
            vec![(head.clone(), classes[&ty.class_key()].fields[*i].ty.clone())]
        }
        (Projection::Field(i), Type::Tuple(fields)) => vec![(head.clone(), fields[*i].clone())],
        (Projection::Variant(i), Type::Enum { variants, .. }) => {
            vec![(head.clone(), Type::Tuple(variants[*i].fields.clone()))]
        }
        (Projection::Dereference, Type::StoredView(target)) => {
            vec![(head.clone(), (**target).clone())]
        }
        _ => return Vec::new(),
    };
    choices
        .into_iter()
        .flat_map(|(head, ty)| {
            concrete_paths(&ty, tail, classes)
                .into_iter()
                .map(move |mut path| {
                    path.insert(0, head.clone());
                    path
                })
        })
        .collect()
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Fact {
    owned: bool,
    readonly: bool,
    origins: BTreeSet<Place>,
}
impl Fact {
    fn owned() -> Self {
        Self {
            owned: true,
            ..Self::default()
        }
    }
    fn merge(&mut self, other: &Self) {
        self.owned |= other.owned;
        self.readonly |= other.readonly;
        self.origins.extend(other.origins.iter().cloned());
        self.normalize();
    }
    fn normalize(&mut self) {
        // Origins are dependencies, not runtime addresses. A containing place
        // subsumes its descendants; widen large recursive path sets to the root.
        let mut counts = BTreeMap::<usize, usize>::new();
        for p in &self.origins {
            *counts.entry(p.local).or_default() += 1;
        }
        let origins = self
            .origins
            .iter()
            .map(|p| {
                let mut p = p.clone();
                if p.projections.len() > 32 || counts[&p.local] > 64 {
                    p.projections.clear();
                }
                if let Some(i) = p
                    .projections
                    .iter()
                    .position(|p| matches!(p, Projection::Index(_)))
                {
                    p.projections.truncate(i);
                }
                p
            })
            .collect::<BTreeSet<_>>();
        self.origins = origins
            .iter()
            .filter(|p| {
                !origins.iter().any(|parent| {
                    parent.local == p.local
                        && parent.projections.len() < p.projections.len()
                        && p.projections.starts_with(&parent.projections)
                })
            })
            .cloned()
            .collect();
    }
    fn project(mut self, path: &[Projection]) -> Self {
        self.origins = self
            .origins
            .into_iter()
            .map(|mut p| {
                // Dynamic callee indices do not escape into caller local numbering.
                // The containing place is a conservative lifetime dependency.
                let end = path
                    .iter()
                    .position(|p| matches!(p, Projection::Index(_)))
                    .unwrap_or(path.len());
                p.projections.extend_from_slice(&path[..end]);
                if p.projections.len() > 32 {
                    p.projections.clear();
                }
                p
            })
            .collect();
        self.normalize();
        self
    }
}
#[derive(Clone, Debug)]
pub struct Error {
    pub origin: Option<Span>,
    pub function: usize,
    pub message: &'static str,
}

pub fn infer(program: &Program) -> Result<Vec<Option<Contract>>, Error> {
    let mut summaries = vec![Fact::default(); program.functions.len()];
    // Every fact is an owned bit, a capability bit and a finite set of symbolic
    // parameter paths. Recursive calls are solved together, never assumed pure.
    for _ in 0..512 {
        let next = program
            .functions
            .iter()
            .map(|f| analyze(f, &summaries))
            .collect::<Vec<_>>();
        if next == summaries {
            return next
                .into_iter()
                .enumerate()
                .map(|(id, fact)| {
                    let f = &program.functions[id];
                    if f.result.copyable() || fact.origins.is_empty() {
                        return Ok(None);
                    }
                    if fact.owned {
                        return Err(Error {
                            origin: None,
                            function: id,
                            message:
                                "one entry cannot return both an owned value and a borrowed view",
                        });
                    }
                    let mut origins = BTreeSet::new();
                    for place in fact.origins {
                        let parameter = place.local;
                        let Some(_) = f.parameters.get(parameter).filter(|p| {
                            f.locals[**p].view.is_some()
                                || place.projections.contains(&Projection::Dereference)
                        }) else {
                            let local = f
                                .parameters
                                .get(parameter)
                                .copied()
                                .unwrap_or_else(|| parameter - f.parameters.len());
                            return Err(Error {
                                origin: f.locals.get(local).map(|l| l.span),
                                function: id,
                                message:
                                    "returned view depends on an owner destroyed by this function",
                            });
                        };
                        origins.insert(Origin {
                            parameter,
                            path: place.projections,
                        });
                    }
                    Ok(Some(Contract {
                        access: if fact.readonly {
                            loans::Access::Readonly
                        } else {
                            loans::Access::Mutable
                        },
                        origins,
                    }))
                })
                .collect();
        }
        summaries = next;
    }
    Err(Error {
        origin: None,
        function: 0,
        message: "recursive result provenance exceeds the supported analysis limit",
    })
}
fn analyze(f: &Function, summaries: &[Fact]) -> Fact {
    if f.result.copyable() {
        return Fact::owned();
    }
    let provenance = borrow_check::provenance(f);
    let flow = reachability::analyze(f);
    let mut initial = BTreeMap::new();
    for p in &f.parameters {
        if f.locals[*p].view.is_none() {
            initial.insert(*p, Fact::owned());
        }
    }
    let mut incoming = vec![None::<BTreeMap<usize, Fact>>; f.blocks.len()];
    incoming[f.entry] = Some(initial);
    let mut queue = VecDeque::from([f.entry]);
    let mut result = Fact::default();
    while let Some(id) = queue.pop_front() {
        let state = incoming[id].clone().unwrap();
        let mut output = state.clone();
        let borrowed = |p: &Place, readonly: bool| {
            let mut fact = Fact {
                readonly,
                ..Fact::default()
            };
            for origin in provenance.places(id, p) {
                // An unresolved call result can still occupy a value temporary
                // in the first checker pass. Follow its inferred value origins.
                if let Some(value) = state
                    .get(&origin.local)
                    .filter(|v| !v.owned && !v.origins.is_empty())
                {
                    fact.merge(&value.clone().project(&origin.projections));
                } else {
                    fact.origins.insert(origin);
                }
            }
            fact.normalize();
            fact
        };
        let read = |p: &Place| {
            if p.projections.first() == Some(&Projection::Dereference) {
                borrowed(p, f.locals[p.local].view == Some(loans::Access::Readonly))
            } else {
                state
                    .get(&p.local)
                    .cloned()
                    .unwrap_or_default()
                    .project(&p.projections)
            }
        };
        let operand = |v: &Operand| match v {
            Operand::Local(l) => read(&Place::local(*l)),
            Operand::Constant(_) => Fact::owned(),
            Operand::Address(p) => borrowed(
                p,
                f.locals[p.local]
                    .view
                    .map_or(!f.locals[p.local].mutable, |a| a == loans::Access::Readonly),
            ),
        };
        match &f.blocks[id].terminator {
            Terminator::Return(value) => result.merge(&operand(value)),
            Terminator::Evaluate {
                operation,
                destination,
                ..
            } if destination.projections.is_empty() => {
                let fact = match operation {
                    Operation::Move(p) => read(p),
                    Operation::Use(v) | Operation::CompleteClass(v) => operand(v),
                    Operation::Call(callee, args)
                    | Operation::CallStorage {
                        function: callee,
                        arguments: args,
                        ..
                    }
                    | Operation::CallView {
                        function: callee,
                        arguments: args,
                        ..
                    } => {
                        let summary = &summaries[*callee];
                        let mut value = Fact {
                            owned: summary.owned,
                            readonly: summary.readonly,
                            origins: BTreeSet::new(),
                        };
                        // Summary roots use parameter positions, independent of
                        // the callee's local numbering.
                        for origin in &summary.origins {
                            if let Some(arg) = args.get(origin.local) {
                                if origin.projections.contains(&Projection::Dereference) {
                                    let mut place = match arg {
                                        Operand::Local(p) => Place::local(*p),
                                        Operand::Address(p) => p.clone(),
                                        Operand::Constant(_) => continue,
                                    };
                                    place.projections.extend_from_slice(&origin.projections);
                                    value.merge(&borrowed(&place, true));
                                } else {
                                    value.merge(&operand(arg).project(&origin.projections));
                                }
                            }
                        }
                        value
                    }
                    Operation::CallIndirect(_, args, contract) => {
                        let mut value = Fact::default();
                        if contract.targets.is_empty() {
                            value = Fact::owned();
                        }
                        for callee in &contract.targets {
                            let summary = &summaries[*callee];
                            let mut next = Fact {
                                owned: summary.owned,
                                readonly: summary.readonly,
                                origins: BTreeSet::new(),
                            };
                            for origin in &summary.origins {
                                if let Some(arg) = args.get(origin.local) {
                                    if origin.projections.contains(&Projection::Dereference) {
                                        let mut place = match arg {
                                            Operand::Local(p) => Place::local(*p),
                                            Operand::Address(p) => p.clone(),
                                            Operand::Constant(_) => continue,
                                        };
                                        place.projections.extend_from_slice(&origin.projections);
                                        next.merge(&borrowed(&place, true));
                                    } else {
                                        next.merge(&operand(arg).project(&origin.projections));
                                    }
                                }
                            }
                            value.merge(&next);
                        }
                        value
                    }
                    Operation::Borrow(p) | Operation::CapturePlace(p) => borrowed(
                        p,
                        f.locals[destination.local].view == Some(loans::Access::Readonly),
                    ),
                    _ => Fact::owned(),
                };
                output.insert(destination.local, fact);
            }
            _ => {}
        }
        for next in &flow.successors[id] {
            let next = *next;
            // A failed operation does not establish a result value. The result
            // analysis follows successful paths; provenance separately models panic.
            if matches!(&f.blocks[id].terminator, Terminator::Evaluate {next: success, ..} if *success != next)
            {
                continue;
            }
            let mut joined = incoming[next].clone().unwrap_or_default();
            for (local, fact) in &output {
                joined.entry(*local).or_default().merge(fact);
            }
            if incoming[next].as_ref() != Some(&joined) {
                incoming[next] = Some(joined);
                queue.push_back(next);
            }
        }
    }
    // Normalize externally observable roots to parameter positions for calls;
    // local roots remain distinguishable beyond the parameter position range.
    result.origins = result
        .origins
        .into_iter()
        .map(|mut p| {
            if let Some(index) = p
                .projections
                .iter()
                .position(|p| matches!(p, Projection::Index(_)))
            {
                p.projections.truncate(index);
            }
            p.local = f
                .parameters
                .iter()
                .position(|id| *id == p.local)
                .unwrap_or(f.parameters.len() + p.local);
            p
        })
        .collect();
    result
}
