use nether_frontend::source::Span;
use nether_semantics::{Integer, IntegerType};

pub type FunctionId = usize;
pub type LocalId = usize;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    Unit,
    Bool,
    Char,
    Integer(IntegerType),
    F32,
    F64,
    Str,
    Tuple(Vec<Type>),
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
            Self::Record { copy, fields, .. } => *copy && fields.iter().all(|f| f.ty.copyable()),
            Self::Enum { copy, variants, .. } => {
                *copy && variants.iter().flat_map(|v| &v.fields).all(Type::copyable)
            }
            Self::Tuple(fields) => fields.iter().all(Type::copyable),
            Self::Array(element, _) => element.copyable(),
            _ => true,
        }
    }

    pub fn numeric(&self) -> bool {
        matches!(self, Self::Integer(_) | Self::F32 | Self::F64)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Unit,
    Bool(bool),
    Char(char),
    Integer(Integer),
    F32(f32),
    F64(f64),
    Str(String),
    Aggregate(Vec<Value>),
    Enum { variant: usize, fields: Vec<Value> },
}

#[derive(Clone, Debug)]
pub struct Program {
    pub functions: Vec<Function>,
    pub main: Option<FunctionId>,
}
#[derive(Clone, Debug)]
pub struct Function {
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
