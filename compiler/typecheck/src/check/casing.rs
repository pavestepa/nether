use super::*;

/// language-spec §5: a `type Name = TypeExpr;` alias's casing must match
/// its *resolved* representation category — PascalCase names a
/// heap/reference type, lowercase names an inline/value type. Structs and
/// enums are not checked here: their own casing *is* definitionally where
/// their category comes from (checking them against themselves would be a
/// tautology) — only an alias can misrepresent what it names. Enums and
/// tuples are exempt regardless of casing, mirroring
/// `crate::alloc::alloc_kind`'s own carve-out, so an alias for `Option`/
/// `Result`-shaped data never fails this check.
pub(super) fn validate_alias_casing(
    module: &Module,
    resolved: &ResolvedNames,
    decls: &DeclIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for item in &module.items {
        let Item::TypeAlias(alias) = item else {
            continue;
        };
        if alias.allow_pascal_case {
            continue;
        }
        let resolved_ty = lower_type_expr(&alias.ty, resolved, decls, diagnostics);
        if resolved_ty.is_error() || matches!(resolved_ty, Type::Enum(_, _) | Type::Tuple(_)) {
            continue;
        }
        let is_pascal = alias
            .name
            .name
            .as_str()
            .chars()
            .next()
            .is_some_and(char::is_uppercase);
        let kind = crate::alloc::alloc_kind(&resolved_ty, &resolved.definitions);
        let described = describe_type(&resolved_ty, resolved);
        match (is_pascal, kind) {
            (true, crate::alloc::AllocKind::Stack) => diagnostics.push(
                Diagnostic::error(format!(
                    "type alias `{}` is PascalCase but resolves to inline/value type `{described}`",
                    alias.name.name
                ))
                .with_label(alias.span, "PascalCase implies a heap/reference type")
                .with_hint("rename the alias to lowercase, or point it at a heap type"),
            ),
            (false, crate::alloc::AllocKind::Heap) => diagnostics.push(
                Diagnostic::error(format!(
                    "type alias `{}` is lowercase but resolves to heap/reference type `{described}`",
                    alias.name.name
                ))
                .with_label(alias.span, "lowercase implies an inline/value type")
                .with_hint("rename the alias to PascalCase, or point it at an inline type"),
            ),
            _ => {}
        }
    }
}
