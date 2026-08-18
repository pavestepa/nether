use std::collections::{HashMap, HashSet};

use nether_ast::{Expr, NodeId, SelfParam, Symbol, Visibility};
use nether_diagnostics::FileId;
use nether_resolver::{DefId, Definitions};

use crate::alloc::{alloc_kind, AllocKind};
use crate::ty::Type;

pub fn variadic_len_param() -> Symbol {
    Symbol::new("$variadic_len")
}

/// One generic/trait constraint with its concrete type arguments.
///
/// Keeping the arguments is essential: `Convert<String>` and
/// `Convert<i32>` are distinct implementations and method signatures
/// declared by a generic trait must be specialized with the chosen
/// arguments before they reach HIR.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GenericBound {
    pub trait_id: DefId,
    pub args: Vec<Type>,
}

/// Which ownership domain a method receiver belongs to (language-spec
/// §8.4) — the second axis (alongside method name) a concrete owner's
/// method set is keyed by, so `foo(self)` and `foo(: self)` can coexist
/// as genuinely distinct overloads on one type rather than colliding as a
/// duplicate definition. `Static` (no `self` at all) is kept as its own
/// domain rather than folded into `Arc`, preserving today's pre-existing
/// behavior that a static method's name can't collide with an instance
/// method's — this pass only adds a new distinction between `Arc`/`Owned`,
/// it doesn't relax the `Static` one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReceiverDomain {
    /// No `self` parameter — a static method, called through the type.
    Static,
    /// `self` / `mut self` — an ordinary ARC-domain receiver.
    Arc,
    /// `: self` / `: &self` / `: &mut self` — a unique-ownership-domain
    /// receiver.
    Owned,
}

impl ReceiverDomain {
    pub fn of_self_param(self_param: Option<&SelfParam>) -> Self {
        match self_param {
            None => ReceiverDomain::Static,
            Some(SelfParam::ByRef | SelfParam::ByMutRef) => ReceiverDomain::Arc,
            Some(SelfParam::Owned | SelfParam::OwnedRef | SelfParam::OwnedMutRef) => {
                ReceiverDomain::Owned
            }
        }
    }

    /// The domain a *receiver expression's* static type belongs to —
    /// `Type::Unique`/`Type::Ref`/`Type::MutRef` all mean the value is in
    /// the owned domain (a `:&T` is a reference *to* an owned `T` — same
    /// declarations apply), anything else is the ordinary ARC domain.
    /// Never `Static` — that only comes from a signature's own (absent)
    /// `self` parameter ([`Self::of_self_param`]), never from a value's
    /// type. Resolving to the `Owned` bucket for a reference is safe by
    /// itself — it only decides *which method set* to search — the
    /// consuming-through-a-mere-reference case this alone doesn't rule
    /// out is rejected separately, once the specific `self_param` found is
    /// known (`nether_typecheck::check::method::check_method_call_on`).
    pub fn of_receiver_ty(ty: &Type) -> Self {
        match ty {
            Type::Unique(_) | Type::Ref(_) | Type::MutRef(_) => ReceiverDomain::Owned,
            _ => ReceiverDomain::Arc,
        }
    }
}

/// A resolved function/method signature — `self`/mutability/generics
/// included, unlike a closure's plain [`Type::Function`].
#[derive(Debug, Clone)]
pub struct FnSig {
    pub visibility: Visibility,
    pub file: FileId,
    pub self_param: Option<SelfParam>,
    pub params: Vec<ParamSig>,
    pub ret: Type,
    /// This item's own generic parameters and their bounds, used for
    /// call-site bound checking (`check.rs`).
    pub generics: Vec<(Symbol, Vec<GenericBound>)>,
    /// Declared compile-time integer parameters, keyed by generic name.
    pub const_params: HashMap<Symbol, Type>,
    /// Reference-return origin summary: zero-based parameter indices the
    /// returned reference may originate from. Empty for non-reference
    /// returns or while no safe origin can be inferred.
    pub return_origins: Vec<ReturnOrigin>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ReturnOrigin {
    SelfValue,
    Parameter(usize),
}

#[derive(Debug, Clone)]
pub struct ParamSig {
    pub name: Symbol,
    pub mutable: bool,
    /// `true` only when this is the last entry in `FnSig::params` — a
    /// trailing variadic parameter (`args: ...String`). `ty` is then the
    /// *element* type; a call site's trailing arguments (zero or more)
    /// are collected into an `Array<ty>` automatically
    /// (`nether_hir::lower::lower_call`'s variadic handling), so callers
    /// still see one ordinary `Array`-typed value at runtime.
    pub variadic: bool,
    pub ty: Type,
}

/// Every signature declared for one `(owner, method name)` pair — plain
/// generic passthrough (`impl<T> Option<T> { ... }`, or a non-generic
/// owner's ordinary methods) plus any concrete overrides
/// (`impl Option<i32> { ... }`, language-spec-equivalent to Rust
/// specialization but positional-renaming/concrete-only, never partial —
/// see `docs/generics.md` § "Methods on generic types"). At most one
/// entry may exist per distinct concrete argument list; `generic` is the
/// fallback used whenever no `specializations` entry's arguments exactly
/// match the receiver's own. A concrete entry's signature must match
/// `generic`'s (after substituting the concrete arguments) when both
/// exist — enforced when this is built, not here.
#[derive(Debug, Clone, Default)]
pub struct MethodSet {
    pub generic: Option<FnSig>,
    pub specializations: Vec<(Vec<Type>, FnSig)>,
}

impl MethodSet {
    /// The signature to use for a receiver whose owner carries `args` —
    /// an exact-match concrete override if one exists, else the generic
    /// fallback. `args` still containing an unsubstituted `Type::Generic`
    /// (a call site inside another still-generic function) can never
    /// exactly match a `specializations` entry (those are always fully
    /// concrete), so it naturally falls through to `generic` — the same
    /// call, once monomorphized for a concrete instantiation, is
    /// re-resolved through this same method and picks up the override.
    pub fn for_args(&self, args: &[Type]) -> Option<&FnSig> {
        self.specializations
            .iter()
            .find(|(specialized_args, _)| specialized_args.as_slice() == args)
            .map(|(_, sig)| sig)
            .or(self.generic.as_ref())
    }
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

#[derive(Debug, Clone)]
pub struct AssociatedConstSig {
    pub visibility: Visibility,
    pub file: FileId,
    pub ty: Type,
    pub value: Expr,
}

#[derive(Debug, Clone)]
pub struct TraitAssociatedConstSig {
    pub visibility: Visibility,
    pub file: FileId,
    pub ty: Type,
    pub default: Option<Expr>,
}

#[derive(Debug, Clone)]
pub struct AssociatedTypeSig {
    pub visibility: Visibility,
    pub file: FileId,
    pub ty: Type,
}

#[derive(Debug, Clone)]
pub struct TraitAssociatedTypeSig {
    pub visibility: Visibility,
    pub file: FileId,
    pub default: Option<Type>,
}

/// Every signature `typecheck` collected from the module, keyed against
/// `nether_resolver`'s [`DefId`]s so results from both crates can be
/// cross-referenced directly.
#[derive(Default)]
pub struct Signatures {
    pub type_shapes: HashMap<DefId, TypeShape>,
    pub field_visibility: HashMap<(DefId, Symbol), Visibility>,
    /// Generic parameters for each `type` declaration, in declaration
    /// order. `type_shapes` keeps the corresponding fields in terms of
    /// these parameters; [`Signatures::type_fields`] performs the
    /// per-instantiation substitution.
    pub type_generics: HashMap<DefId, Vec<Symbol>>,
    /// Bounds corresponding to generic type/enum parameters, in the same
    /// declaration order as `type_generics` / `EnumSig::generics`.
    pub generic_type_bounds: HashMap<DefId, Vec<Vec<GenericBound>>>,
    pub const_type_params: HashMap<DefId, HashMap<Symbol, Type>>,
    pub enum_sigs: HashMap<DefId, EnumSig>,
    /// Standalone `fn` signatures, keyed by their own `DefId`.
    pub fns: HashMap<DefId, FnSig>,
    /// `impl`/`trait` method signatures, keyed by `(owner type or enum
    /// DefId, method name, receiver domain)` — not by `nether_resolver`'s
    /// per-method index, so this table doesn't need to replicate that
    /// index's construction order; a call site recovers the name from
    /// `Definitions::get(owner).methods[idx]` and looks it up here. The
    /// `ReceiverDomain` component is what lets `foo(self)` and
    /// `foo(: self)` coexist as distinct entries (language-spec §8.4)
    /// rather than colliding — see [`MethodSet`] for why one
    /// owner/name/domain triple can *itself* still hold more than one
    /// signature (generic + concrete specializations).
    pub methods: HashMap<(DefId, Symbol, ReceiverDomain), MethodSet>,
    /// Which `impl` blocks are a concrete specialization
    /// (`impl Option<i32> { ... }`, keyed by the block's own
    /// [`NodeId`]) and, if so, their fully-resolved concrete owner
    /// arguments — the same key `hir` re-derives each block's signatures
    /// and specialization bucket from, without re-lowering `TypeExpr`s
    /// itself. Absent for every other `impl` block (the implicit form, or
    /// the explicit `impl<T> Owner<T> { ... }` generic-passthrough form).
    pub impl_specializations: HashMap<NodeId, Vec<Type>>,
    /// Fully inherited trait method signatures.
    pub trait_methods: HashMap<(DefId, Symbol), FnSig>,
    /// Effective associated constants for concrete owners and declarations
    /// for traits. Trait defaults are copied into an owner when its impl
    /// does not override them.
    pub associated_consts: HashMap<(DefId, Symbol), AssociatedConstSig>,
    pub trait_associated_consts: HashMap<(DefId, Symbol), TraitAssociatedConstSig>,
    pub associated_types: HashMap<(DefId, Symbol), AssociatedTypeSig>,
    pub trait_associated_types: HashMap<(DefId, Symbol), TraitAssociatedTypeSig>,
    /// Generic parameter names and direct parent templates for traits.
    pub trait_generics: HashMap<DefId, Vec<Symbol>>,
    pub trait_parents: HashMap<DefId, Vec<GenericBound>>,
    /// `(owner type pattern, trait + arguments)` pairs with a declared
    /// implementation. The owner can contain `Type::Generic` arguments,
    /// e.g. `Boxed<T>`, so one impl applies to every monomorphization.
    pub impls: HashSet<(Type, GenericBound)>,
    /// Type substitutions used by inherited generic-trait defaults.
    /// The trait body is type-checked once in its generic form, then
    /// HIR uses this map when lowering the copy attached to an impl.
    /// Keyed the same 3-tuple way as `methods` — two different traits
    /// could require the same method name in different domains on one
    /// owner, and each needs its own default tracked separately.
    pub default_method_substitutions:
        HashMap<(DefId, Symbol, ReceiverDomain), HashMap<Symbol, Type>>,
    /// Trait declaration that supplies each inherited default body.
    pub default_method_sources: HashMap<(DefId, Symbol, ReceiverDomain), DefId>,
}

impl Signatures {
    pub fn normalize_associated(&self, ty: &Type) -> Type {
        self.normalize_associated_inner(ty, &mut HashSet::new())
    }

    fn normalize_associated_inner(
        &self,
        ty: &Type,
        visiting: &mut HashSet<(DefId, Symbol)>,
    ) -> Type {
        match ty {
            Type::Associated(owner, name) => {
                let owner = self.normalize_associated_inner(owner, visiting);
                let (id, args) = match &owner {
                    Type::Struct(id, args) | Type::TupleStruct(id, args) | Type::Enum(id, args) => {
                        (*id, args.as_slice())
                    }
                    _ => return Type::Associated(Box::new(owner), name.clone()),
                };
                let key = (id, name.clone());
                let Some(signature) = self.associated_types.get(&key) else {
                    return Type::Associated(Box::new(owner), name.clone());
                };
                if !visiting.insert(key.clone()) {
                    return Type::Error;
                }
                let generic_names = self
                    .type_generics
                    .get(&id)
                    .cloned()
                    .or_else(|| self.enum_sigs.get(&id).map(|sig| sig.generics.clone()))
                    .unwrap_or_default();
                let subst = generic_names
                    .into_iter()
                    .zip(args.iter().cloned())
                    .collect::<HashMap<_, _>>();
                let projected = crate::check::substitute_generic(&signature.ty, &subst);
                let result = self.normalize_associated_inner(&projected, visiting);
                visiting.remove(&key);
                result
            }
            Type::Struct(id, args) => Type::Struct(
                *id,
                args.iter()
                    .map(|arg| self.normalize_associated_inner(arg, visiting))
                    .collect(),
            ),
            Type::TupleStruct(id, args) => Type::TupleStruct(
                *id,
                args.iter()
                    .map(|arg| self.normalize_associated_inner(arg, visiting))
                    .collect(),
            ),
            Type::Enum(id, args) => Type::Enum(
                *id,
                args.iter()
                    .map(|arg| self.normalize_associated_inner(arg, visiting))
                    .collect(),
            ),
            Type::Tuple(items) => Type::Tuple(
                items
                    .iter()
                    .map(|item| self.normalize_associated_inner(item, visiting))
                    .collect(),
            ),
            Type::Array(inner) => {
                Type::Array(Box::new(self.normalize_associated_inner(inner, visiting)))
            }
            Type::FixedArray(element, length) => Type::FixedArray(
                Box::new(self.normalize_associated_inner(element, visiting)),
                Box::new(self.normalize_associated_inner(length, visiting)),
            ),
            Type::Weak(inner) => {
                Type::Weak(Box::new(self.normalize_associated_inner(inner, visiting)))
            }
            Type::Unique(inner) => {
                Type::Unique(Box::new(self.normalize_associated_inner(inner, visiting)))
            }
            Type::Ref(inner) => {
                Type::Ref(Box::new(self.normalize_associated_inner(inner, visiting)))
            }
            Type::MutRef(inner) => {
                Type::MutRef(Box::new(self.normalize_associated_inner(inner, visiting)))
            }
            Type::Function(params, ret) => Type::Function(
                params
                    .iter()
                    .map(|param| self.normalize_associated_inner(param, visiting))
                    .collect(),
                Box::new(self.normalize_associated_inner(ret, visiting)),
            ),
            _ => ty.clone(),
        }
    }

    /// Whether `ty` opts into compiler-derived structural equality and all
    /// of its fields can participate in that comparison.
    pub fn can_derive_eq(&self, ty: &Type, defs: &Definitions) -> bool {
        let ty = match ty {
            Type::Unique(inner) => inner.as_ref(),
            _ => ty,
        };
        self.can_derive_eq_inner(ty, defs, &mut HashSet::new())
    }

    pub fn declares_eq(&self, ty: &Type, defs: &Definitions) -> bool {
        let ty = match ty {
            Type::Unique(inner) => inner.as_ref(),
            _ => ty,
        };
        self.implements_marker(ty, defs, "Eq")
    }

    fn can_derive_eq_inner(
        &self,
        ty: &Type,
        defs: &Definitions,
        visiting: &mut HashSet<Type>,
    ) -> bool {
        let (Type::Struct(_, _) | Type::TupleStruct(_, _)) = ty else {
            return false;
        };
        if !visiting.insert(ty.clone()) {
            return false;
        }
        let implements_eq = self.implements_marker(ty, defs, "Eq");
        let result = implements_eq
            && self.type_fields(ty).is_some_and(|fields| {
                fields
                    .iter()
                    .all(|field| eq_field_is_structural(self, field, defs, visiting))
            });
        visiting.remove(ty);
        result
    }

    fn implements_marker(&self, ty: &Type, defs: &Definitions, name: &str) -> bool {
        let Some((trait_id, _)) = defs.iter().find(|(_, def)| {
            def.kind == nether_resolver::DefKind::Trait && def.name.as_str() == name
        }) else {
            return false;
        };
        self.satisfies(
            ty,
            &GenericBound {
                trait_id,
                args: Vec::new(),
            },
        )
    }

    /// Whether `ty` opts into compiler-derived deterministic structural
    /// hashing. Floating-point fields are deliberately excluded until the
    /// language specifies normalization of NaNs and signed zero.
    pub fn can_derive_hash(&self, ty: &Type, defs: &Definitions) -> bool {
        let ty = match ty {
            Type::Unique(inner) => inner.as_ref(),
            _ => ty,
        };
        self.can_derive_hash_inner(ty, defs, &mut HashSet::new())
    }

    fn can_derive_hash_inner(
        &self,
        ty: &Type,
        defs: &Definitions,
        visiting: &mut HashSet<Type>,
    ) -> bool {
        let (Type::Struct(_, _) | Type::TupleStruct(_, _)) = ty else {
            return false;
        };
        if !visiting.insert(ty.clone()) {
            return false;
        }
        let result = self.implements_marker(ty, defs, "Hash")
            && self.type_fields(ty).is_some_and(|fields| {
                fields
                    .iter()
                    .all(|field| hash_field_is_structural(self, field, defs, visiting))
            });
        visiting.remove(ty);
        result
    }

    /// Whether `ty` may be structurally copied into a fresh unique heap
    /// allocation for `to<:T>(value)`. The outer type must explicitly
    /// implement the compiler-known `Clone` marker. Unique fields remain
    /// excluded until recursive user-defined clone bodies are available:
    /// bit-copying one would create two owners of the same allocation.
    pub fn can_clone_to_unique(&self, ty: &Type, defs: &Definitions) -> bool {
        self.can_clone_to_unique_inner(ty, defs, &mut HashSet::new())
    }

    fn can_clone_to_unique_inner(
        &self,
        ty: &Type,
        defs: &Definitions,
        visiting: &mut HashSet<Type>,
    ) -> bool {
        let (Type::Struct(_, _) | Type::TupleStruct(_, _)) = ty else {
            return false;
        };
        if !visiting.insert(ty.clone()) {
            return false;
        }
        let Some((clone_id, _)) = defs.iter().find(|(_, def)| {
            def.kind == nether_resolver::DefKind::Trait && def.name.as_str() == "Clone"
        }) else {
            return false;
        };
        let bound = GenericBound {
            trait_id: clone_id,
            args: Vec::new(),
        };
        let implements_clone = self.satisfies(ty, &bound);
        let has_override = self.has_user_clone_candidate(ty);
        let cloneable = implements_clone
            && if has_override {
                self.has_user_clone_method(ty)
            } else {
                self.type_fields(ty).is_some_and(|fields| {
                    fields
                        .iter()
                        .all(|field| clone_field_is_structural(self, field, defs, visiting))
                })
            };
        visiting.remove(ty);
        cloneable
    }

    /// A user override of structural cloning. For now it is deliberately
    /// non-generic: `clone(: &self): Owner`, with no additional parameters.
    /// The returned `:Owner` already owns its allocation, so lowering can
    /// call this method directly without a synthesized clone shim.
    pub fn has_user_clone_method(&self, ty: &Type) -> bool {
        let Some(owner) = type_owner(ty) else {
            return false;
        };
        self.method(owner, &Symbol::new("clone"), ReceiverDomain::Owned)
            .is_some_and(|signature| {
                signature.self_param == Some(SelfParam::OwnedRef)
                    && signature.params.is_empty()
                    && signature.generics.is_empty()
                    && signature.ret == Type::Unique(Box::new(ty.clone()))
            })
    }

    pub fn has_user_clone_candidate(&self, ty: &Type) -> bool {
        type_owner(ty).is_some_and(|owner| {
            self.methods
                .contains_key(&(owner, Symbol::new("clone"), ReceiverDomain::Owned))
        })
    }

    /// The generic/non-specialized signature for `(owner, name, domain)` —
    /// every caller that isn't resolving an actual instance-method call
    /// site (static-member calls, trait-conformance checks, `Into<String>`
    /// lookups) uses this with an explicit domain it already knows;
    /// specialization is deliberately scoped to instance methods only
    /// (`docs/generics.md`), so a static method always has exactly this
    /// one signature.
    pub fn method(&self, owner: DefId, name: &Symbol, domain: ReceiverDomain) -> Option<&FnSig> {
        self.methods
            .get(&(owner, name.clone(), domain))?
            .generic
            .as_ref()
    }

    /// Looks up `(owner, name)` across every [`ReceiverDomain`], for the
    /// one call site (`static_member_call_or_value`) that needs to give a
    /// precise "this is an instance method, not static" diagnostic even
    /// when it doesn't yet know which domain the (possibly wrong-kind-of)
    /// method actually lives in. `Static` can never coexist with `Arc`/
    /// `Owned` for the same `(owner, name)` (still a hard duplicate-
    /// definition error, unchanged by this pass), so at most one domain
    /// ever matches in practice.
    pub fn method_any_domain(&self, owner: DefId, name: &Symbol) -> Option<&FnSig> {
        [
            ReceiverDomain::Static,
            ReceiverDomain::Arc,
            ReceiverDomain::Owned,
        ]
        .into_iter()
        .find_map(|domain| self.method(owner, name, domain))
    }

    /// The signature to use for an instance-method call whose receiver's
    /// owner carries `args` — see [`MethodSet::for_args`].
    pub fn method_for(
        &self,
        owner: DefId,
        name: &Symbol,
        domain: ReceiverDomain,
        args: &[Type],
    ) -> Option<&FnSig> {
        self.methods
            .get(&(owner, name.clone(), domain))?
            .for_args(args)
    }

    pub fn satisfies(&self, ty: &Type, bound: &GenericBound) -> bool {
        self.impls.iter().any(|(owner_pattern, implemented)| {
            let mut subst = HashMap::new();
            if !collect_pattern_bindings(owner_pattern, ty, &mut subst) {
                return false;
            }
            let implemented = GenericBound {
                trait_id: implemented.trait_id,
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
            .trait_generics
            .get(&actual.trait_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let subst: HashMap<Symbol, Type> = generics
            .iter()
            .cloned()
            .zip(actual.args.iter().cloned())
            .collect();
        let found = self
            .trait_parents
            .get(&actual.trait_id)
            .into_iter()
            .flatten()
            .any(|parent| {
                let parent = GenericBound {
                    trait_id: parent.trait_id,
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
            Type::String
            | Type::Array(_)
            | Type::Function(_, _)
            | Type::Weak(_)
            | Type::Any(_, _)
            | Type::Some(_, _) => true,
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
            Type::FixedArray(element, _) => self.has_managed_content(element, defs),
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

fn eq_field_is_structural(
    sigs: &Signatures,
    ty: &Type,
    defs: &Definitions,
    visiting: &mut HashSet<Type>,
) -> bool {
    match ty {
        Type::Primitive(_) => true,
        Type::Unique(inner) => eq_field_is_structural(sigs, inner, defs, visiting),
        Type::Struct(_, _) | Type::TupleStruct(_, _) => {
            sigs.can_derive_eq_inner(ty, defs, visiting)
        }
        Type::Tuple(items) => items
            .iter()
            .all(|item| eq_field_is_structural(sigs, item, defs, visiting)),
        Type::FixedArray(element, _) => eq_field_is_structural(sigs, element, defs, visiting),
        _ => false,
    }
}

fn hash_field_is_structural(
    sigs: &Signatures,
    ty: &Type,
    defs: &Definitions,
    visiting: &mut HashSet<Type>,
) -> bool {
    match ty {
        Type::Primitive(kind) => !kind.is_float(),
        Type::String => true,
        Type::Unique(inner) => hash_field_is_structural(sigs, inner, defs, visiting),
        Type::Struct(_, _) | Type::TupleStruct(_, _) => {
            sigs.can_derive_hash_inner(ty, defs, visiting)
        }
        Type::Tuple(items) => items
            .iter()
            .all(|item| hash_field_is_structural(sigs, item, defs, visiting)),
        Type::FixedArray(element, _) => hash_field_is_structural(sigs, element, defs, visiting),
        _ => false,
    }
}

fn type_owner(ty: &Type) -> Option<DefId> {
    match ty {
        Type::Struct(owner, _) | Type::TupleStruct(owner, _) => Some(*owner),
        _ => None,
    }
}

fn clone_field_is_structural(
    sigs: &Signatures,
    ty: &Type,
    defs: &Definitions,
    visiting: &mut HashSet<Type>,
) -> bool {
    match ty {
        Type::Unique(inner) if alloc_kind(inner, defs) == AllocKind::Heap => {
            sigs.can_clone_to_unique_inner(inner, defs, visiting)
        }
        Type::Unique(inner) => matches!(inner.as_ref(), Type::Primitive(_)),
        Type::Generic(_) | Type::Error => false,
        Type::Tuple(items) => items
            .iter()
            .all(|item| clone_field_is_structural(sigs, item, defs, visiting)),
        Type::FixedArray(element, _) => clone_field_is_structural(sigs, element, defs, visiting),
        Type::Struct(_, _) | Type::TupleStruct(_, _)
            if alloc_kind(ty, defs) == AllocKind::Stack =>
        {
            sigs.type_fields(ty).is_some_and(|fields| {
                fields
                    .iter()
                    .all(|field| clone_field_is_structural(sigs, field, defs, visiting))
            })
        }
        Type::Enum(id, _) => sigs.enum_sigs.get(id).is_some_and(|signature| {
            signature.variants.iter().enumerate().all(|(variant, _)| {
                sigs.enum_payload(ty, variant as u32).is_some_and(|fields| {
                    fields
                        .iter()
                        .all(|field| clone_field_is_structural(sigs, field, defs, visiting))
                })
            })
        }),
        Type::Struct(_, _) | Type::TupleStruct(_, _) => true,
        Type::Array(_) | Type::Weak(_) | Type::String | Type::Function(_, _) => true,
        Type::Ref(_) | Type::MutRef(_) => false,
        _ => true,
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
        (Type::FixedArray(a_element, a_length), Type::FixedArray(b_element, b_length)) => {
            collect_pattern_bindings(a_element, b_element, subst)
                && collect_pattern_bindings(a_length, b_length, subst)
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
        Type::FixedArray(element, length) => Type::FixedArray(
            Box::new(substitute(element, subst)),
            Box::new(substitute(length, subst)),
        ),
        Type::Function(params, ret) => Type::Function(
            params.iter().map(|ty| substitute(ty, subst)).collect(),
            Box::new(substitute(ret, subst)),
        ),
        Type::Weak(inner) => Type::Weak(Box::new(substitute(inner, subst))),
        _ => ty.clone(),
    }
}
