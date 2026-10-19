//! Type-directed destruction recipes. Arrays stay compact even for huge lengths;
//! execution consults initialization state before reading any payload or tag.
use crate::hir::{FunctionId, Type};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub struct Destructors {
    entries: BTreeMap<String, FunctionId>,
}
impl Destructors {
    pub fn from_functions(functions: &[crate::hir::Function]) -> Result<Self, &'static str> {
        let mut result = Self::default();
        for (id, function) in functions.iter().enumerate() {
            if function
                .name
                .rsplit_once(".$drop")
                .is_some_and(|(_, suffix)| {
                    suffix.is_empty() || suffix.starts_with("$g") || suffix.starts_with("$entry")
                })
            {
                let parameter = function
                    .parameters
                    .first()
                    .ok_or("destructor requires this")?;
                result.insert(&function.locals[*parameter].ty, id)?;
            }
        }
        Ok(result)
    }

    pub fn insert(&mut self, ty: &Type, entry: FunctionId) -> Result<(), &'static str> {
        let (Type::Record { copy, .. } | Type::Class { copy, .. }) = ty else {
            return Err("destructor requires a record target");
        };
        if *copy {
            return Err("Copy type cannot declare a destructor");
        }
        let key = match ty {
            Type::Class { .. } => ty.class_key(),
            _ => format!("{ty:?}"),
        };
        if self.entries.contains_key(&key) {
            return Err("duplicate destructor");
        }
        self.entries.insert(key, entry);
        Ok(())
    }
    pub fn entry(&self, ty: &Type) -> Option<FunctionId> {
        match ty {
            Type::Record { .. } => self.entries.get(&format!("{ty:?}")).copied(),
            Type::Class { .. } => self.entries.get(&ty.class_key()).copied(),
            _ => None,
        }
    }
    pub(crate) fn class_payload(&self, name: &str, ty: &Type) -> Plan {
        let Type::Record { fields, .. } = ty else {
            panic!("class payload must be record")
        };
        Plan::aggregate(
            self.entries.get(name).copied(),
            fields.iter().map(|f| self.plan(&f.ty)).collect(),
        )
    }
    pub fn plan(&self, ty: &Type) -> Plan {
        match ty {
            Type::Class { .. } => Plan::Class(ty.class_key()),
            Type::Record { fields, .. } => Plan::aggregate(
                self.entry(ty),
                fields.iter().map(|f| self.plan(&f.ty)).collect(),
            ),
            Type::Tuple(fields) => {
                Plan::aggregate(None, fields.iter().map(|t| self.plan(t)).collect())
            }
            Type::Array(element, count) => {
                let element = self.plan(element);
                if element == Plan::None || *count == 0 {
                    Plan::None
                } else {
                    Plan::Array {
                        element: Box::new(element),
                        count: *count,
                    }
                }
            }
            Type::Enum { variants, .. } => {
                let variants: Vec<_> = variants
                    .iter()
                    .map(|v| Plan::aggregate(None, v.fields.iter().map(|t| self.plan(t)).collect()))
                    .collect();
                if variants.iter().all(|v| *v == Plan::None) {
                    Plan::None
                } else {
                    Plan::Enum { variants }
                }
            }
            _ => Plan::None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    Class(String),
    None,
    Aggregate {
        destructor: Option<FunctionId>,
        fields: Vec<(usize, Plan)>,
    },
    Array {
        element: Box<Plan>,
        count: u64,
    },
    Enum {
        variants: Vec<Plan>,
    },
}
impl Plan {
    fn aggregate(destructor: Option<FunctionId>, fields: Vec<Plan>) -> Self {
        let fields: Vec<_> = fields
            .into_iter()
            .enumerate()
            .filter(|(_, p)| *p != Self::None)
            .collect();
        if destructor.is_none() && fields.is_empty() {
            Self::None
        } else {
            Self::Aggregate { destructor, fields }
        }
    }
    pub fn needs_drop(&self) -> bool {
        *self != Self::None
    }
}

/// Logical paths in the independent cleanup reference model. Field numbering
/// is declaration order; array indices and enum payloads remain explicit.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Part {
    Field(usize),
    Element(u64),
    Variant(usize),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Initialization {
    Vacant,
    Partial,
    Constructing,
    Live,
    Finalizing,
}
/// Implementations keep child state readable while their parent is finalizing.
/// No payload/tag may be inspected for a Vacant place.
pub trait Memory {
    type Panic;
    fn class(
        &mut self,
        _name: &str,
        _path: &[Part],
        _pending: &mut Option<Self::Panic>,
    ) -> Result<(), Failure<Self::Panic>> {
        Err(Failure::InvalidLifecycle)
    }
    fn initialization(&self, path: &[Part]) -> Initialization;
    fn begin(&mut self, path: &[Part]);
    fn finish(&mut self, path: &[Part]);
    fn variant(&self, path: &[Part]) -> Option<usize>;
    fn destructor(&mut self, entry: FunctionId, path: &[Part]) -> Result<(), Self::Panic>;
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure<E> {
    InvalidLifecycle,
    InvalidVariant,
    DoublePanic { first: E, second: E },
}
/// Execute one recipe. `pending` retains the first panic while remaining fields
/// are cleaned. A second panic ends execution immediately; native glue aborts.
pub fn execute<M: Memory>(
    plan: &Plan,
    memory: &mut M,
    pending: &mut Option<M::Panic>,
) -> Result<(), Failure<M::Panic>> {
    execute_at(plan, memory, pending, &mut Vec::new())
}
fn execute_at<M: Memory>(
    plan: &Plan,
    memory: &mut M,
    pending: &mut Option<M::Panic>,
    path: &mut Vec<Part>,
) -> Result<(), Failure<M::Panic>> {
    if !plan.needs_drop() {
        return Ok(());
    }
    let state = memory.initialization(path);
    match state {
        Initialization::Vacant => return Ok(()),
        Initialization::Finalizing => return Err(Failure::InvalidLifecycle),
        _ => {}
    }
    // Snapshot the tag before beginning cleanup. No tag read occurs for a vacant
    // enum, even if its backing bytes have never held a valid discriminant.
    let variant = if matches!(plan, Plan::Enum { .. }) {
        memory.variant(path)
    } else {
        None
    };
    if matches!(
        plan,
        Plan::Aggregate {
            destructor: Some(_),
            ..
        }
    ) && state == Initialization::Partial
    {
        return Err(Failure::InvalidLifecycle);
    }
    memory.begin(path);
    match plan {
        Plan::Class(name) => memory.class(name, path, pending)?,
        Plan::None => unreachable!(),
        Plan::Aggregate { destructor, fields } => {
            if let Some(entry) = destructor.filter(|_| state == Initialization::Live) {
                if let Err(second) = memory.destructor(entry, path) {
                    if let Some(first) = pending.take() {
                        return Err(Failure::DoublePanic { first, second });
                    }
                    *pending = Some(second);
                }
            }
            for (index, field) in fields {
                path.push(Part::Field(*index));
                execute_at(field, memory, pending, path)?;
                path.pop();
            }
        }
        Plan::Array { element, count } => {
            for index in 0..*count {
                path.push(Part::Element(index));
                execute_at(element, memory, pending, path)?;
                path.pop();
            }
        }
        Plan::Enum { variants } => {
            let Some(variant) = variant else {
                if state != Initialization::Constructing {
                    return Err(Failure::InvalidVariant);
                }
                memory.finish(path);
                return Ok(());
            };
            let active = variants.get(variant).ok_or(Failure::InvalidVariant)?;
            path.push(Part::Variant(variant));
            execute_at(active, memory, pending, path)?;
            path.pop();
        }
    }
    memory.finish(path);
    Ok(())
}
