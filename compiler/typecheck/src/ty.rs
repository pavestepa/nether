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

/// How a closure captures one free variable from its enclosing scope
/// (language-spec §16, Stage 7's `mut (...) => {}` closures).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureMode {
    /// Copies the value into the closure's own environment at creation
    /// time — mutating it inside the body never affects the outer
    /// binding. The only mode before Stage 7, and still the only one for
    /// an ordinary or `move` closure.
    ByValue,
    /// `mut (...) => {}` only, and only for a capture whose outer local
    /// is itself declared `mut`: the environment stores the address of
    /// the outer local's own storage instead of a copy of its value, so
    /// every read/write inside the closure body transparently reads/
    /// writes the *same* storage the outer binding uses — the captured
    /// local's own type stays its ordinary `T` throughout typecheck/HIR;
    /// only `nether_monomorphization`/`nether_mir`/`nether_codegen` know
    /// the storage is aliased.
    ByRef,
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
    /// Compile-time integer argument in a generic argument list.
    Const(u128),
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
    /// Inline fixed-size array `{T, N}`. `N` is `Const` after monomorphization
    /// and may be `Generic` while checking a generic body.
    FixedArray(Box<Type>, Box<Type>),
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
    /// `any Trait<...>` existential package.
    Any(DefId, Vec<Type>),
    /// `some Trait<...>` opaque value inside its declaring API.
    Some(DefId, Vec<Type>),
    /// A suspended computation produced by calling an `async fn`.
    Task(Box<Type>),
    /// An opaque handle to a real OS thread produced by `thread.spawn`
    /// (language-spec §19, Stage 6). Structurally mirrors `Task`'s own
    /// `Type`-level footprint (an ordinary heap-managed pointer with a
    /// generic output type — `codegen/src/layout.rs`, `shims.rs`) but
    /// carries none of `Task`'s async-frame machinery: `.join()` is a
    /// plain blocking builtin call, never a suspension point, so nothing
    /// in `frame.rs`/`function/async_fn.rs` needs to know about this
    /// type at all.
    Thread(Box<Type>),
    /// An unsubstituted generic type parameter, scoped to the item
    /// currently being checked. Monomorphization substitutes a concrete
    /// `Type` for this once a generic item is
    /// instantiated; `typecheck` itself only checks bound satisfaction at
    /// call sites (`check.rs`), it does not substitute.
    Generic(Symbol),
    /// `T.Item` (or a projection whose owner has not yet been normalized).
    Associated(Box<Type>, Symbol),
    Weak(Box<Type>),
    /// `:T`/`:t` — the uniquely-owned form of `inner` (language-spec §3).
    /// Orthogonal to `inner`'s own representation category (heap vs.
    /// inline, decided by `inner`'s own shape): `Unique(Struct(..))` is
    /// `:T`, `Unique(Primitive(..))` is `:t`. HIR preserves heap `Unique`
    /// through MIR/codegen for unrefcounted allocation and move transfer;
    /// inline `Unique` is erased after type checking because its bits are
    /// identical to the inner value.
    Unique(Box<Type>),
    /// `:&T` — a shared borrow, always within the unique-ownership domain
    /// (language-spec §3.1; there is no bare, always-ARC reference form).
    Ref(Box<Type>),
    /// `:&mut T` — an exclusive borrow, always within the unique-ownership
    /// domain (language-spec §3.1).
    MutRef(Box<Type>),
    /// `*const T` — a raw, unchecked pointer (language-spec §17, Stage
    /// 5). No borrow-tracking participates in forming one (`&raw const`/
    /// `&raw mut` bypass the unique-ownership borrow system entirely —
    /// see `nether_hir`'s own lowering docs); only *dereferencing* one
    /// requires an `unsafe` context.
    RawConstPtr(Box<Type>),
    /// `*mut T` — a raw, unchecked mutable pointer (language-spec §17,
    /// Stage 5). See [`Type::RawConstPtr`]'s own docs.
    RawMutPtr(Box<Type>),
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
            Type::Any(_, args) | Type::Some(_, args) => args.iter().any(Type::contains_error),
            Type::FixedArray(element, length) => {
                element.contains_error() || length.contains_error()
            }
            Type::Array(inner)
            | Type::Associated(inner, _)
            | Type::Weak(inner)
            | Type::Unique(inner)
            | Type::Ref(inner)
            | Type::MutRef(inner)
            | Type::RawConstPtr(inner)
            | Type::RawMutPtr(inner) => inner.contains_error(),
            Type::Task(output) | Type::Thread(output) => output.contains_error(),
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
            Type::Any(_, args) | Type::Some(_, args) => args.iter().any(Type::contains_generic),
            Type::FixedArray(element, length) => {
                element.contains_generic() || length.contains_generic()
            }
            Type::Array(inner)
            | Type::Associated(inner, _)
            | Type::Weak(inner)
            | Type::Unique(inner)
            | Type::Ref(inner)
            | Type::MutRef(inner)
            | Type::RawConstPtr(inner)
            | Type::RawMutPtr(inner) => inner.contains_generic(),
            Type::Task(output) | Type::Thread(output) => output.contains_generic(),
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
            (Type::Array(a), Type::Array(b))
            | (Type::Weak(a), Type::Weak(b))
            | (Type::Task(a), Type::Task(b))
            | (Type::Thread(a), Type::Thread(b)) => a.compatible(b),
            (Type::FixedArray(a_elem, a_len), Type::FixedArray(b_elem, b_len)) => {
                a_len == b_len && a_elem.compatible(b_elem)
            }
            (Type::Associated(a_owner, a_name), Type::Associated(b_owner, b_name)) => {
                a_name == b_name && a_owner.compatible(b_owner)
            }
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
            | (Type::MutRef(a), Type::MutRef(b))
            | (Type::RawConstPtr(a), Type::RawConstPtr(b))
            | (Type::RawMutPtr(a), Type::RawMutPtr(b)) => a.compatible(b),
            // `*mut T` degrades implicitly to an expected `*const T` (one
            // direction only — mirrors Rust's own raw-pointer coercion);
            // the reverse never holds, same asymmetry as the `Weak` case
            // right below.
            (Type::RawMutPtr(a), Type::RawConstPtr(b)) => a.compatible(b),
            (_, Type::Weak(inner)) => **inner == *self,
            _ => false,
        }
    }
}
