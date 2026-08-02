use std::collections::HashMap;

use nether_ast::{BinaryOp, Literal, SelfParam, Symbol, UnaryOp};
use nether_resolver::DefId;
use nether_typecheck::{GenericBound, Signatures, Type};

/// Identifies one function or method for the lifetime of one lowered
/// [`HirModule`] — minted fresh here rather than reusing
/// `nether_resolver::DefId` (which only identifies *standalone* `fn`s) or
/// `nether_typecheck::Signatures`' `(DefId, Symbol)` method keys (which
/// can't name a value directly), since `hir` is the first stage that
/// needs to refer to *any* callable — free function or method — uniformly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HirFnId(pub(crate) u32);

/// Identifies one local variable within a single [`HirFunction`] body —
/// including synthesized temporaries introduced by desugaring (e.g. a
/// `for` loop's hidden index variable) that have no counterpart in the
/// original source. Distinct from `nether_resolver::LocalId`: that space
/// only covers bindings the programmer wrote; this one is minted by
/// `lower.rs` and covers those *plus* every synthesized temporary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HirLocalId(pub(crate) u32);

impl HirLocalId {
    /// Dense per-function index used by downstream lowering tables.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Every function/method in the module, plus the structural type
/// information (`nether_typecheck::Signatures`) needed to make sense of
/// them — carried forward as-is rather than re-wrapped, since it's already
/// exactly the desugared, structural form `monomorphization`/`mir` need.
pub struct HirModule {
    pub signatures: Signatures,
    pub fns: Vec<HirFunction>,
    /// Standalone functions, by name.
    pub fn_by_name: HashMap<Symbol, HirFnId>,
    /// Standalone functions by their module-qualified resolver identity.
    pub fn_by_def: HashMap<DefId, HirFnId>,
    /// `impl`/`interface` methods (including inherited interface
    /// defaults), by `(owner type/enum DefId, method name)` — mirrors
    /// `Signatures::methods`' key shape and, like it, one owner/name pair
    /// can hold both a generic body and concrete-specialization overrides
    /// (see [`MethodFnSet`]).
    pub methods: HashMap<(DefId, Symbol), MethodFnSet>,
    /// `Array`'s own `DefId`, if this module was compiled with the bundled
    /// prelude (`None` for a prelude-less test fixture, which can still
    /// use array *literals* — those need no declaration at all). Unlike
    /// `Struct`/`TupleStruct`/`Enum`, a `Type::Array(_)` value carries no
    /// `DefId` of its own, so any stage that needs to resolve a method
    /// call on an `Array` receiver back to an owner
    /// (`monomorphization::owner_def_id`) needs this looked up once and
    /// threaded through rather than re-derived from the `Type`.
    pub array_owner: Option<DefId>,
}

/// Mirrors `nether_typecheck::sig::MethodSet`, one level down: a
/// [`HirFnId`] (a lowered function body) in place of each `FnSig`.
/// `monomorphization` picks between `generic` and an exact-match
/// `specializations` entry using the receiver's own *substituted*
/// argument types — see that crate's handling of
/// [`HirExprKind::CallMethod`].
#[derive(Debug, Clone, Default)]
pub struct MethodFnSet {
    pub generic: Option<HirFnId>,
    pub specializations: Vec<(Vec<Type>, HirFnId)>,
}

impl MethodFnSet {
    /// The body to run for a receiver whose (already fully substituted,
    /// concrete — `monomorphization` only ever calls this post-substitution)
    /// owner carries `args` — mirrors
    /// `nether_typecheck::sig::MethodSet::for_args`.
    pub fn for_args(&self, args: &[Type]) -> Option<HirFnId> {
        self.specializations
            .iter()
            .find(|(specialized_args, _)| specialized_args.as_slice() == args)
            .map(|(_, id)| *id)
            .or(self.generic)
    }
}

impl HirModule {
    pub fn get(&self, id: HirFnId) -> &HirFunction {
        &self.fns[id.0 as usize]
    }
}

pub struct HirFunction {
    pub id: HirFnId,
    pub name: Symbol,
    /// `Some` for a method (the type/enum it belongs to), `None` for a
    /// standalone function.
    pub owner: Option<DefId>,
    pub self_param: Option<SelfParam>,
    /// The owner type including generic arguments (`Boxed<T>`, not a raw
    /// `Boxed`). Monomorphization substitutes this alongside parameters.
    pub self_ty: Option<Type>,
    /// `self`'s own [`HirLocalId`] when `self_param.is_some()` — reserved
    /// up front during lowering but, unlike an ordinary parameter, not
    /// listed in `params` (mirroring `nether_typecheck::FnSig`'s own
    /// self/params split). Exposed as its own field (rather than left
    /// only implicit in whatever `HirExprKind::Local` nodes the body
    /// happens to reference) because a later stage that needs to bind the
    /// receiver argument at a call boundary — `mir`, wiring up a
    /// function's entry block — has no other way to discover which local
    /// that is.
    pub self_local: Option<HirLocalId>,
    pub generics: Vec<(Symbol, Option<GenericBound>)>,
    pub params: Vec<HirParam>,
    pub ret: Type,
    pub body: HirExpr,
}

#[derive(Debug, Clone)]
pub struct HirParam {
    pub local: HirLocalId,
    pub name: Symbol,
    pub mutable: bool,
    pub ty: Type,
}

#[derive(Debug, Clone)]
pub struct HirCapture {
    pub local: HirLocalId,
    pub ty: Type,
}

/// One lowered expression: a smaller, uniform node set than
/// `nether_ast::Expr`, with its type baked in directly (read from
/// `nether_typecheck::TypedTables` at lowering time, not re-inferred).
#[derive(Debug, Clone)]
pub struct HirExpr {
    pub kind: HirExprKind,
    pub ty: Type,
}

#[derive(Debug, Clone)]
pub enum HirExprKind {
    Literal(Literal),
    Local(HirLocalId),
    /// A reference to a function/method value that isn't being called
    /// directly here. Monomorphization turns it into a closure-ABI adapter.
    FnRef(HirFnId),
    Unit,
    Tuple(Vec<HirExpr>),
    Array(Vec<HirExpr>),
    /// The desugared form of a template string (language-spec §2.3):
    /// concatenates string-typed pieces in order.
    Concat(Vec<HirExpr>),
    /// Wraps a non-`String` piece of an interpolated template string —
    /// the desugared stand-in for an implicit `Into<String>` conversion
    /// (see this crate's module docs for why it isn't bound-checked yet).
    ToString(Box<HirExpr>),
    Unary {
        op: UnaryOp,
        expr: Box<HirExpr>,
    },
    Binary {
        op: BinaryOp,
        lhs: Box<HirExpr>,
        rhs: Box<HirExpr>,
    },
    Assign {
        target: Box<HirExpr>,
        value: Box<HirExpr>,
    },
    /// Calling a first-class function/closure value whose callee isn't
    /// statically known to be one specific `HirFnId`.
    Call {
        callee: Box<HirExpr>,
        args: Vec<HirExpr>,
    },
    /// Calling one specific, statically-known function or method — the
    /// unified desugared form of `Type.method(...)`, `value.method(...)`,
    /// and a direct call to a standalone `fn` (language-spec §10:
    /// `typecheck` already determined exactly which callable this is).
    CallStatic {
        fn_id: HirFnId,
        /// Concrete generic arguments in the target function's declared
        /// order. Empty for non-generic calls.
        generic_args: Vec<Type>,
        args: Vec<HirExpr>,
    },
    /// `println`/`print` — kept distinct since they're variadic and have
    /// no `HirFnId`/[`nether_typecheck::FnSig`] of their own.
    CallBuiltin {
        name: Symbol,
        args: Vec<HirExpr>,
    },
    /// A named-field access already resolved to its position within the
    /// struct's declared field order.
    Field {
        base: Box<HirExpr>,
        index: u32,
    },
    Index {
        base: Box<HirExpr>,
        index: Box<HirExpr>,
    },
    /// Constructs a struct or tuple-struct. Both source syntaxes —
    /// `Dog { name }` and `Point(x, y)` — unify to this one positional
    /// -field shape (language-spec/architecture: tuple-struct construction
    /// desugars to the same shape as struct-literal construction).
    Construct {
        ty: DefId,
        fields: Vec<HirExpr>,
    },
    ConstructVariant {
        enum_id: DefId,
        variant: u32,
        payload: Vec<HirExpr>,
    },
    /// A method call on a value whose type is still an unsubstituted
    /// generic parameter — resolvable only once `monomorphization`
    /// substitutes a concrete type for it. Kept
    /// distinct from [`HirExprKind::CallStatic`] since there is no single
    /// `HirFnId` to call until then; `bound_interface` names which
    /// interface declares `method_name`.
    CallGenericMethod {
        receiver: Box<HirExpr>,
        bound_interface: DefId,
        method_name: Symbol,
        is_static: bool,
        generic_args: Vec<Type>,
        args: Vec<HirExpr>,
    },
    /// `Array<T>`'s runtime methods (`push`/`pop`/`len`, language-spec
    /// §3.5) — provided by `runtime/array`, not backed by any `HirFnId`,
    /// the same reason `println`/`print` get [`HirExprKind::CallBuiltin`].
    CallArrayMethod {
        receiver: Box<HirExpr>,
        method: Symbol,
        args: Vec<HirExpr>,
    },
    /// An instance method call (`self`/`mut self`) whose receiver's owner
    /// is already a concrete `Struct`/`TupleStruct`/`Enum` `DefId` at
    /// lowering time — always resolved lazily by `monomorphization`
    /// (never a fixed [`HirFnId`] here, unlike [`HirExprKind::CallStatic`])
    /// because which body runs can depend on the receiver's *substituted*
    /// argument types, not just its owner: a concrete specialization
    /// (`impl Option<i32> { ... }`) only becomes knowable once a still-generic
    /// enclosing function is itself instantiated (`docs/generics.md` §
    /// "Methods on generic types"). Static methods keep using
    /// `CallStatic` with a fixed `fn_id` — specialization is scoped to
    /// instance methods only, so a static method's target never varies.
    CallMethod {
        receiver: Box<HirExpr>,
        method_name: Symbol,
        generic_args: Vec<Type>,
        args: Vec<HirExpr>,
    },
    If {
        cond: Box<HirExpr>,
        then_branch: Box<HirExpr>,
        else_branch: Option<Box<HirExpr>>,
    },
    Match {
        scrutinee: Box<HirExpr>,
        arms: Vec<HirMatchArm>,
    },
    Block(Vec<HirStmt>, Option<Box<HirExpr>>),
    While {
        cond: Box<HirExpr>,
        body: Box<HirExpr>,
    },
    /// `for pattern in iter { body }` is fully desugared away by this
    /// point into a `Block` around synthesized index/length temporaries
    /// and a `While` — see `lower.rs`'s `lower_for_in`. There is
    /// deliberately no `ForIn` variant left in HIR.
    Loop {
        body: Box<HirExpr>,
    },
    Break(Option<Box<HirExpr>>),
    Continue,
    Return(Option<Box<HirExpr>>),
    Closure {
        params: Vec<HirParam>,
        captures: Vec<HirCapture>,
        body: Box<HirExpr>,
    },
}

#[derive(Debug, Clone)]
pub struct HirStmt {
    pub kind: HirStmtKind,
}

#[derive(Debug, Clone)]
pub enum HirStmtKind {
    Let {
        local: HirLocalId,
        ty: Type,
        value: HirExpr,
    },
    Expr(HirExpr),
}

#[derive(Debug, Clone)]
pub struct HirMatchArm {
    pub pattern: HirPattern,
    pub body: HirExpr,
}

#[derive(Debug, Clone)]
pub enum HirPattern {
    Wildcard,
    Binding(HirLocalId),
    Literal(Literal),
    Tuple(Vec<HirPattern>),
    Variant {
        enum_id: DefId,
        variant: u32,
        payload: Vec<HirPattern>,
    },
}
