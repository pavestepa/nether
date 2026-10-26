use nether_frontend::source::Span;
use nether_semantics::{Integer, IntegerType};

pub type FunctionId = usize;
pub type LocalId = usize;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    /// Static readonly descriptor stored as an aggregate component.
    StoredView(Box<Type>),
    /// Non-owning address. Validity and aliasing are obligations of unsafe code.
    Pointer {
        mutable: bool,
        pointee: Box<Type>,
    },
    Unit,
    Bool,
    Char,
    Integer(IntegerType),
    F32,
    F64,
    Str,
    Function {
        unsafe_: bool,
        parameters: Vec<(bool, Type)>,
        result: Box<Type>,
        contract: Option<Box<FunctionContract>>,
    },
    Tuple(Vec<Type>),
    /// Identity-bearing handle. Payload definitions live in a nominal registry,
    /// so recursive classes do not recursively expand the Type value.
    Class {
        name: String,
        copy: bool,
        storage: Option<Vec<RecordField>>,
    },
    Record {
        copy: bool,
        name: String,
        fields: Vec<RecordField>,
    },
    Array(Box<Type>, u64),
    Enum {
        copy: bool,
        name: String,
        variants: Vec<Variant>,
    },
}
/// Compile-time ABI and provenance of a plain code pointer. No environment is stored.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct FunctionContract {
    pub sources: Vec<String>,
    pub modes: Vec<Option<crate::mir::loans::Access>>,
    pub view: Option<crate::mir::returns::Contract>,
    pub storage: crate::mir::returns::StorageContract,
    pub writes: crate::mir::storage_writes::Contract,
}
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CallContract {
    pub targets: Vec<FunctionId>,
    pub view: Option<crate::mir::returns::Contract>,
    pub storage: crate::mir::returns::StorageContract,
    pub writes: crate::mir::storage_writes::Contract,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variant {
    pub name: String,
    pub fields: Vec<Type>,
    pub shape: VariantShape,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VariantShape {
    Unit,
    Tuple,
    Named(Vec<String>),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordField {
    pub name: String,
    pub ty: Type,
    pub private: bool,
    pub source: nether_frontend::source::SourceId,
}
impl Type {
    pub fn copyable(&self) -> bool {
        match self {
            Self::Class { copy, .. } => *copy,
            Self::Record { copy, fields, .. } => *copy && fields.iter().all(|f| f.ty.copyable()),
            Self::Enum { copy, variants, .. } => {
                *copy && variants.iter().flat_map(|v| &v.fields).all(Type::copyable)
            }
            Self::Tuple(fields) => fields.iter().all(Type::copyable),
            Self::Array(element, _) => element.copyable(),
            _ => true,
        }
    }

    /// Copy classes still own allocations. MIR value operands transfer those
    /// temporary owners; only an explicit Copy operation duplicates identity.
    pub fn owns_storage(&self) -> bool {
        !self.copyable()
            || match self {
                Self::Class { .. } => true,
                Self::Record { fields, .. } => fields.iter().any(|f| f.ty.owns_storage()),
                Self::Enum { variants, .. } => variants
                    .iter()
                    .flat_map(|v| &v.fields)
                    .any(Self::owns_storage),
                Self::Tuple(fields) => fields.iter().any(Self::owns_storage),
                Self::Array(element, count) => *count != 0 && element.owns_storage(),
                _ => false,
            }
    }

    pub fn contains_views(&self) -> bool {
        match self {
            Self::StoredView(_) => true,
            Self::Class {
                storage: Some(fields),
                ..
            } => fields.iter().any(|f| f.ty.contains_views()),
            Self::Record { fields, .. } => fields.iter().any(|f| f.ty.contains_views()),
            Self::Tuple(fields) => fields.iter().any(Self::contains_views),
            Self::Array(element, _) => element.contains_views(),
            Self::Enum { variants, .. } => variants
                .iter()
                .flat_map(|v| &v.fields)
                .any(Self::contains_views),
            _ => false,
        }
    }
    /// Source annotations name the logical type; physical storage modes remain
    /// part of the concrete HIR type and cannot change on assignment/call.
    pub fn logical(&self) -> Self {
        let mut ty = self.clone();
        match &mut ty {
            Self::StoredView(target) => return target.logical(),
            Self::Class { storage, .. } => *storage = None,
            Self::Function {
                parameters,
                result,
                contract,
                ..
            } => {
                *contract = None;
                for (_, ty) in parameters {
                    *ty = ty.logical()
                }
                **result = result.logical();
            }
            Self::Record { fields, .. } => {
                for f in fields {
                    f.ty = f.ty.logical();
                }
            }
            Self::Tuple(fields) => {
                for t in fields {
                    *t = t.logical();
                }
            }
            Self::Array(t, _) => **t = t.logical(),
            Self::Enum { variants, .. } => {
                for v in variants {
                    for t in &mut v.fields {
                        *t = t.logical();
                    }
                }
            }
            _ => (),
        }
        ty
    }
    pub fn class_key(&self) -> String {
        let Self::Class { name, storage, .. } = self else {
            panic!("class key requires class")
        };
        if storage.is_some() {
            format!("{name}${storage:?}")
        } else {
            name.clone()
        }
    }
    pub fn raw_scalar(&self) -> bool {
        matches!(
            self,
            Self::Bool | Self::Char | Self::Integer(_) | Self::F32 | Self::F64
        )
    }
    pub fn numeric(&self) -> bool {
        matches!(self, Self::Integer(_) | Self::F32 | Self::F64)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassDefinition {
    pub fields: Vec<RecordField>,
    pub copy: bool,
}
pub type Classes = std::collections::BTreeMap<String, ClassDefinition>;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    View(crate::reference_memory::ViewValue),
    Pointer(crate::reference_memory::RawPointer),
    Class(crate::reference_memory::ClassValue),
    Unit,
    Bool(bool),
    Char(char),
    Integer(Integer),
    F32(f32),
    F64(f64),
    Str(String),
    Function(FunctionId),
    Aggregate(Vec<Value>),
    Enum { variant: usize, fields: Vec<Value> },
}

#[derive(Clone, Debug)]
pub struct Program {
    pub classes: Classes,
    pub functions: Vec<Function>,
    pub main: Option<FunctionId>,
}
#[derive(Clone, Debug)]
pub struct Function {
    pub storage_writes: crate::mir::storage_writes::Contract,
    pub result_view: Option<crate::mir::returns::Contract>,
    pub name: String,
    pub parameters: Vec<LocalId>,
    pub locals: Vec<Local>,
    pub result: Type,
    pub body: Block,
    pub span: Span,
}
#[derive(Clone, Debug)]
pub struct Local {
    pub view: Option<crate::mir::loans::Access>,
    pub name: String,
    pub ty: Type,
    pub mutable: bool,
    pub span: Span,
}
#[derive(Clone, Debug)]
pub struct Block {
    /// Staged named fields, in declaration order, owned by this construction.
    pub construction: Vec<LocalId>,
    pub storage: Vec<LocalId>,
    pub statements: Vec<Statement>,
    pub result: Type,
    pub diverges: bool,
    pub span: Span,
}
#[derive(Clone, Debug)]
pub enum Statement {
    Initialize(LocalId, Expr),
    Evaluate(Expr),
    Return(Option<Expr>),
    Break,
    Continue,
}
#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub ty: Type,
    pub span: Span,
}
#[derive(Clone, Debug)]
pub enum ExprKind {
    StoreView(Box<Expr>),
    StoredView(Box<Expr>),
    StorageCall {
        function: FunctionId,
        arguments: Vec<Expr>,
        contract: crate::mir::returns::StorageContract,
    },
    BorrowedCall {
        function: FunctionId,
        arguments: Vec<Expr>,
        contract: crate::mir::returns::Contract,
    },
    Identity(Box<Expr>, Box<Expr>),
    /// Transfer an already evaluated internal owner; never a language Copy.
    Take(Box<Expr>),
    New {
        constructor: FunctionId,
        arguments: Vec<Expr>,
    },
    ArgumentView {
        place: Box<Expr>,
        access: crate::mir::loans::Access,
    },
    View(LocalId),
    Borrow {
        local: LocalId,
        place: Box<Expr>,
    },
    Block(Block),
    Enum {
        variant: usize,
        fields: Vec<Expr>,
    },
    Tag(Box<Expr>),
    Payload(Box<Expr>, usize),
    Match(Vec<MatchArm>),
    Value(Value),
    Local(LocalId),
    Tuple(Vec<Expr>),
    Array(Vec<Expr>),
    Unary(&'static str, Box<Expr>),
    Binary(&'static str, Box<Expr>, Box<Expr>),
    Cast(Box<Expr>),
    Call(FunctionId, Vec<Expr>),
    CallIndirect(Box<Expr>, Vec<Expr>, CallContract),
    Assign {
        place: Box<Expr>,
        operator: &'static str,
        value: Box<Expr>,
    },
    Index(Box<Expr>, Box<Expr>),
    Field(Box<Expr>, usize),
    If {
        condition: Box<Expr>,
        yes: Block,
        no: Option<Block>,
    },
    While {
        condition: Box<Expr>,
        body: Block,
    },
}

#[derive(Clone, Debug)]
pub struct MatchArm {
    pub guard_bindings: Vec<Statement>,
    pub condition: Expr,
    pub bindings: Vec<Statement>,
    pub guard: Option<Expr>,
    pub body: Block,
}
