use crate::source::Span;

#[derive(Clone, Debug, PartialEq)]
pub struct Module {
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub kind: ItemKind,
    pub private: bool,
    pub attributes: Vec<Attribute>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Attribute {
    Copy,
    ReprC,
    Link(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ItemKind {
    Function(Function),
    Aggregate {
        class: bool,
        name: String,
        generics: Vec<Generic>,
        interfaces: Vec<Type>,
        members: Vec<Member>,
    },
    Enum {
        name: String,
        generics: Vec<Generic>,
        variants: Vec<Variant>,
    },
    Interface {
        name: String,
        generics: Vec<Generic>,
        methods: Vec<Function>,
    },
    Const {
        name: String,
        ty: Type,
        value: Expr,
    },
    Import {
        export: bool,
        names: Vec<String>,
        from: String,
    },
    Extern(Vec<Function>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Generic {
    pub name: String,
    pub kind: GenericKind,
    pub span: Span,
}
#[derive(Clone, Debug, PartialEq)]
pub enum GenericKind {
    Type {
        constraints: Vec<Type>,
        default: Option<Type>,
    },
    Const {
        ty: Type,
        default: Option<Expr>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    pub name: String,
    pub generics: Vec<Generic>,
    pub parameters: Vec<Parameter>,
    pub result: Type,
    pub body: Option<Block>,
    pub c_abi: bool,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Parameter {
    pub pattern: Pattern,
    pub mutable: bool,
    pub ty: Option<Type>,
    pub receiver: bool,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Member {
    Field {
        name: String,
        ty: Type,
        weak: bool,
        private: bool,
        span: Span,
    },
    Method {
        function: Function,
        private: bool,
    },
    Constructor {
        parameters: Vec<Parameter>,
        body: Block,
        span: Span,
    },
    Destructor(Block),
    Const {
        name: String,
        ty: Type,
        value: Expr,
        span: Span,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Variant {
    pub name: String,
    pub fields: VariantFields,
    pub span: Span,
}
#[derive(Clone, Debug, PartialEq)]
pub enum VariantFields {
    Unit,
    Tuple(Vec<Type>),
    Named(Vec<(String, Type)>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Type {
    pub kind: TypeKind,
    pub span: Span,
}
#[derive(Clone, Debug, PartialEq)]
pub enum TypeKind {
    Named {
        path: Vec<String>,
        arguments: Vec<TypeArgument>,
    },
    Tuple(Vec<Type>),
    Array {
        element: Box<Type>,
        length: Box<Expr>,
    },
    Pointer {
        mutable: bool,
        pointee: Box<Type>,
    },
    Function {
        parameters: Vec<(bool, Type)>,
        result: Box<Type>,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub enum TypeArgument {
    Type(Type),
    Const(Expr),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub statements: Vec<Statement>,
    pub span: Span,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Statement {
    pub kind: StatementKind,
    pub span: Span,
}
#[derive(Clone, Debug, PartialEq)]
pub enum StatementKind {
    Binding {
        mutable: bool,
        pattern: Pattern,
        ty: Option<Type>,
        value: Expr,
    },
    Expression(Expr),
    Return(Option<Expr>),
    Break,
    Continue,
    Unsafe(Block),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Literal {
    Integer(String),
    Float(String),
    String(String),
    Char(char),
    Bool(bool),
}
#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    Name(String),
    Specialize {
        value: Box<Expr>,
        arguments: Vec<TypeArgument>,
    },
    Literal(Literal),
    Group(Box<Expr>),
    Tuple(Vec<Expr>),
    Array(Vec<Expr>),
    Construct {
        ty: Type,
        fields: Vec<(String, Expr)>,
    },
    New {
        ty: Type,
        arguments: Vec<Expr>,
    },
    Call {
        callee: Box<Expr>,
        types: Vec<TypeArgument>,
        arguments: Vec<Expr>,
    },
    Member {
        value: Box<Expr>,
        name: String,
    },
    Index {
        value: Box<Expr>,
        index: Box<Expr>,
    },
    Unary {
        operator: &'static str,
        value: Box<Expr>,
    },
    Binary {
        operator: &'static str,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Assign {
        operator: &'static str,
        place: Box<Expr>,
        value: Box<Expr>,
    },
    Cast {
        value: Box<Expr>,
        ty: Type,
    },
    If {
        condition: Box<Expr>,
        then_block: Block,
        else_branch: Option<Box<Expr>>,
    },
    While {
        condition: Box<Expr>,
        body: Block,
    },
    Branch(Block),
    Match {
        value: Box<Expr>,
        arms: Vec<Arm>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Arm {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub value: Expr,
    pub span: Span,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Pattern {
    pub kind: PatternKind,
    pub span: Span,
}
#[derive(Clone, Debug, PartialEq)]
pub enum PatternKind {
    Wildcard,
    Binding {
        name: String,
        by_ref: bool,
        mutable: bool,
    },
    Literal {
        negative: bool,
        value: Literal,
    },
    Tuple(Vec<Pattern>),
    Array(Vec<Pattern>),
    Rest,
    Variant {
        path: Vec<String>,
        fields: PatternFields,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub enum PatternFields {
    Unit,
    Tuple(Vec<Pattern>),
    Named(Vec<(String, Pattern)>, bool),
}
