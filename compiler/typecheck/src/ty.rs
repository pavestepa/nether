use nether_ast::Symbol;
use nether_resolver::DefId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrimitiveKind {
    Bool,
    Char,
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    Usize,
    Isize,
    F32,
    F64,
}

impl PrimitiveKind {
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "bool" => Self::Bool,
            "char" => Self::Char,
            "i8" => Self::I8,
            "i16" => Self::I16,
            "i32" => Self::I32,
            "i64" => Self::I64,
            "u8" => Self::U8,
            "u16" => Self::U16,
            "u32" => Self::U32,
            "u64" => Self::U64,
            "usize" => Self::Usize,
            "isize" => Self::Isize,
            "f32" => Self::F32,
            "f64" => Self::F64,
            _ => return None,
        })
    }

    pub fn is_integer(self) -> bool {
        !matches!(self, Self::Bool | Self::Char | Self::F32 | Self::F64)
    }

    pub fn is_float(self) -> bool {
        matches!(self, Self::F32 | Self::F64)
    }

    pub fn is_numeric(self) -> bool {
        self.is_integer() || self.is_float()
    }
}

/// Nether's structured internal type representation
/// (`docs/architecture/type-system.md`). Deliberately never a string —
/// every variant here is compared/hashed structurally.
///
/// `Struct`/`TupleStruct`/`Enum`/`Trait` carry a [`DefId`] from
/// `nether_resolver` rather than inventing a separate id space, since one
/// already exists and identifies the same declarations.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Type {
    Primitive(PrimitiveKind),
    /// A `type` declaration with named fields.
    Struct(DefId, Vec<Type>),
    /// A `type Name(A, B);` tuple-struct declaration.
    TupleStruct(DefId, Vec<Type>),
    /// An anonymous tuple — always stack/value (language-spec §3.3),
    /// unlike `Struct`/`TupleStruct`, whose allocation depends on casing.
    Tuple(Vec<Type>),
    /// An `enum` declaration, monomorphization-pending generic arguments
    /// substituted at codegen time; always stack/value regardless of
    /// casing (language-spec §3.3) — see [`crate::alloc::alloc_kind`].
    Enum(DefId, Vec<Type>),
    Array(Box<Type>),
    String,
    /// A closure's type: parameter types and a return type. Distinct from
    /// a named function/method's signature ([`crate::sig::FnSig`]), which
    /// additionally carries `self`/mutability/generics information that a
    /// closure value's *type* doesn't need to expose.
    Function(Vec<Type>, Box<Type>),
    /// Only ever appears inside a generic bound constraint, never as the
    /// type of a value directly (language-spec §7;
    /// `docs/architecture/type-system.md` §4) — enforced in `check.rs`.
    Trait(DefId),
    /// An unsubstituted generic type parameter, scoped to the item
    /// currently being checked. Monomorphization substitutes a concrete
    /// `Type` for this once a generic item is
    /// instantiated; `typecheck` itself only checks bound satisfaction at
    /// call sites (`check.rs`), it does not substitute.
    Generic(Symbol),
    Weak(Box<Type>),
    /// `:T`/`:t` — the uniquely-owned form of `inner` (language-spec §3).
    /// Orthogonal to `inner`'s own representation category (heap vs.
    /// inline, decided by `inner`'s own shape): `Unique(Struct(..))` is
    /// `:T`, `Unique(Primitive(..))` is `:t`. In Stage 1 this is purely a
    /// compile-time-checked distinction — [`crate::alloc::alloc_kind`]
    /// passes straight through to `inner`, so `:T` shares `T`'s ARC
    /// runtime representation until Stage 2's borrow checker can safely
    /// skip retain/release once uniqueness is actually enforced.
    Unique(Box<Type>),
    /// `:&T` — a shared borrow, always within the unique-ownership domain
    /// (language-spec §3.1; there is no bare, always-ARC reference form).
    Ref(Box<Type>),
    /// `:&mut T` — an exclusive borrow, always within the unique-ownership
    /// domain (language-spec §3.1).
    MutRef(Box<Type>),
    /// The type of `return`/`break`/`continue` and other
    /// never-produces-a-value expressions — unifies with any other type
    /// in branch-merging contexts (`if`/`match`), the same technique
    /// Rust's own `!` type uses, so an early return in one arm doesn't
    /// force every other arm to also return.
    Never,
    /// A poison value substituted after a diagnostic has already been
    /// reported for this expression, so later checks don't cascade a
    /// second, redundant error from the same root cause.
    Error,
}

impl Type {
    pub fn unit() -> Type {
        Type::Tuple(Vec::new())
    }

    /// Strips one layer of `Unique` if present, so field/method resolution
    /// works uniformly for an owned value (`:Dog`) the same way it already
    /// does for its ARC counterpart (`Dog`) — language-spec §3 doesn't
    /// give the owned domain a separate field/member namespace, and Stage
    /// 1 has no real receiver-domain overload resolution yet (that's a
    /// later-stage refinement once `: self`/`: &self` overloads need to
    /// be told apart at a call site). Never recurses — `Type::Unique`
    /// never wraps another `Unique` (the grammar has no `::T` form).
    pub fn strip_unique(&self) -> &Type {
        match self {
            Type::Unique(inner) => inner,
            other => other,
        }
    }

    /// Like [`Self::strip_unique`], but also peels a single layer of
    /// `Ref`/`MutRef` — so field/method resolution works the same way
    /// through a `:&T`/`:&mut T` parameter as it already does through
    /// `T`/`:T` (Stage 2, slice 2: `:&T`/`:&mut T` share the referent's
    /// exact runtime representation, a raw pointer with no retain/release,
    /// same as `:T` already shares `T`'s — see
    /// `docs/architecture/roadmap.md`).
    ///
    /// Peels exactly one layer, not a full chain — `:&&mut T` (`Unique(
    /// Ref(MutRef(T)))`) needs two calls to fully unwrap. This slice only
    /// targets `:&T`/`:&mut T` used directly as an ordinary parameter
    /// type, not arbitrary multi-level reference chains, so one layer is
    /// what every call site here actually needs; deeper chains remain
    /// exactly as unsupported for field/method access as before this
    /// slice.
    pub fn strip_indirection(&self) -> &Type {
        match self {
            Type::Unique(inner) | Type::Ref(inner) | Type::MutRef(inner) => inner,
            other => other,
        }
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Type::Error)
    }

    /// Whether this type still contains a poison/unknown component.
    ///
    /// Besides suppressing cascaded diagnostics, this is used at
    /// inference boundaries to ensure an unresolved generic argument
    /// (for example a context-free `Option.None`) never reaches
    /// monomorphization or codegen.
    pub fn contains_error(&self) -> bool {
        match self {
            Type::Error => true,
            Type::Struct(_, args)
            | Type::TupleStruct(_, args)
            | Type::Tuple(args)
            | Type::Enum(_, args) => args.iter().any(Type::contains_error),
            Type::Array(inner) | Type::Weak(inner) | Type::Unique(inner) | Type::Ref(inner)
            | Type::MutRef(inner) => inner.contains_error(),
            Type::Function(params, ret) => {
                params.iter().any(Type::contains_error) || ret.contains_error()
            }
            _ => false,
        }
    }

    /// Whether this type still depends on an unsubstituted generic
    /// parameter.
    pub fn contains_generic(&self) -> bool {
        match self {
            Type::Generic(_) => true,
            Type::Struct(_, args)
            | Type::TupleStruct(_, args)
            | Type::Tuple(args)
            | Type::Enum(_, args) => args.iter().any(Type::contains_generic),
            Type::Array(inner) | Type::Weak(inner) | Type::Unique(inner) | Type::Ref(inner)
            | Type::MutRef(inner) => inner.contains_generic(),
            Type::Function(params, ret) => {
                params.iter().any(Type::contains_generic) || ret.contains_generic()
            }
            _ => false,
        }
    }

    /// Structural equality that treats [`Type::Error`]/[`Type::Never`] as
    /// compatible with anything, so one earlier mistyped expression
    /// doesn't produce a second, unrelated "type mismatch" diagnostic
    /// downstream, and so a `return`/`break` arm doesn't force sibling
    /// `if`/`match` arms to match its (nonexistent) value type.
    ///
    /// One asymmetric, one-directional exception: a `T` value is
    /// `compatible` with an *expected* `weak T` (this is `weak T`'s only
    /// construction syntax — implicit coercion at the assignment/field
    /// -init/call-argument site, per `arc-model.md`'s weak-reference
    /// section). The reverse never holds — a `weak T` place is never
    /// itself an expression's type; reading one always already produced
    /// an `Option<T>` before `compatible` is ever consulted (see
    /// `check::TypeChecker::upgrade_weak`) — so there is no matching case
    /// for `self` being `Weak`.
    pub fn compatible(&self, other: &Type) -> bool {
        if matches!(self, Type::Error | Type::Never)
            || matches!(other, Type::Error | Type::Never)
            || self == other
        {
            return true;
        }
        match (self, other) {
            (Type::Tuple(a), Type::Tuple(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.compatible(b))
            }
            (Type::Enum(a_id, a), Type::Enum(b_id, b)) => {
                a_id == b_id && a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.compatible(b))
            }
            (Type::Struct(a_id, a), Type::Struct(b_id, b))
            | (Type::TupleStruct(a_id, a), Type::TupleStruct(b_id, b)) => {
                a_id == b_id && a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.compatible(b))
            }
            (Type::Array(a), Type::Array(b)) | (Type::Weak(a), Type::Weak(b)) => a.compatible(b),
            (Type::Function(a_params, a_ret), Type::Function(b_params, b_ret)) => {
                a_params.len() == b_params.len()
                    && a_params.iter().zip(b_params).all(|(a, b)| a.compatible(b))
                    && a_ret.compatible(b_ret)
            }
            // No implicit ownership-domain coercion in either direction
            // (language-spec §9, §23): a `T` value is never `compatible`
            // with an expected `:T`, and vice versa — only matched shapes
            // recurse into their inner type.
            (Type::Unique(a), Type::Unique(b))
            | (Type::Ref(a), Type::Ref(b))
            | (Type::MutRef(a), Type::MutRef(b)) => a.compatible(b),
            (_, Type::Weak(inner)) => **inner == *self,
            _ => false,
        }
    }
}
