use std::collections::{HashMap, HashSet};

use nether_ast::{SelfParam, Symbol};
use nether_resolver::{DefId, Definitions};

use crate::alloc::{alloc_kind, AllocKind};
use crate::ty::Type;

/// One generic/interface constraint with its concrete type arguments.
///
/// Keeping the arguments is essential: `Convert<String>` and
/// `Convert<i32>` are distinct implementations and method signatures
/// declared by a generic interface must be specialized with the chosen
/// arguments before they reach HIR.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GenericBound {
    pub interface: DefId,
    pub args: Vec<Type>,
}

/// A resolved function/method signature — `self`/mutability/generics
/// included, unlike a closure's plain [`Type::Function`].
#[derive(Debug, Clone)]
pub struct FnSig {
    pub self_param: Option<SelfParam>,
    pub params: Vec<ParamSig>,
    pub ret: Type,
    /// This item's own generic parameters: name plus an optional bound
    /// interface, used for call-site bound checking (`check.rs`).
    pub generics: Vec<(Symbol, Option<GenericBound>)>,
}

#[derive(Debug, Clone)]
pub struct ParamSig {
    pub name: Symbol,
    pub mutable: bool,
    pub ty: Type,
}

#[derive(Debug, Clone)]
pub enum TypeShape {
    Struct(Vec<(Symbol, Type)>),
    TupleStruct(Vec<Type>),
    Unit,
}

#[derive(Debug, Clone, Default)]
pub struct EnumSig {
    /// Generic parameters in declaration order.  Payload types below may
    /// contain `Type::Generic` entries referring to these names.
    pub generics: Vec<Symbol>,
    /// Variant payload types, in declared order — this order is exactly
    /// what `nether_resolver::Resolution::EnumVariant`'s index refers to,
    /// since both this crate and `resolver` build it from the same
    /// `EnumDecl::variants` iteration.
    pub variants: Vec<(Symbol, Vec<Type>)>,
}

/// Every signature `typecheck` collected from the module, keyed against
/// `nether_resolver`'s [`DefId`]s so results from both crates can be
/// cross-referenced directly.
#[derive(Default)]
pub struct Signatures {
    pub type_shapes: HashMap<DefId, TypeShape>,
    /// Generic parameters for each `type` declaration, in declaration
    /// order. `type_shapes` keeps the corresponding fields in terms of
    /// these parameters; [`Signatures::type_fields`] performs the
    /// per-instantiation substitution.
    pub type_generics: HashMap<DefId, Vec<Symbol>>,
    /// Bounds corresponding to generic type/enum parameters, in the same
    /// declaration order as `type_generics` / `EnumSig::generics`.
    pub generic_type_bounds: HashMap<DefId, Vec<Option<GenericBound>>>,
    pub enum_sigs: HashMap<DefId, EnumSig>,
    /// Standalone `fn` signatures, keyed by their own `DefId`.
    pub fns: HashMap<DefId, FnSig>,
    /// `impl`/`interface` method signatures, keyed by `(owner type or enum
    /// DefId, method name)` — not by `nether_resolver`'s per-method index,
    /// so this table doesn't need to replicate that index's construction
    /// order; a call site recovers the name from
    /// `Definitions::get(owner).methods[idx]` and looks it up here.
    pub methods: HashMap<(DefId, Symbol), FnSig>,
    /// Fully inherited interface method signatures.
    pub interface_methods: HashMap<(DefId, Symbol), FnSig>,
    /// Generic parameter names and direct parent templates for interfaces.
    pub interface_generics: HashMap<DefId, Vec<Symbol>>,
    pub interface_parents: HashMap<DefId, Vec<GenericBound>>,
    /// `(owner type pattern, interface + arguments)` pairs with a declared
    /// implementation. The owner can contain `Type::Generic` arguments,
    /// e.g. `Boxed<T>`, so one impl applies to every monomorphization.
    pub impls: HashSet<(Type, GenericBound)>,
    /// Type substitutions used by inherited generic-interface defaults.
    /// The interface body is type-checked once in its generic form, then
    /// HIR uses this map when lowering the copy attached to an impl.
    pub default_method_substitutions: HashMap<(DefId, Symbol), HashMap<Symbol, Type>>,
    /// Interface declaration that supplies each inherited default body.
    pub default_method_sources: HashMap<(DefId, Symbol), DefId>,
}

impl Signatures {
    pub fn method(&self, owner: DefId, name: &Symbol) -> Option<&FnSig> {
        self.methods.get(&(owner, name.clone()))
    }

    pub fn satisfies(&self, ty: &Type, bound: &GenericBound) -> bool {
        self.impls.iter().any(|(owner_pattern, implemented)| {
            let mut subst = HashMap::new();
            if !collect_pattern_bindings(owner_pattern, ty, &mut subst) {
                return false;
            }
            let implemented = GenericBound {
                interface: implemented.interface,
                args: implemented
                    .args
                    .iter()
                    .map(|arg| substitute(arg, &subst))
                    .collect(),
            };
            self.bound_satisfies(&implemented, bound)
        })
    }

    pub fn bound_satisfies(&self, actual: &GenericBound, required: &GenericBound) -> bool {
        self.bound_satisfies_inner(actual, required, &mut HashSet::new())
    }

    fn bound_satisfies_inner(
        &self,
        actual: &GenericBound,
        required: &GenericBound,
        visiting: &mut HashSet<GenericBound>,
    ) -> bool {
        if actual == required {
            return true;
        }
        if !visiting.insert(actual.clone()) {
            return false;
        }
        let generics = self
            .interface_generics
            .get(&actual.interface)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let subst: HashMap<Symbol, Type> = generics
            .iter()
            .cloned()
            .zip(actual.args.iter().cloned())
            .collect();
        let found = self
            .interface_parents
            .get(&actual.interface)
            .into_iter()
            .flatten()
            .any(|parent| {
                let parent = GenericBound {
                    interface: parent.interface,
                    args: parent
                        .args
                        .iter()
                        .map(|arg| substitute(arg, &subst))
                        .collect(),
                };
                self.bound_satisfies_inner(&parent, required, visiting)
            });
        visiting.remove(actual);
        found
    }

    /// Returns a struct/tuple-struct's fields after substituting the
    /// concrete arguments carried by its `Type`.
    pub fn type_fields(&self, ty: &Type) -> Option<Vec<Type>> {
        let (id, args) = match ty {
            Type::Struct(id, args) | Type::TupleStruct(id, args) => (*id, args),
            _ => return None,
        };
        let generics = self
            .type_generics
            .get(&id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if generics.len() != args.len() {
            return None;
        }
        let subst: HashMap<Symbol, Type> =
            generics.iter().cloned().zip(args.iter().cloned()).collect();
        match self.type_shapes.get(&id)? {
            TypeShape::Struct(fields) => Some(
                fields
                    .iter()
                    .map(|(_, ty)| substitute(ty, &subst))
                    .collect(),
            ),
            TypeShape::TupleStruct(fields) => {
                Some(fields.iter().map(|ty| substitute(ty, &subst)).collect())
            }
            TypeShape::Unit => Some(Vec::new()),
        }
    }

    pub fn named_type_fields(&self, ty: &Type) -> Option<Vec<(Symbol, Type)>> {
        let id = match ty {
            Type::Struct(id, _) => *id,
            _ => return None,
        };
        let concrete = self.type_fields(ty)?;
        let TypeShape::Struct(fields) = self.type_shapes.get(&id)? else {
            return None;
        };
        Some(
            fields
                .iter()
                .map(|(name, _)| name.clone())
                .zip(concrete)
                .collect(),
        )
    }

    /// Whether owning/copying `ty` requires reference-count work.
    ///
    /// This is deliberately broader than `alloc_kind == Heap`: a
    /// stack/value tuple, enum, camelCase struct, or `weak` pointer can
    /// itself contain managed references and therefore needs a generated
    /// deep retain/drop shim when copied or when its scope ends.
    pub fn has_managed_content(&self, ty: &Type, defs: &Definitions) -> bool {
        match ty {
            Type::String | Type::Array(_) | Type::Function(_, _) | Type::Weak(_) => true,
            Type::Struct(_, _) | Type::TupleStruct(_, _) => {
                if alloc_kind(ty, defs) == AllocKind::Heap {
                    true
                } else {
                    self.type_fields(ty)
                        .unwrap_or_default()
                        .iter()
                        .any(|field| self.has_managed_content(field, defs))
                }
            }
            Type::Tuple(items) => items
                .iter()
                .any(|item| self.has_managed_content(item, defs)),
            Type::Enum(_, _) => self
                .enum_sigs
                .get(match ty {
                    Type::Enum(id, _) => id,
                    _ => unreachable!(),
                })
                .is_some_and(|sig| {
                    sig.variants.iter().enumerate().any(|(variant, _)| {
                        self.enum_payload(ty, variant as u32)
                            .unwrap_or_default()
                            .iter()
                            .any(|field| self.has_managed_content(field, defs))
                    })
                }),
            _ => false,
        }
    }

    /// Returns a variant's payload after substituting the concrete type
    /// arguments carried by `enum_ty`.  Keeping this operation here avoids
    /// every later compiler stage accidentally reading the generic
    /// declaration payload directly (the source of the old
    /// `Option<HeapType>` codegen ICE).
    pub fn enum_payload(&self, enum_ty: &Type, variant: u32) -> Option<Vec<Type>> {
        let Type::Enum(id, args) = enum_ty else {
            return None;
        };
        let sig = self.enum_sigs.get(id)?;
        let (_, payload) = sig.variants.get(variant as usize)?;
        let subst: HashMap<Symbol, Type> = sig
            .generics
            .iter()
            .cloned()
            .zip(args.iter().cloned())
            .collect();
        Some(payload.iter().map(|ty| substitute(ty, &subst)).collect())
    }
}

fn collect_pattern_bindings(
    pattern: &Type,
    concrete: &Type,
    subst: &mut HashMap<Symbol, Type>,
) -> bool {
    match (pattern, concrete) {
        (Type::Generic(name), concrete) => match subst.get(name) {
            Some(existing) => existing == concrete,
            None => {
                subst.insert(name.clone(), concrete.clone());
                true
            }
        },
        (Type::Struct(a_id, a), Type::Struct(b_id, b))
        | (Type::TupleStruct(a_id, a), Type::TupleStruct(b_id, b))
        | (Type::Enum(a_id, a), Type::Enum(b_id, b)) => {
            a_id == b_id
                && a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|(a, b)| collect_pattern_bindings(a, b, subst))
        }
        (Type::Tuple(a), Type::Tuple(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|(a, b)| collect_pattern_bindings(a, b, subst))
        }
        (Type::Array(a), Type::Array(b)) | (Type::Weak(a), Type::Weak(b)) => {
            collect_pattern_bindings(a, b, subst)
        }
        (Type::Function(a_params, a_ret), Type::Function(b_params, b_ret)) => {
            a_params.len() == b_params.len()
                && a_params
                    .iter()
                    .zip(b_params)
                    .all(|(a, b)| collect_pattern_bindings(a, b, subst))
                && collect_pattern_bindings(a_ret, b_ret, subst)
        }
        _ => pattern == concrete,
    }
}

fn substitute(ty: &Type, subst: &HashMap<Symbol, Type>) -> Type {
    match ty {
        Type::Generic(name) => subst.get(name).cloned().unwrap_or_else(|| ty.clone()),
        Type::Struct(id, args) => {
            Type::Struct(*id, args.iter().map(|ty| substitute(ty, subst)).collect())
        }
        Type::TupleStruct(id, args) => {
            Type::TupleStruct(*id, args.iter().map(|ty| substitute(ty, subst)).collect())
        }
        Type::Tuple(items) => Type::Tuple(items.iter().map(|ty| substitute(ty, subst)).collect()),
        Type::Enum(id, args) => {
            Type::Enum(*id, args.iter().map(|ty| substitute(ty, subst)).collect())
        }
        Type::Array(elem) => Type::Array(Box::new(substitute(elem, subst))),
        Type::Function(params, ret) => Type::Function(
            params.iter().map(|ty| substitute(ty, subst)).collect(),
            Box::new(substitute(ret, subst)),
        ),
        Type::Weak(inner) => Type::Weak(Box::new(substitute(inner, subst))),
        _ => ty.clone(),
    }
}
