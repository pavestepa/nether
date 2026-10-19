use nether_core::{
    hir::{RecordField, Type, Variant, VariantShape},
    mir::drop_plan::{Destructors, Plan},
};
use nether_frontend::source::SourceId;
fn record(name: &str, fields: Vec<Type>) -> Type {
    Type::Record {
        copy: false,
        name: name.into(),
        fields: fields
            .into_iter()
            .enumerate()
            .map(|(i, ty)| RecordField {
                name: format!("f{i}"),
                ty,
                private: false,
                source: SourceId(0),
            })
            .collect(),
    }
}
#[test]
fn destructor_precedes_fields_and_resource_free_fields_keep_their_indices() {
    let leaf = record("Leaf", vec![Type::Bool]);
    let parent = record(
        "Parent",
        vec![Type::Bool, leaf.clone(), Type::Unit, leaf.clone()],
    );
    let mut registry = Destructors::default();
    registry.insert(&leaf, 7).unwrap();
    registry.insert(&parent, 9).unwrap();
    let leaf_plan = Plan::Aggregate {
        destructor: Some(7),
        fields: vec![],
    };
    assert_eq!(
        registry.plan(&parent),
        Plan::Aggregate {
            destructor: Some(9),
            fields: vec![(1, leaf_plan.clone()), (3, leaf_plan)]
        }
    );
    assert_eq!(registry.entry(&leaf), Some(7));
}
#[test]
fn enum_and_arrays_preserve_active_payload_and_element_order_without_expansion() {
    let resource = record("Resource", vec![]);
    let mut registry = Destructors::default();
    registry.insert(&resource, 4).unwrap();
    let item = registry.plan(&resource);
    let array = Type::Array(Box::new(resource.clone()), 1_000_000_000);
    assert_eq!(
        registry.plan(&array),
        Plan::Array {
            element: Box::new(item.clone()),
            count: 1_000_000_000
        }
    );
    assert_eq!(
        registry.plan(&Type::Array(Box::new(resource.clone()), 0)),
        Plan::None
    );
    let ty = Type::Enum {
        name: "E".into(),
        copy: false,
        variants: vec![
            Variant {
                name: "Empty".into(),
                shape: VariantShape::Unit,
                fields: vec![],
            },
            Variant {
                name: "Live".into(),
                shape: VariantShape::Tuple,
                fields: vec![Type::Bool, resource],
            },
        ],
    };
    assert_eq!(
        registry.plan(&ty),
        Plan::Enum {
            variants: vec![
                Plan::None,
                Plan::Aggregate {
                    destructor: None,
                    fields: vec![(1, item)]
                }
            ]
        }
    );
}
#[test]
fn invalid_destructor_registration_does_not_replace_existing_entries() {
    let mut registry = Destructors::default();
    let resource = record("Resource", vec![]);
    registry.insert(&resource, 1).unwrap();
    assert!(registry.insert(&resource, 2).is_err());
    assert_eq!(registry.entry(&resource), Some(1));
    assert!(registry.insert(&Type::Bool, 3).is_err());
    let Type::Record { name, fields, .. } = resource else {
        unreachable!()
    };
    assert!(registry
        .insert(
            &Type::Record {
                name,
                fields,
                copy: true
            },
            4
        )
        .is_err());
}

use nether_core::mir::drop_plan::{self, Failure, Initialization, Memory, Part};
use std::collections::BTreeMap;
#[derive(Default)]
struct Model {
    states: BTreeMap<Vec<Part>, Initialization>,
    events: Vec<usize>,
    fail: Vec<usize>,
    tag: usize,
}
impl Memory for Model {
    type Panic = usize;
    fn initialization(&self, path: &[Part]) -> Initialization {
        self.states
            .get(path)
            .copied()
            .unwrap_or(Initialization::Live)
    }
    fn begin(&mut self, path: &[Part]) {
        self.states.insert(path.into(), Initialization::Finalizing);
    }
    fn finish(&mut self, path: &[Part]) {
        self.states.insert(path.into(), Initialization::Vacant);
    }
    fn variant(&self, _: &[Part]) -> Option<usize> {
        (self.tag != usize::MAX).then_some(self.tag)
    }
    fn destructor(&mut self, entry: usize, path: &[Part]) -> Result<(), usize> {
        assert_eq!(self.initialization(path), Initialization::Finalizing);
        self.events.push(entry);
        if self.fail.contains(&entry) {
            Err(entry)
        } else {
            Ok(())
        }
    }
}
fn leaf(entry: usize) -> Plan {
    Plan::Aggregate {
        destructor: Some(entry),
        fields: vec![],
    }
}
#[test]
fn first_panic_continues_fields_and_second_panic_stops_immediately() {
    let plan = Plan::Aggregate {
        destructor: Some(1),
        fields: vec![(0, leaf(2)), (1, leaf(3))],
    };
    let mut memory = Model {
        fail: vec![1],
        ..Model::default()
    };
    let mut pending = None;
    drop_plan::execute(&plan, &mut memory, &mut pending).unwrap();
    assert_eq!(memory.events, [1, 2, 3]);
    assert_eq!(pending, Some(1));
    drop_plan::execute(&plan, &mut memory, &mut pending).unwrap();
    assert_eq!(memory.events, [1, 2, 3]);
    let mut memory = Model {
        fail: vec![1, 2],
        ..Model::default()
    };
    assert_eq!(
        drop_plan::execute(&plan, &mut memory, &mut None),
        Err(Failure::DoublePanic {
            first: 1,
            second: 2
        })
    );
    assert_eq!(memory.events, [1, 2]);
    let mut memory = Model {
        fail: vec![2],
        ..Model::default()
    };
    assert_eq!(
        drop_plan::execute(&plan, &mut memory, &mut Some(99)),
        Err(Failure::DoublePanic {
            first: 99,
            second: 2
        })
    );
    assert_eq!(memory.events, [1, 2]);
}
#[test]
fn partial_aggregates_skip_missing_parts_and_enum_only_drops_active_payload() {
    let plan = Plan::Aggregate {
        destructor: None,
        fields: vec![
            (0, leaf(1)),
            (
                1,
                Plan::Array {
                    element: Box::new(leaf(2)),
                    count: 3,
                },
            ),
        ],
    };
    let mut memory = Model::default();
    memory.states.insert(vec![], Initialization::Partial);
    memory
        .states
        .insert(vec![Part::Field(0)], Initialization::Vacant);
    memory.states.insert(
        vec![Part::Field(1), Part::Element(1)],
        Initialization::Vacant,
    );
    drop_plan::execute(&plan, &mut memory, &mut None).unwrap();
    assert_eq!(memory.events, [2, 2]);
    let plan = Plan::Enum {
        variants: vec![leaf(4), leaf(5)],
    };
    let mut memory = Model {
        tag: 1,
        ..Model::default()
    };
    drop_plan::execute(&plan, &mut memory, &mut None).unwrap();
    assert_eq!(memory.events, [5]);
    let mut memory = Model {
        tag: usize::MAX,
        ..Model::default()
    };
    memory.states.insert(vec![], Initialization::Vacant);
    drop_plan::execute(&plan, &mut memory, &mut None).unwrap();
    assert!(memory.events.is_empty());
}
#[test]
fn partially_moved_user_destructor_and_reentrant_drop_are_invalid() {
    for state in [Initialization::Partial, Initialization::Finalizing] {
        let mut memory = Model::default();
        memory.states.insert(vec![], state);
        assert_eq!(
            drop_plan::execute(&leaf(1), &mut memory, &mut None),
            Err(Failure::InvalidLifecycle)
        );
        assert!(memory.events.is_empty());
    }
}

#[test]
fn unfinished_construction_cleans_fields_without_calling_parent_destructor() {
    let plan = Plan::Aggregate {
        destructor: Some(9),
        fields: vec![(0, leaf(1)), (1, leaf(2))],
    };
    let mut memory = Model::default();
    memory.states.insert(vec![], Initialization::Constructing);
    memory
        .states
        .insert(vec![Part::Field(1)], Initialization::Vacant);
    drop_plan::execute(&plan, &mut memory, &mut Some(55)).unwrap();
    assert_eq!(memory.events, [1]);
    let mut memory = Model {
        tag: usize::MAX,
        ..Model::default()
    };
    memory.states.insert(vec![], Initialization::Constructing);
    drop_plan::execute(
        &Plan::Enum {
            variants: vec![leaf(1)],
        },
        &mut memory,
        &mut Some(55),
    )
    .unwrap();
    assert!(memory.events.is_empty());
}
