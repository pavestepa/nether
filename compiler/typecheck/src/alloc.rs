use crate::ty::Type;
use nether_resolver::Definitions;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocKind {
    Heap,
    Stack,
}

/// Implements language-spec §3.3's heap-vs-stack rule, precisely as
/// specified in `docs/architecture/type-system.md` §3: PascalCase `type`
/// declarations (and the built-in `String`/`Array`) are heap/ARC;
/// camelCase `type` declarations and primitives are stack/value; enums and
/// tuples are *always* stack/value regardless of name, the one exemption
/// from the naming rule (kept so `Option`/`Result`/`match` stay zero-cost).
pub fn alloc_kind(ty: &Type, defs: &Definitions) -> AllocKind {
    match ty {
        Type::Primitive(_) => AllocKind::Stack,
        Type::String | Type::Array(_) => AllocKind::Heap,
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
        Type::Weak(_) => AllocKind::Stack,
        Type::Generic(_) => AllocKind::Stack, // meaningless before substitution; never queried before monomorphization in practice
        Type::Interface(_) | Type::Never | Type::Error => AllocKind::Stack,
    }
}

fn is_pascal_case(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_uppercase())
}
