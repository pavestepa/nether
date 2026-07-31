use std::collections::HashMap;

use nether_diagnostics::{FileId, Span};

use crate::expr::Block;
use crate::ident::{Ident, Path};
use crate::ids::NodeId;
use crate::ty::TypeExpr;

/// One parsed source file: language-spec §10 treats each file as a module.
/// The driver follows `mod` declarations and maps `use` imports before
/// resolver operates on the combined module graph.
#[derive(Debug, Clone)]
pub struct Module {
    pub file: FileId,
    pub items: Vec<Item>,
    /// Filled by the multi-file driver: each `use` declaration's node id
    /// maps to the file/module that path loaded. A parser operating on one
    /// standalone source leaves this empty.
    pub imports: HashMap<NodeId, FileId>,
}

#[derive(Debug, Clone)]
pub enum Item {
    Type(TypeDecl),
    Impl(ImplBlock),
    Enum(EnumDecl),
    Interface(InterfaceDecl),
    Fn(FnDecl),
    Use(UseDecl),
    Mod(ModDecl),
}

/// `type Dog { name: String }` / `type Point(i32, i32);` / `type Unit;`
/// (language-spec §3.2). Whether this allocates on the heap or the stack is
/// a semantic fact derived later from `name`'s casing
/// (`docs/architecture/type-system.md` §2-3) — this node just records what
/// was written.
#[derive(Debug, Clone)]
pub struct TypeDecl {
    pub id: NodeId,
    pub name: Ident,
    /// `Box<T>` — language-spec §8 lists "generic types" as supported;
    /// empty for a non-generic declaration.
    pub generics: Vec<GenericParam>,
    /// Interfaces opted into on the declaration (`type Dog: Sound, Clone`).
    /// Missing default methods are inherited only through this list.
    pub interfaces: Vec<TypeExpr>,
    pub kind: TypeDeclKind,
    /// Joined text of any leading `///` doc comments (language-spec §2.1).
    pub doc: Option<String>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum TypeDeclKind {
    Struct(Vec<Field>),
    TupleStruct(Vec<TypeExpr>),
    Unit,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name: Ident,
    pub ty: TypeExpr,
    /// True if written with a leading `_` or the `private` keyword
    /// (language-spec §4 — the two spellings are equivalent).
    pub private: bool,
}

/// `impl Dog { ... }` or `impl Dog: Sound { ... }` (language-spec §6-7).
#[derive(Debug, Clone)]
pub struct ImplBlock {
    pub id: NodeId,
    pub target: Ident,
    /// The interfaces in `impl Dog: Sound, Clone` — full type expressions
    /// (not bare
    /// [`Path`]) because an interface name may itself be generic
    /// (language-spec §7.1).
    pub interfaces: Vec<TypeExpr>,
    pub methods: Vec<FnDecl>,
    pub span: Span,
}

/// `enum Color { Red, Custom(String) }` (language-spec §9). Always a
/// stack/value type regardless of `name`'s casing — the one exemption from
/// the general heap/stack naming rule (language-spec §3.3).
#[derive(Debug, Clone)]
pub struct EnumDecl {
    pub id: NodeId,
    pub name: Ident,
    pub generics: Vec<GenericParam>,
    /// Interfaces opted into on the declaration (`enum State: Display`).
    pub interfaces: Vec<TypeExpr>,
    pub variants: Vec<EnumVariant>,
    pub doc: Option<String>,
    pub span: Span,
}

/// A generic parameter as written, e.g. the `T` in `<T>` or the `T: Sound`
/// / `T: Into<String>` in `<T: Sound>` / `<T: Into<String>>`. `bound` names
/// an [`interface`](InterfaceDecl), possibly itself generic — a full
/// [`TypeExpr`] rather than a bare [`Path`], for the same reason as
/// [`ImplBlock::interface`]. Language-spec §8 generics are resolved via
/// monomorphization, so this is purely syntactic — `resolver`/`typecheck`
/// turn `bound` into an actual constraint check.
#[derive(Debug, Clone)]
pub struct GenericParam {
    pub name: Ident,
    pub bound: Option<TypeExpr>,
}

#[derive(Debug, Clone)]
pub struct EnumVariant {
    pub name: Ident,
    /// Empty for a unit variant (`Red`); one or more for a payload variant
    /// (`Custom(String)`) written in tuple-call form (language-spec §9).
    pub payload: Vec<TypeExpr>,
    pub span: Span,
}

/// `interface Sound { sound(): String { "..." } }` (language-spec §7).
/// Static dispatch only — an `InterfaceDecl` never gains a runtime
/// representation; it is erased by monomorphization (see
/// `docs/architecture/type-system.md` §4).
#[derive(Debug, Clone)]
pub struct InterfaceDecl {
    pub id: NodeId,
    pub name: Ident,
    /// language-spec §8 lists "generic interfaces" as supported; empty for
    /// a non-generic declaration.
    pub generics: Vec<GenericParam>,
    /// Direct parent interfaces (`interface Child: ParentA, ParentB`).
    pub parents: Vec<TypeExpr>,
    /// A method with `body: None` has no default implementation and must
    /// be provided by every `impl`; a method with `body: Some(_)` is a
    /// default, overridable per-impl.
    pub methods: Vec<FnDecl>,
    pub doc: Option<String>,
    pub span: Span,
}

/// A standalone `fn` or an `impl`/`interface` method. There is no `fn`
/// keyword inside an `impl` block (language-spec §6) — the parser only
/// requires the keyword for top-level functions and sets it implicitly for
/// methods; both shapes reuse this one node.
#[derive(Debug, Clone)]
pub struct FnDecl {
    pub id: NodeId,
    pub name: Ident,
    pub generics: Vec<GenericParam>,
    /// `None` for a static method / standalone function; `Some(_)` for an
    /// instance method (language-spec §6).
    pub self_param: Option<SelfParam>,
    pub params: Vec<Param>,
    pub ret: Option<TypeExpr>,
    /// `None` only for an interface method with no default body
    /// (language-spec §7).
    pub body: Option<Block>,
    pub private: bool,
    pub doc: Option<String>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfParam {
    /// `self` — immutable reference (language-spec §6).
    ByRef,
    /// `mut self` — mutable reference. There is no owning `self` in
    /// Nether (language-spec §6).
    ByMutRef,
}

/// A function/method parameter. `mutable` marks the explicit
/// mutable-reference form (`mut name: Type`, language-spec §5.1) — distinct
/// from an ordinary by-value (stack: clone) or by-shared-reference (heap:
/// ARC) parameter.
#[derive(Debug, Clone)]
pub struct Param {
    /// Identifies this binding site for `resolver`, the same way
    /// [`crate::expr::LetStmt`]'s `id` does.
    pub id: NodeId,
    pub name: Ident,
    pub mutable: bool,
    pub ty: TypeExpr,
    pub span: Span,
}

/// `use user.User;` (language-spec §10).
#[derive(Debug, Clone)]
pub struct UseDecl {
    pub id: NodeId,
    pub path: Path,
    pub span: Span,
}

/// `mod child;` declares and loads a child file module. The driver maps
/// it to `child.nt`/`child.nr` or `child/mod.nt`/`child/mod.nr` relative
/// to the declaring module.
#[derive(Debug, Clone)]
pub struct ModDecl {
    pub id: NodeId,
    pub name: Ident,
    pub span: Span,
}
