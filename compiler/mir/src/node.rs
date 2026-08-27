use nether_ast::{BinaryOp, Literal, Symbol, UnaryOp};
use nether_monomorphization::MonoFnId;
use nether_resolver::DefId;
use nether_typecheck::{AllocKind, CaptureMode, Type};

/// Identifies one local (parameter, `let`-binding, or MIR-internal
/// temporary) within a single [`MirFunction`] — a plain storage slot, not
/// SSA: a `Local` may be assigned to more than once (e.g. a `while` loop's
/// header block re-runs the same `Assign` each iteration), unlike
/// `nether_hir::HirLocalId`, which this space is minted fresh from rather
/// than reused, since building the CFG introduces temporaries with no HIR
/// counterpart at all (holding a sub-expression's value while control
/// flow is threaded through blocks, an `if`/`match`'s shared result slot,
/// a pattern test's intermediate boolean).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Local(pub(crate) u32);

impl Local {
    /// This local's position in its owning [`MirFunction`]'s `locals`
    /// vector — `nether_codegen`'s only handle for keying its own
    /// per-function `Local -> LLVM value` table (a plain `Vec`, since
    /// every `Local` this crate ever mints is exactly its position in
    /// that vector — see [`MirFunction::local_decl`]).
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Identifies one [`BasicBlock`] within a single [`MirFunction`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(pub(crate) u32);

/// One function's locals, laid out as an explicit control-flow graph —
/// the output of [`crate::build_mir`], and after [`crate::insert_arc`]
/// runs, with `Retain`/`Release` already inserted
/// (`docs/architecture/crates.md` § `compiler/mir`).
///
/// `Vec<MirFunction>` is indexed by the same [`MonoFnId`] every
/// `CallTarget::Fn` in this module refers to — `build_mir` builds one
/// `MirFunction` per `MonoFunction` in the same order, so no separate id
/// space is minted for this crate's own functions.
#[derive(Debug, Clone)]
pub struct MirFunction {
    /// Carried over unchanged from `nether_monomorphization::MonoFunction`
    /// — lets `nether_codegen` key a `MonoFnId -> LLVM function` table
    /// without relying on `Vec<MirFunction>`'s position matching it
    /// (mirrors `nether_hir::HirFunction`/`nether_monomorphization::
    /// MonoFunction`'s own `id` field, which this struct had originally
    /// omitted since `mir`'s own passes never needed to look a function
    /// up by id — only a later, cross-function-reference consumer like
    /// `codegen` does).
    pub id: MonoFnId,
    pub name: Symbol,
    pub owner: Option<DefId>,
    pub is_closure: bool,
    pub is_async: bool,
    /// `true` for an `extern "C" { ... }` block member — `blocks` is a
    /// trivial placeholder (`Terminator::Unreachable`, never actually
    /// reached); `nether_codegen` special-cases this flag to declare the
    /// symbol (real link name, C-ABI type mapping) rather than define a
    /// body for it.
    pub is_extern: bool,
    /// Locals initialized from fields in the hidden closure environment
    /// parameter, paired with how each one was captured. Empty for
    /// ordinary functions.
    pub closure_captures: Vec<(Local, CaptureMode)>,
    /// This function's arguments' locals, in calling-convention order —
    /// the receiver first if the function has a `self` parameter
    /// (matching `nether_hir`/`nether_monomorphization`'s own convention
    /// that a method call's `args[0]` is the receiver), then the
    /// declared parameters.
    pub params: Vec<Local>,
    pub ret: Type,
    pub locals: Vec<LocalDecl>,
    pub blocks: Vec<BasicBlock>,
    pub entry: BlockId,
}

impl MirFunction {
    pub fn block(&self, id: BlockId) -> &BasicBlock {
        &self.blocks[id.0 as usize]
    }

    pub fn local_decl(&self, local: Local) -> &LocalDecl {
        &self.locals[local.0 as usize]
    }
}

#[derive(Debug, Clone)]
pub struct LocalDecl {
    pub ty: Type,
    pub mutable: bool,
    /// Whether this local participates in ARC — read once, at
    /// declaration time, from `nether_typecheck::alloc_kind` (language-
    /// spec §3.3); [`crate::insert_arc`] only ever inserts `Retain`/
    /// `Release` for `AllocKind::Heap` locals (`arc-model.md` §1).
    pub alloc: AllocKind,
    /// True for both heap pointers and stack/value containers that own
    /// managed fields (including `weak` fields). Retain/release on the
    /// latter is implemented by generated deep-walk shims.
    pub needs_drop: bool,
}

#[derive(Debug, Clone)]
pub struct BasicBlock {
    pub id: BlockId,
    pub instrs: Vec<Instr>,
    pub terminator: Terminator,
}

/// A location an [`Instr::Assign`] can write to: a bare local or a local
/// followed by any validated field/index projection chain, such as
/// `self.inner.items[i].name`.
#[derive(Debug, Clone)]
pub struct Place {
    pub local: Local,
    pub projection: Vec<Projection>,
}

impl Place {
    pub fn local(local: Local) -> Place {
        Place {
            local,
            projection: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Projection {
    Deref,
    Field(u32),
    VariantField { variant: u32, index: u32 },
    Index(Operand),
}

#[derive(Debug, Clone)]
pub enum Operand {
    Local(Local),
    /// The type is carried alongside the literal (rather than left for a
    /// consumer to re-derive from context) specifically for `codegen`:
    /// unlike every other `Operand` variant, a bare literal has no
    /// `Local`/declaration anywhere to read a concrete
    /// `nether_typecheck::Type` back off of (a numeric literal's *width*
    /// — `i32` vs `u8` vs `i64`, say — is exactly the information
    /// `typecheck` already resolved and that would otherwise be lost
    /// here).
    Literal(Literal, Type),
    Unit,
    /// A direct reference to a known callable. Function values are
    /// normally converted to closure-ABI adapters before this stage.
    Fn(MonoFnId),
}

#[derive(Debug, Clone)]
pub enum CallTarget {
    Fn(MonoFnId),
    /// A call through an ARC-managed closure/function environment.
    Dynamic(Operand),
}

#[derive(Debug, Clone)]
pub enum Rvalue {
    Use(Operand),
    Await(Operand),
    /// Address of an assignable place; no ownership credit is created.
    AddressOf(Place),
    Deref(Operand),
    PromoteUnique(Operand),
    CloneToUnique(Operand),
    Hash(Operand),
    PackExistential {
        methods: Vec<Operand>,
    },
    Unary(UnaryOp, Operand),
    Binary(BinaryOp, Operand, Operand),
    Call {
        target: CallTarget,
        args: Vec<Operand>,
    },
    CallBuiltin {
        name: Symbol,
        args: Vec<Operand>,
    },
    CallArrayMethod {
        receiver: Operand,
        method: Symbol,
        args: Vec<Operand>,
    },
    CallWitness {
        receiver: Operand,
        slot: u32,
        function_ty: Type,
        args: Vec<Operand>,
    },
    Field {
        base: Operand,
        index: u32,
    },
    VariantField {
        base: Operand,
        variant: u32,
        index: u32,
    },
    /// Reads an enum value's variant tag as an integer — the basis for a
    /// `match` arm's discriminant test (`arc-model.md`'s scope note:
    /// enums are "tag + inline payload", language-spec §3.3).
    Discriminant(Operand),
    Index {
        base: Operand,
        index: Operand,
    },
    Construct {
        ty: DefId,
        fields: Vec<Operand>,
    },
    ConstructVariant {
        enum_id: DefId,
        variant: u32,
        payload: Vec<Operand>,
    },
    Tuple(Vec<Operand>),
    Array(Vec<Operand>),
    Concat(Vec<Operand>),
    ToString(Operand),
    /// Allocates a closure environment containing a code pointer followed
    /// by the captured values in declaration order.
    Closure {
        function: MonoFnId,
        captures: Vec<Operand>,
    },
}

#[derive(Debug, Clone)]
pub enum Instr {
    Assign(Place, Rvalue),
    /// Marks a moved-from unique heap local as empty. Later lexical drops
    /// remain valid because ARC release accepts null.
    Clear(Local),
    Retain(Local),
    /// A genuine, ownership-ending release: scope exit, an overwritten
    /// field/place's old value, or a purpose-built temporary's own single
    /// use. In every case, `local`'s reference is spent for good the
    /// instant this runs — nothing later in this function still expects
    /// to find a live value there. Distinct from [`Instr::TransientRelease`]
    /// for exactly this reason: `nether_codegen`'s async-frame codegen
    /// nulls a local's frame field after this instruction specifically
    /// (`FnCodegen::null_frame_field_after_release`), and would corrupt a
    /// still-alive local if it treated the two the same.
    Release(Local),
    /// The other half of `crate::arc::insert_arc`'s call-argument
    /// retain/release pair for a native (`CallBuiltin`/`CallArrayMethod`)
    /// callee (`arc-model.md` §3.3) — a purely transactional bump for the
    /// call's own duration that nets to zero and leaves `local`'s own
    /// ownership completely unaffected (`crate::arc`'s own module docs:
    /// "grants the native callee nothing lasting"). Unlike
    /// [`Instr::Release`], `local` may well still be read again later in
    /// this same function — `println(held); println(held);` retains and
    /// releases `held` around *each* call without ending its life either
    /// time.
    TransientRelease(Local),
    /// Bumps a `weak T` value's *weak* count — never its referent's
    /// strong count (`arc-model.md` §3.5) — inserted only where
    /// `nether_mir::build` constructs a fresh `weak T` from a `T`
    /// (a struct field initializer or assignment target declared `weak`).
    /// `local` is always freshly materialized with declared type
    /// `Weak(_)` for exactly this purpose, never a long-lived binding —
    /// standalone `let w: weak T = ...;` bindings are a documented gap
    /// (see `nether_mir::build`'s own module docs).
    WeakRetain(Local),
    /// The release counterpart of [`Instr::WeakRetain`] — decrements a
    /// `weak T` value's weak count, run when overwriting a `weak`-typed
    /// field's old value. A struct's own *remaining* `weak`-typed fields
    /// are released by its generated drop shim instead
    /// (`nether_codegen::shims`), not by this instruction, since that
    /// happens outside any single function's MIR entirely.
    WeakRelease(Local),
}

#[derive(Debug, Clone)]
pub enum Terminator {
    Goto(BlockId),
    Branch {
        cond: Operand,
        then_block: BlockId,
        else_block: BlockId,
    },
    /// Always carries an operand — `Operand::Unit` stands in for a
    /// unit-returning function's bare `return;`, so every `Return` edge
    /// is handled uniformly by [`crate::insert_arc`]'s retain rule (§3.4)
    /// rather than needing an `Option` special case.
    Return(Operand),
    /// A `match`'s final, never-taken fallback edge — `typecheck` already
    /// proved every arm chain here is exhaustive (language-spec:
    /// `check_match`'s exhaustiveness check), so this point is
    /// provably dead, the same concept as rustc's own MIR `Unreachable`
    /// terminator.
    Unreachable,
    /// A real suspension point inside an `is_async` function, introduced
    /// by [`crate::split_await_points`] in place of an ordinary
    /// `Instr::Assign(_, Rvalue::Await(task))` — only ever appears in an
    /// `is_async` function's blocks, and only after that pass has run.
    /// `codegen` polls `task` once; on `Ready`, it must store the output
    /// into `output_local` and continue at `resume` — on `Pending`, it
    /// must persist enough state to re-enter *this* block (not `resume`)
    /// on the next poll, since `output_local` isn't populated yet.
    Await {
        task: Operand,
        output_local: Local,
        resume: BlockId,
    },
}
