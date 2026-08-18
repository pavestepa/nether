use crate::ty::Type;
use nether_resolver::Definitions;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocKind {
    Heap,
    Stack,
}

/// Determines a type's representation category — heap/reference (`T`) vs.
/// inline/value (`t`), language-spec §3/§4.4.
///
/// For a `struct` declaration this is still, as before this rewrite,
/// determined by the declaration's own casing (PascalCase → heap/ARC,
/// lowercase → inline) — there is no other signal for a hand-authored
/// aggregate's representation category, and the language spec's own §25
/// example (`struct point { x f32, y f32 }`, explicitly inline/`Copy`)
/// confirms lowercase `struct`s are legal and inline. What Stage 1
/// actually changes is that this is no longer the *only* thing casing
/// does: a `type Name = TypeExpr;` alias's casing is instead cross-checked
/// against its *resolved* target's category rather than determining
/// anything itself (language-spec §5 — see `crate::casing`), since an
/// alias could otherwise "lie" about what it names.
///
/// Enums and tuples are *always* inline regardless of name, the one
/// exemption from the naming rule (kept so `Option`/`Result`/`match` stay
/// zero-cost) — carried forward unchanged from the pre-rewrite compiler
/// and now also documented as a deliberate carve-out in language-spec §5.
///
/// The unique-ownership qualifier (`Type::Unique`) is orthogonal to
/// representation category and passes straight through to its inner
/// type's `alloc_kind` — in Stage 1, `:T` shares `T`'s ARC representation
/// (language-spec §3.2). References (`Type::Ref`/`Type::MutRef`) are
/// themselves always stack-representable pointers, the same treatment as
/// `Weak` below.
pub fn alloc_kind(ty: &Type, defs: &Definitions) -> AllocKind {
    match ty {
        Type::Primitive(_) | Type::Const(_) | Type::FixedArray(_, _) => AllocKind::Stack,
        Type::String | Type::Array(_) | Type::Any(_, _) | Type::Some(_, _) | Type::Task(_) => {
            AllocKind::Heap
        }
        Type::Struct(id, _) | Type::TupleStruct(id, _) => {
            if is_pascal_case(defs.get(*id).name.as_str()) {
                AllocKind::Heap
            } else {
                AllocKind::Stack
            }
        }
        Type::Tuple(_) => AllocKind::Stack,
        Type::Enum(_, _) => AllocKind::Stack,
        // A closure value is an ARC-managed environment object containing
        // its code pointer and captured values. Named function values use
        // the same zero-capture representation, keeping first-class calls
        // uniform.
        Type::Function(_, _) => AllocKind::Heap,
        Type::Weak(_) | Type::Ref(_) | Type::MutRef(_) => AllocKind::Stack,
        Type::Unique(inner) => alloc_kind(inner, defs),
        Type::Generic(_) | Type::Associated(_, _) => AllocKind::Stack, // meaningless before substitution; never queried before monomorphization in practice
        Type::Trait(_) | Type::Never | Type::Error => AllocKind::Stack,
    }
}

fn is_pascal_case(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_uppercase())
}
