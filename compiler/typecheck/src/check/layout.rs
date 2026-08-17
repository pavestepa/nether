use super::*;

pub(super) fn owner_as_type(id: DefId, resolved: &ResolvedNames, decls: &DeclIndex) -> Type {
    let args: Vec<Type> = decls
        .type_decls
        .get(&id)
        .map(|decl| {
            decl.generics
                .iter()
                .map(|generic| Type::Generic(generic.name.name.clone()))
                .collect()
        })
        .or_else(|| {
            decls.enum_decls.get(&id).map(|decl| {
                decl.generics
                    .iter()
                    .map(|generic| Type::Generic(generic.name.name.clone()))
                    .collect()
            })
        })
        .unwrap_or_default();
    // `Array<T>` still lowers to the distinct `Type::Array` representation
    // (codegen/ARC layout depend on it), not `Type::Struct` — even though
    // it's now an ordinary `type Array<T>;` declaration like any other.
    if resolved.definitions.get(id).name.as_str() == "Array" {
        return Type::Array(Box::new(args.into_iter().next().unwrap_or(Type::Error)));
    }
    match resolved.definitions.get(id).kind {
        DefKind::Enum => Type::Enum(id, args),
        _ => match decls.type_decls.get(&id).map(|t| &t.kind) {
            Some(StructDeclKind::TupleStruct(_)) => Type::TupleStruct(id, args),
            _ => Type::Struct(id, args),
        },
    }
}

/// Like [`owner_as_type`], but takes its generic argument names from an
/// already-resolved owner-generics list instead of re-reading a
/// declaration. The only way to get a non-empty owner's type for a
/// declaration-less builtin (`Option`, `Result`) under an explicit
/// `impl<T> Option<T>: SomeTrait { ... }` block.
pub(super) fn owner_as_type_from_generics(
    id: DefId,
    resolved: &ResolvedNames,
    decls: &DeclIndex,
    owner_generics: &[(Symbol, Option<GenericBound>)],
) -> Type {
    let args: Vec<Type> = owner_generics
        .iter()
        .map(|(name, _)| Type::Generic(name.clone()))
        .collect();
    // See `owner_as_type`'s matching comment: `Array<T>` keeps its own
    // distinct `Type` variant regardless of generic-scope source.
    if resolved.definitions.get(id).name.as_str() == "Array" {
        return Type::Array(Box::new(args.into_iter().next().unwrap_or(Type::Error)));
    }
    match resolved.definitions.get(id).kind {
        DefKind::Enum => Type::Enum(id, args),
        _ => match decls.type_decls.get(&id).map(|t| &t.kind) {
            Some(StructDeclKind::TupleStruct(_)) => Type::TupleStruct(id, args),
            _ => Type::Struct(id, args),
        },
    }
}

pub(super) fn validate_finite_value_layouts(
    resolved: &ResolvedNames,
    decls: &DeclIndex,
    sigs: &Signatures,
    diags: &mut Vec<Diagnostic>,
) {
    for (&id, decl) in &decls.type_decls {
        let ty = owner_as_type(id, resolved, decls);
        if crate::alloc::alloc_kind(&ty, &resolved.definitions) == crate::alloc::AllocKind::Heap {
            continue;
        }
        if value_layout_reaches_cycle(&ty, resolved, sigs, &mut Vec::new()) {
            diags.push(
                Diagnostic::error(format!(
                    "value type `{}` has an infinitely recursive layout",
                    decl.name.name
                ))
                .with_label(
                    decl.span,
                    "introduce a heap-allocated PascalCase type or another indirection",
                ),
            );
        }
    }
    for (&id, decl) in &decls.enum_decls {
        let ty = owner_as_type(id, resolved, decls);
        if value_layout_reaches_cycle(&ty, resolved, sigs, &mut Vec::new()) {
            diags.push(
                Diagnostic::error(format!(
                    "enum `{}` has an infinitely recursive layout",
                    decl.name.name
                ))
                .with_label(
                    decl.span,
                    "introduce a heap-allocated PascalCase type or another indirection",
                ),
            );
        }
    }
}

pub(super) fn value_layout_reaches_cycle(
    ty: &Type,
    resolved: &ResolvedNames,
    sigs: &Signatures,
    stack: &mut Vec<DefId>,
) -> bool {
    match ty {
        Type::Struct(id, _) | Type::TupleStruct(id, _) => {
            if crate::alloc::alloc_kind(ty, &resolved.definitions) == crate::alloc::AllocKind::Heap
            {
                return false;
            }
            if stack.contains(id) {
                return true;
            }
            stack.push(*id);
            let recursive = sigs
                .type_fields(ty)
                .unwrap_or_default()
                .iter()
                .any(|field| value_layout_reaches_cycle(field, resolved, sigs, stack));
            stack.pop();
            recursive
        }
        Type::Enum(id, _) => {
            if stack.contains(id) {
                return true;
            }
            stack.push(*id);
            let recursive = sigs.enum_sigs.get(id).is_some_and(|sig| {
                (0..sig.variants.len()).any(|variant| {
                    sigs.enum_payload(ty, variant as u32)
                        .unwrap_or_default()
                        .iter()
                        .any(|field| value_layout_reaches_cycle(field, resolved, sigs, stack))
                })
            });
            stack.pop();
            recursive
        }
        Type::Tuple(items) => items
            .iter()
            .any(|item| value_layout_reaches_cycle(item, resolved, sigs, stack)),
        // `:T`/`:t` share their inner type's layout (Stage 1, language-spec
        // §3.2 — no distinct unique-inline representation yet), so a
        // recursive owned-inline field is exactly as cyclic as a bare one.
        Type::Unique(inner) => value_layout_reaches_cycle(inner, resolved, sigs, stack),
        // These all provide an indirection or have a fixed scalar layout.
        Type::Array(_)
        | Type::String
        | Type::Function(_, _)
        | Type::Weak(_)
        | Type::Ref(_)
        | Type::MutRef(_)
        | Type::Primitive(_)
        | Type::Trait(_)
        | Type::Generic(_)
        | Type::Never
        | Type::Error => false,
    }
}
