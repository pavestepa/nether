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
    /// Like `imports`, but for a `use module.Enum.Variant;` path (3+
    /// segments) where the trailing *two* segments name an enum and one
    /// of its variants within the target file, rather than the trailing
    /// *one* segment `imports` maps to a top-level name. Lets
    /// `resolver::collect` promote a bare `Variant` name (e.g. `Some`) as
    /// an expression-position value, mirroring how `imports`/`prelude`
    /// promote an ordinary top-level name.
    pub variant_imports: HashMap<NodeId, FileId>,
}

#[derive(Debug, Clone)]
pub enum Item {
    Struct(StructDecl),
    TypeAlias(TypeAliasDecl),
    Impl(ImplBlock),
    Enum(EnumDecl),
    Trait(TraitDecl),
    Fn(FnDecl),
    Use(UseDecl),
    Mod(ModDecl),
}

/// Declaration visibility. Nether is default-private: crossing a module
/// boundary is permitted only for declarations explicitly marked `pub`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Visibility {
    #[default]
    Private,
    Public,
}

impl Visibility {
    pub fn is_public(self) -> bool {
        matches!(self, Self::Public)
    }
}

/// `struct Dog { name String }` / `struct Point(i32, i32);` /
/// `struct Unit;` (language-spec §4.2). Always a heap/reference-category
/// type — `struct` is reserved exclusively for this, distinct from the
/// alias-only `type` keyword ([`TypeAliasDecl`]). Whether a *use* of this
/// type is ARC (`T`) or uniquely owned (`:T`) is chosen at each use site
/// (language-spec §3), not recorded here.
#[derive(Debug, Clone)]
pub struct StructDecl {
    pub id: NodeId,
    pub name: Ident,
    pub visibility: Visibility,
    /// `Box<T>` — language-spec §13 lists "generic types" as supported;
    /// empty for a non-generic declaration.
    pub generics: Vec<GenericParam>,
    /// Traits opted into on the declaration (`struct Dog: Sound, Clone`).
    /// Missing default methods are inherited only through this list.
    pub traits: Vec<TypeExpr>,
    pub kind: StructDeclKind,
    /// Joined text of any leading `///` doc comments (language-spec §2.2).
    pub doc: Option<String>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum StructDeclKind {
    Struct(Vec<Field>),
    TupleStruct(Vec<TypeExpr>),
    Unit,
}

/// `type color = (u32, u32, u32);` (language-spec §4.3) — a type alias.
/// Distinct from [`StructDecl`]: `type` never declares a new
/// heap/reference-category type, only names an existing [`TypeExpr`].
/// Naming-convention validation (language-spec §5) is checked against
/// `ty`'s *resolved* representation category, not against this node's
/// syntax.
///
/// The ownership-qualified form (`type: Name = :TypeExpr;`) is specified
/// for the full language but not yet implemented — the parser recognizes
/// and rejects it with a dedicated diagnostic rather than silently
/// mis-parsing it (language-spec §4.3).
#[derive(Debug, Clone)]
pub struct TypeAliasDecl {
    pub id: NodeId,
    pub name: Ident,
    pub visibility: Visibility,
    /// Narrow Stage 3 escape hatch for aliases whose public spelling
    /// intentionally does not reflect their representation category.
    pub allow_pascal_case: bool,
    pub ty: TypeExpr,
    pub doc: Option<String>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name: Ident,
    pub ty: TypeExpr,
    pub visibility: Visibility,
}

/// `impl Dog { ... }` or `impl Dog: Sound { ... }` (language-spec §6-7).
///
/// A target's own generic parameters are normally implicit — declared once
/// on the `type`/`enum` and automatically in scope in every `impl` block
/// (no `<T>` repeated after `impl`). `generics`/`target_args` instead hold
/// the Rust-like explicit form, `impl<T> Boxed<T> { ... }`, which is the
/// only way to bind a type parameter for an owner with no local
/// declaration to point at (the compiler-builtin `Option`/`Result`). Both
/// are empty for the implicit form. When present, `target_args` must be
/// exactly a permutation of `generics`' names — positional renaming only,
/// not specialization (`impl Option<i32>` stays unsupported).
#[derive(Debug, Clone)]
pub struct ImplBlock {
    pub id: NodeId,
    pub generics: Vec<GenericParam>,
    pub target: Ident,
    pub target_args: Vec<TypeExpr>,
    /// The traits in `impl Dog: Sound, Clone` — full type expressions
    /// (not bare
    /// [`Path`]) because a trait name may itself be generic
    /// (language-spec §7.1).
    pub traits: Vec<TypeExpr>,
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
    pub visibility: Visibility,
    pub generics: Vec<GenericParam>,
    /// Traits opted into on the declaration (`enum State: Display`).
    pub traits: Vec<TypeExpr>,
    pub variants: Vec<EnumVariant>,
    pub doc: Option<String>,
    pub span: Span,
}

/// A generic parameter as written, e.g. the `T` in `<T>` or the `T: Sound`
/// / `T: Into<String>` in `<T: Sound>` / `<T: Into<String>>`. `bound` names
/// an [`trait`](TraitDecl), possibly itself generic — a full
/// [`TypeExpr`] rather than a bare [`Path`], for the same reason as
/// [`ImplBlock::trait`]. Language-spec §8 generics are resolved via
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

/// `trait Sound { sound(): String { "..." } }` (language-spec §7).
/// Static dispatch only — an `TraitDecl` never gains a runtime
/// representation; it is erased by monomorphization (see
/// `docs/architecture/type-system.md` §4).
#[derive(Debug, Clone)]
pub struct TraitDecl {
    pub id: NodeId,
    pub name: Ident,
    pub visibility: Visibility,
    /// language-spec §8 lists "generic traits" as supported; empty for
    /// a non-generic declaration.
    pub generics: Vec<GenericParam>,
    /// Direct parent traits (`trait Child: ParentA, ParentB`).
    pub parents: Vec<TypeExpr>,
    /// A method with `body: None` has no default implementation and must
    /// be provided by every `impl`; a method with `body: Some(_)` is a
    /// default, overridable per-impl.
    pub methods: Vec<FnDecl>,
    pub doc: Option<String>,
    pub span: Span,
}

/// A standalone `fn` or an `impl`/`trait` method. There is no `fn`
/// keyword inside an `impl` block (language-spec §6) — the parser only
/// requires the keyword for top-level functions and sets it implicitly for
/// methods; both shapes reuse this one node.
#[derive(Debug, Clone)]
pub struct FnDecl {
    pub id: NodeId,
    pub name: Ident,
    pub visibility: Visibility,
    pub generics: Vec<GenericParam>,
    /// `None` for a static method / standalone function; `Some(_)` for an
    /// instance method (language-spec §6).
    pub self_param: Option<SelfParam>,
    pub params: Vec<Param>,
    pub ret: Option<TypeExpr>,
    /// `None` only for a trait method with no default body
    /// (language-spec §7).
    pub body: Option<Block>,
    pub doc: Option<String>,
    pub span: Span,
}

/// A method's receiver form (language-spec §8.4). The three `Owned*`
/// variants are the unique-ownership-domain counterparts of `ByRef`/
/// `ByMutRef` — a trait/impl may declare both an ARC-domain and an
/// owned-domain overload of the same method name, and receiver mode is
/// part of overload resolution, not merely a mutability flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfParam {
    /// `self` — ordinary ARC receiver.
    ByRef,
    /// `mut self` — ARC receiver with mutation permission.
    ByMutRef,
    /// `: self` — owned, consuming receiver.
    Owned,
    /// `: &self` — borrowed unique receiver.
    OwnedRef,
    /// `: &mut self` — mutable borrowed unique receiver.
    OwnedMutRef,
}

/// A function/method parameter. Five forms exist (language-spec §8.1),
/// distinguished by `mutable` and `ty`'s shape, not by a separate
/// ownership-mode field:
///
/// | Source | `mutable` | `ty` |
/// |---|---|---|
/// | `a Animal` (ordinary ARC) | `false` | `Named(Animal)` |
/// | `b mut Animal` (ARC + mutation permission) | `true` | `Named(Animal)` |
/// | `c: Animal` (owned, moved) | `false` | `Unique(Named(Animal))` |
/// | `d: &Animal` (borrow) | `false` | `Ref(Named(Animal))` |
/// | `e: &mut Animal` (mutable borrow) | `false` | `MutRef(Named(Animal))` |
///
/// Note the `mut` placement for the ARC+mutation form is *after* the name
/// and type-less (`b mut Animal`), deliberately asymmetric with `let mut`
/// (which places `mut` before the name) — this mirrors `mut self`.
#[derive(Debug, Clone)]
pub struct Param {
    /// Identifies this binding site for `resolver`, the same way
    /// [`crate::expr::LetStmt`]'s `id` does.
    pub id: NodeId,
    pub name: Ident,
    pub mutable: bool,
    /// `true` for a trailing variadic parameter (`name ...Type`) that
    /// collects every remaining call-site argument. `ty` is then the
    /// *element* type (`Type`, not `Array<Type>`); a call site collects
    /// its trailing arguments into an `Array<Type>` automatically. Parser
    /// rejects this anywhere but the last parameter.
    pub variadic: bool,
    pub ty: TypeExpr,
    pub span: Span,
}

/// `use user.User;` (language-spec §10).
#[derive(Debug, Clone)]
pub struct UseDecl {
    pub id: NodeId,
    pub visibility: Visibility,
    pub path: Path,
    pub span: Span,
}

/// `mod child;` declares and loads a child file module. The driver maps
/// it to `child.nr` or `child/mod.nr` relative to the declaring module.
#[derive(Debug, Clone)]
pub struct ModDecl {
    pub id: NodeId,
    pub name: Ident,
    pub visibility: Visibility,
    pub span: Span,
}
