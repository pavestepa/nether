use nether_ast::{BinaryOp, Literal, SelfParam, Symbol, UnaryOp};
use nether_hir::{HirLocalId, HirPattern};
use nether_resolver::DefId;
use nether_typecheck::Type;

/// Identifies one function in a [`MonoModule`] — a fresh id space,
/// separate from `nether_hir::HirFnId`. One generic `HirFunction` can
/// expand into many `MonoFunction`s (one per concrete instantiation), and
/// conversely a `Call`/`FnRef` to a non-generic `HirFunction` collapses to
/// exactly one `MonoFunction`, so `HirFnId`s and `MonoFnId`s are not in a
/// 1:1 relationship and reusing `HirFnId` here would be misleading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MonoFnId(pub(crate) u32);

impl MonoFnId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// The output of [`crate::monomorphize`]: every function/method actually
/// reachable from the entry point, with all generics eliminated — no
/// remaining `Type::Generic`, no remaining `HirExprKind::CallGenericMethod`
/// (`docs/architecture/crates.md` § `compiler/monomorphization`).
pub struct MonoModule {
    pub functions: Vec<MonoFunction>,
    pub entry: MonoFnId,
}

impl MonoModule {
    pub fn get(&self, id: MonoFnId) -> &MonoFunction {
        &self.functions[id.0 as usize]
    }
}

/// One fully-concrete function or method — a generic `HirFunction`'s
/// `generics` list is gone entirely here, substituted away rather than
/// merely instantiated-with-defaults, since every remaining `Type` in this
/// struct is guaranteed concrete (invariant of this crate).
pub struct MonoFunction {
    pub id: MonoFnId,
    pub name: Symbol,
    pub owner: Option<DefId>,
    pub is_closure: bool,
    pub self_param: Option<SelfParam>,
    pub self_ty: Option<Type>,
    /// Carried over unchanged from `nether_hir::HirFunction::self_local` —
    /// see that field's docs for why it exists.
    pub self_local: Option<HirLocalId>,
    /// Locals loaded from a closure environment before the lifted
    /// function body runs. Empty for ordinary functions and methods.
    pub captures: Vec<MonoCapture>,
    pub params: Vec<MonoParam>,
    pub ret: Type,
    pub body: MonoExpr,
}

#[derive(Debug, Clone)]
pub struct MonoCapture {
    pub local: HirLocalId,
    pub ty: Type,
}

#[derive(Debug, Clone)]
pub struct MonoParam {
    pub local: HirLocalId,
    pub name: Symbol,
    pub mutable: bool,
    pub ty: Type,
}

#[derive(Debug, Clone)]
pub struct MonoExpr {
    pub kind: MonoExprKind,
    pub ty: Type,
}

/// Mirrors `nether_hir::HirExprKind` node-for-node, with two differences:
/// `CallStatic` now names a [`MonoFnId`] instead of a `HirFnId`, and there
/// is no `CallGenericMethod` variant at all — every generic method call is
/// resolved to a concrete `CallStatic` during monomorphization (see this
/// crate's module docs).
#[derive(Debug, Clone)]
pub enum MonoExprKind {
    Literal(Literal),
    Local(HirLocalId),
    Borrow(Box<MonoExpr>),
    Deref(Box<MonoExpr>),
    PromoteUnique(Box<MonoExpr>),
    CloneToUnique(Box<MonoExpr>),
    Hash(Box<MonoExpr>),
    FnRef(MonoFnId),
    Unit,
    Tuple(Vec<MonoExpr>),
    Array(Vec<MonoExpr>),
    PackExistential {
        value: Box<MonoExpr>,
        adapters: Vec<(MonoFnId, Type)>,
    },
    Concat(Vec<MonoExpr>),
    ToString(Box<MonoExpr>),
    Unary {
        op: UnaryOp,
        expr: Box<MonoExpr>,
    },
    Binary {
        op: BinaryOp,
        lhs: Box<MonoExpr>,
        rhs: Box<MonoExpr>,
    },
    Assign {
        target: Box<MonoExpr>,
        value: Box<MonoExpr>,
    },
    Call {
        callee: Box<MonoExpr>,
        args: Vec<MonoExpr>,
    },
    CallStatic {
        fn_id: MonoFnId,
        args: Vec<MonoExpr>,
    },
    CallBuiltin {
        name: Symbol,
        args: Vec<MonoExpr>,
    },
    Field {
        base: Box<MonoExpr>,
        index: u32,
    },
    Index {
        base: Box<MonoExpr>,
        index: Box<MonoExpr>,
    },
    Construct {
        ty: DefId,
        fields: Vec<MonoExpr>,
    },
    ConstructVariant {
        enum_id: DefId,
        variant: u32,
        payload: Vec<MonoExpr>,
    },
    CallArrayMethod {
        receiver: Box<MonoExpr>,
        method: Symbol,
        args: Vec<MonoExpr>,
    },
    CallWitness {
        receiver: Box<MonoExpr>,
        slot: u32,
        function_ty: Type,
        args: Vec<MonoExpr>,
    },
    If {
        cond: Box<MonoExpr>,
        then_branch: Box<MonoExpr>,
        else_branch: Option<Box<MonoExpr>>,
    },
    Match {
        scrutinee: Box<MonoExpr>,
        arms: Vec<MonoMatchArm>,
    },
    Block(Vec<MonoStmt>, Option<Box<MonoExpr>>),
    While {
        cond: Box<MonoExpr>,
        body: Box<MonoExpr>,
    },
    Loop {
        body: Box<MonoExpr>,
    },
    Break(Option<Box<MonoExpr>>),
    Continue,
    Return(Option<Box<MonoExpr>>),
    /// A closure after conversion: the body has been lifted into
    /// `function`; evaluating this expression constructs an environment
    /// from the listed capture values.
    Closure {
        function: MonoFnId,
        captures: Vec<MonoExpr>,
    },
}

#[derive(Debug, Clone)]
pub struct MonoStmt {
    pub kind: MonoStmtKind,
}

#[derive(Debug, Clone)]
pub enum MonoStmtKind {
    Let {
        local: HirLocalId,
        ty: Type,
        value: MonoExpr,
    },
    Expr(MonoExpr),
}

#[derive(Debug, Clone)]
pub struct MonoMatchArm {
    /// Patterns never mention a type parameter or a callable, so
    /// `nether_hir::HirPattern` is reused as-is rather than mirrored.
    pub pattern: HirPattern,
    pub body: MonoExpr,
}
