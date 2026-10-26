//! Explicit control-flow representation. Every fallible operation has a panic edge.
//! Current lowering admits Copy values; ownership operations are added only with checking.
use crate::hir::{self as h, Type, Value};
use nether_frontend::source::Span;
pub mod borrow_check;
pub mod cleanup;
pub mod dataflow;
mod drop_borrows;
pub mod drop_plan;
pub mod drop_state;
pub mod effects;
pub mod initialization;
pub mod interpret;
pub mod loans;
mod lower;
mod reachability;
pub mod returns;
pub mod storage_writes;
pub mod verify;
pub use lower::lower;
pub type BlockId = usize;
pub type LocalId = usize;
#[derive(Clone, Debug)]
pub struct Program {
    pub classes: h::Classes,
    pub destructors: drop_plan::Destructors,
    pub functions: Vec<Function>,
    pub main: Option<usize>,
}
#[derive(Clone, Debug)]
pub struct Function {
    pub drop_reads: std::collections::BTreeSet<LocalId>,
    pub storage_writes: std::sync::Arc<Vec<storage_writes::Contract>>,
    pub result_view: Option<returns::Contract>,
    pub classes: std::sync::Arc<h::Classes>,
    pub cleanup: cleanup::Plan,
    pub name: String,
    pub parameters: Vec<LocalId>,
    pub locals: Vec<h::Local>,
    pub result: Type,
    pub blocks: Vec<Block>,
    pub entry: BlockId,
    pub span: Span,
}
#[derive(Clone, Debug)]
pub struct Block {
    pub terminator: Terminator,
    pub span: Span,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Place {
    pub local: LocalId,
    pub projections: Vec<Projection>,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Projection {
    Dereference,
    Field(usize),
    Variant(usize),
    Index(LocalId),
    ConstantIndex(u64),
}
#[derive(Clone, Debug)]
pub enum Operand {
    Address(Place),
    Constant(Value),
    Local(LocalId),
}
#[derive(Clone, Debug)]
pub enum Operation {
    StoreView(Place),
    LoadView(Place),
    CallStorage {
        function: usize,
        arguments: Vec<Operand>,
        contract: returns::StorageContract,
    },
    CallView {
        function: usize,
        arguments: Vec<Operand>,
        contract: returns::Contract,
    },
    Identity(Place, Place),
    AllocateClass(String),
    CompleteClass(Operand),
    Borrow(Place),
    CapturePlace(Place),
    Copy(Place),
    Move(Place),
    Use(Operand),
    Unary(&'static str, Operand),
    Binary(&'static str, Operand, Operand),
    Cast(Operand, Type),
    Aggregate(Vec<Operand>),
    Enum(usize, Vec<Operand>),
    Tag(Operand),
    TagPlace(Place),
    Call(usize, Vec<Operand>),
    CallIndirect(Operand, Vec<Operand>, h::CallContract),
}
#[derive(Clone, Debug)]
pub enum Terminator {
    EndStorage {
        locals: Vec<LocalId>,
        next: BlockId,
    },
    Evaluate {
        operation: Operation,
        destination: Place,
        next: BlockId,
        unwind: BlockId,
    },
    CheckPlace {
        place: Place,
        next: BlockId,
        unwind: BlockId,
    },
    Goto(BlockId),
    Branch {
        condition: Operand,
        yes: BlockId,
        no: BlockId,
    },
    Return(Operand),
    ResumePanic,
    Unreachable,
}
impl Place {
    pub fn local(local: LocalId) -> Self {
        Self {
            local,
            projections: Vec::new(),
        }
    }
}
impl Terminator {
    pub fn successors(&self) -> Vec<BlockId> {
        match self {
            Self::Evaluate { next, unwind, .. } | Self::CheckPlace { next, unwind, .. } => {
                vec![*next, *unwind]
            }
            Self::Goto(next) | Self::EndStorage { next, .. } => vec![*next],
            Self::Branch { yes, no, .. } => vec![*yes, *no],
            _ => vec![],
        }
    }
}
