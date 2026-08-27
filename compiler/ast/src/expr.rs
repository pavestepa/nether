use nether_diagnostics::Span;

use crate::ident::{Ident, Path};
use crate::ids::NodeId;
use crate::item::Param;
use crate::literal::Literal;
use crate::pattern::Pattern;
use crate::ty::TypeExpr;

/// One expression node. Carries its own [`NodeId`] (separate from its
/// [`ExprKind`]) so `typecheck` can key `expr_types: HashMap<NodeId, Type>`
/// uniformly across every expression shape — see
/// `docs/architecture/crates.md` (`typecheck`).
#[derive(Debug, Clone)]
pub struct Expr {
    pub id: NodeId,
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    Literal(Literal),
    Path(Path),
    Tuple(Vec<Expr>),
    /// `[1, 2, 3]` — always a growable `Array<T>` (language-spec §2.4,
    /// Stage 7). Distinct from [`ExprKind::FixedArray`]'s `{1, 2, 3}` —
    /// before Stage 7 this single node also stood in for a fixed-size
    /// `{T, N}` literal whenever the surrounding expected type called
    /// for one; that context-driven coercion is gone; the bracket kind
    /// alone now decides the representation.
    Array(Vec<Expr>),
    /// `{1, 2, 3}` — always an inline `{T, N}` fixed-size array
    /// (language-spec §2.4, Stage 7). Unambiguous at parse time with no
    /// preceding type name: a bare `{ ... }` is never a block expression
    /// (§2.6) and never a struct literal (that always has a `Path`
    /// immediately before the `{`), so encountering `{` in expression-atom
    /// position can only mean this.
    FixedArray(Vec<Expr>),
    /// Prefix `await expr`. Type checking restricts it to async bodies.
    Await(Box<Expr>),
    /// `unsafe { ... }` (language-spec §17, Stage 5) — a pure compile-time
    /// permission gate enabling raw-pointer dereference and calls to
    /// `unsafe fn`/`extern "C" fn` inside it. Has no runtime effect of its
    /// own; `hir` lowering erases this wrapper entirely once typecheck has
    /// used it to gate the checks inside.
    Unsafe(Block),
    /// `&raw const expr` / `&raw mut expr` (language-spec §17, Stage 5) —
    /// takes the address of an arbitrary *place* (a local, field, index,
    /// or deref chain — `typecheck` validates the shape) as a raw
    /// pointer, bypassing the unique-ownership borrow-tracking system
    /// entirely. Forming a raw pointer this way is always safe; only
    /// dereferencing one (`RawDeref`) requires `unsafe`.
    RawBorrow {
        mutable: bool,
        place: Box<Expr>,
    },
    /// Prefix `*expr` (language-spec §17, Stage 5) — dereferences a raw
    /// pointer (`*const T`/`*mut T`), read or (as an assignment target)
    /// write. Always requires `unsafe` context — unlike `Ref`/`MutRef`
    /// dereference, which is auto-inserted by the compiler and never
    /// written by hand, this is the one explicit surface dereference
    /// operator in the language.
    RawDeref(Box<Expr>),
    /// A backtick template string; see [`Literal::Str`] for the
    /// non-interpolating plain-string counterpart (language-spec §2.3).
    StringTemplate(Vec<TemplatePart>),
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Assign {
        target: Box<Expr>,
        value: Box<Expr>,
    },
    Call {
        callee: Box<Expr>,
        /// Explicit call-site type arguments (`foo<u32>(2)`). Empty when
        /// all generic arguments are inferred.
        generic_args: Vec<TypeExpr>,
        args: Vec<Expr>,
    },
    /// `mut x` written at a call-argument position — marks that the
    /// callee's matching `mut name: Type` parameter should alias `x`
    /// directly instead of cloning it (language-spec §5.1). Only
    /// meaningful in argument position; `typecheck` rejects it elsewhere.
    MutArg(Box<Expr>),
    MethodCall {
        receiver: Box<Expr>,
        method: Ident,
        /// Explicit method type arguments (`value.map<String>(...)`).
        generic_args: Vec<TypeExpr>,
        args: Vec<Expr>,
    },
    Field {
        base: Box<Expr>,
        field: FieldAccessor,
    },
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
    },
    If {
        cond: Box<Expr>,
        then_branch: Block,
        else_branch: Option<Box<Expr>>,
    },
    Match {
        scrutinee: Box<Expr>,
        arms: Vec<MatchArm>,
    },
    Block(Block),
    While {
        cond: Box<Expr>,
        body: Block,
    },
    ForIn {
        pattern: Pattern,
        iter: Box<Expr>,
        body: Block,
    },
    Loop {
        body: Block,
    },
    Break(Option<Box<Expr>>),
    Continue,
    Return(Option<Box<Expr>>),
    /// `(a: i32, b: i32) => { a + b }` — language-spec §11. Nether has no
    /// nested named functions, only closures.
    Closure {
        /// `move (...) => ...` explicitly transfers captured unique values
        /// into the closure environment at closure creation.
        move_capture: bool,
        /// `mut (...) => ...` (Stage 7) captures every `mut`-declared
        /// outer local the body touches by mutable reference instead of
        /// by value — mutating it inside the body writes back to the
        /// outer binding. Mutually exclusive with `move_capture`: the
        /// parser only ever recognizes one of `move`/`mut` before a
        /// closure's parameter list.
        mut_capture: bool,
        params: Vec<Param>,
        body: Box<Expr>,
    },
    /// `Dog { name }` — also used for tuple-struct/enum-variant
    /// construction written in call syntax (`Point(x, y)`), which the
    /// parser instead represents as an ordinary [`ExprKind::Call`] whose
    /// callee is a [`ExprKind::Path`]; `StructLit` is reserved for the
    /// brace-field form only. Fields use `=` (language-spec §11), not `:`.
    /// `owned` is `true` for `:Dog { ... }`, the unique-ownership-domain
    /// form — the leading `:` sits directly on the literal itself, one of
    /// the few expression-position places the ownership sigil appears
    /// (language-spec §3).
    StructLit {
        path: Path,
        fields: Vec<(Ident, Expr)>,
        owned: bool,
    },
}

#[derive(Debug, Clone)]
pub enum FieldAccessor {
    Named(Ident),
    /// `.0`, `.1` tuple/tuple-struct field access (language-spec §3.6).
    Index(u32, Span),
}

#[derive(Debug, Clone)]
pub enum TemplatePart {
    Literal(String),
    Expr(Expr),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

#[derive(Debug, Clone)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    /// The final expression with no trailing `;`, if any — its value is
    /// the block's value (language-spec §2.4).
    pub tail: Option<Box<Expr>>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Let(LetStmt),
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub struct LetStmt {
    /// Identifies this binding site so `resolver` can key "which local did
    /// this `let` introduce" — later reference sites resolve to that same
    /// local via their own [`Expr`]/[`Path`] `id`, not this one.
    pub id: NodeId,
    pub mutable: bool,
    pub name: Ident,
    pub ty: Option<TypeExpr>,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub body: Expr,
    pub span: Span,
}
