use super::*;

const PRIMITIVES: &[&str] = &[
    "bool", "char", "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "usize", "isize", "f32",
    "f64",
];

/// Seeds a fresh [`Definitions`] table with everything the language spec
/// makes available with no `use` (§12) plus the primitives and built-in
/// heap types the lexer deliberately does *not* treat as keywords (§2.2).
fn builtin_definitions() -> Definitions {
    let mut defs = Definitions::default();
    for name in PRIMITIVES {
        defs.insert_builtin(Symbol::new(name), DefKind::Primitive);
    }
    defs.insert_builtin(Symbol::new("String"), DefKind::Type);

    // `Array`, like `Option`/`Result`, is *not* seeded here — it's an
    // ordinary generic `type Array<T>;` declaration in
    // `stdlib/array.nt`, reachable everywhere the same way any other name
    // in the bundled prelude is (`stdlib/mod.nt`'s own `use`, promoted by
    // `Definitions::promote_to_prelude` — see `nether_driver`'s module
    // docs). This is what a compiler-builtin generic type actually needs
    // to be written in Nether source with no special support at all:
    // `type`/`enum` + `impl<T> Owner<T> { ... }` already exists as
    // ordinary language features. `String` stays a true builtin above —
    // it has no user-facing generic parameter and no user `impl` blocks
    // are expected to extend it.

    defs.insert_builtin(Symbol::new("println"), DefKind::Fn);
    defs.insert_builtin(Symbol::new("print"), DefKind::Fn);

    // `Into<T>` is the compiler-known conversion interface from
    // language-spec §7.1. Its concrete `Into<String>` convention is used
    // by interpolation and the print builtins; user-defined conversions
    // still use ordinary `impl Type: Into<T>` blocks.
    defs.insert_builtin(Symbol::new("Into"), DefKind::Interface);

    defs
}

/// Builds the top-level namespace for `module`: builtins, then every
/// user-declared `type`/`enum`/`interface`/`fn`, then merges each `impl`
/// block's method names into its target type's or enum's method list.
///
/// Duplicate top-level names produce a diagnostic and keep the first
/// definition (so name resolution can still proceed for the rest of the
/// module rather than cascading unresolved-name errors).
pub fn collect(module: &Module, prelude_file: Option<FileId>) -> (Definitions, Vec<Diagnostic>) {
    let mut defs = builtin_definitions();
    let mut diags = Vec::new();

    for item in &module.items {
        match item {
            Item::Type(t) => {
                defs.insert_checked(&t.name, DefKind::Type, &mut diags);
            }
            Item::Enum(e) => {
                let id = defs.insert_checked(&e.name, DefKind::Enum, &mut diags);
                let variants = e.variants.iter().map(|v| v.name.name.clone()).collect();
                defs.defs[id.0 as usize].variants = variants;
            }
            Item::Interface(i) => {
                let id = defs.insert_checked(&i.name, DefKind::Interface, &mut diags);
                defs.defs[id.0 as usize].methods =
                    i.methods.iter().map(|m| m.name.name.clone()).collect();
            }
            Item::Fn(f) => {
                defs.insert_checked(&f.name, DefKind::Fn, &mut diags);
            }
            Item::Impl(_) | Item::Use(_) | Item::Mod(_) => {}
        }
    }

    fn interface_id(ty: &TypeExpr, defs: &Definitions) -> Option<DefId> {
        let TypeExpr::Named { path, .. } = ty else {
            return None;
        };
        let name = path.segments.first()?;
        let id = defs.lookup_in(path.span.file, &name.name)?;
        (defs.get(id).kind == DefKind::Interface).then_some(id)
    }

    let interface_parents: HashMap<DefId, Vec<DefId>> = module
        .items
        .iter()
        .filter_map(|item| {
            let Item::Interface(interface) = item else {
                return None;
            };
            let id = defs.lookup_in(interface.span.file, &interface.name.name)?;
            Some((
                id,
                interface
                    .parents
                    .iter()
                    .filter_map(|parent| interface_id(parent, &defs))
                    .collect(),
            ))
        })
        .collect();

    fn inherited_method_names(
        interface: DefId,
        defs: &Definitions,
        parents: &HashMap<DefId, Vec<DefId>>,
        visiting: &mut Vec<DefId>,
    ) -> Vec<Symbol> {
        if visiting.contains(&interface) {
            return Vec::new();
        }
        visiting.push(interface);
        let mut names = defs.get(interface).methods.clone();
        for parent in parents.get(&interface).into_iter().flatten() {
            for name in inherited_method_names(*parent, defs, parents, visiting) {
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        visiting.pop();
        names
    }

    let mut declared_members = Vec::new();
    for item in &module.items {
        let (owner, interfaces) = match item {
            Item::Type(decl) => (
                defs.lookup_in(decl.span.file, &decl.name.name),
                decl.interfaces.as_slice(),
            ),
            Item::Enum(decl) => (
                defs.lookup_in(decl.span.file, &decl.name.name),
                decl.interfaces.as_slice(),
            ),
            Item::Impl(block) => (
                defs.lookup_in(block.span.file, &block.target.name),
                block.interfaces.as_slice(),
            ),
            _ => continue,
        };
        let Some(owner) = owner else { continue };
        let mut names = Vec::new();
        for interface in interfaces {
            let Some(interface) = interface_id(interface, &defs) else {
                continue;
            };
            for name in
                inherited_method_names(interface, &defs, &interface_parents, &mut Vec::new())
            {
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        declared_members.push((owner, names));
    }
    for (owner, names) in declared_members {
        for name in names {
            if !defs.defs[owner.0 as usize].methods.contains(&name) {
                defs.defs[owner.0 as usize].methods.push(name);
            }
        }
    }

    for item in &module.items {
        let Item::Use(use_decl) = item else { continue };
        // `use module.Enum.Variant;` is handled by its own loop below —
        // it's tracked in `variant_imports`, not `imports`, so it must
        // not also fall into this loop's `declare_imported` fallback
        // (which would otherwise bind the variant name to a bogus opaque
        // `DefKind::Imported` placeholder, shadowing the real resolution).
        if module.variant_imports.contains_key(&use_decl.id) {
            continue;
        }
        let Some(imported_name) = use_decl.path.segments.last() else {
            continue;
        };
        if let Some(target_file) = module.imports.get(&use_decl.id) {
            match defs
                .by_file_name
                .get(&(*target_file, imported_name.name.clone()))
                .copied()
            {
                Some(id) => defs.import(use_decl.span.file, imported_name.name.clone(), id),
                None => diags.push(
                    Diagnostic::error(format!("module does not define `{}`", imported_name.name))
                        .with_label(imported_name.span, "not found in this module"),
                ),
            }
        } else {
            defs.declare_imported(use_decl.span.file, imported_name.name.clone());
        }
    }

    // `use module.Enum.Variant;` — the trailing *two* segments name an
    // enum and one of its variants within the target file, rather than
    // the trailing one plain name the loop above handles.
    for item in &module.items {
        let Item::Use(use_decl) = item else { continue };
        let Some(target_file) = module.variant_imports.get(&use_decl.id) else {
            continue;
        };
        let segments = &use_decl.path.segments;
        let Some(enum_seg) = segments.get(segments.len().wrapping_sub(2)) else {
            continue;
        };
        let Some(variant_seg) = segments.last() else {
            continue;
        };
        match defs
            .by_file_name
            .get(&(*target_file, enum_seg.name.clone()))
            .copied()
        {
            Some(id) if defs.get(id).kind == DefKind::Enum => {
                match variant_index(defs.get(id), &variant_seg.name) {
                    Some(idx) => {
                        defs.import_variant(use_decl.span.file, variant_seg.name.clone(), (id, idx))
                    }
                    None => diags.push(
                        Diagnostic::error(format!(
                            "enum `{}` has no member `{}`",
                            enum_seg.name, variant_seg.name
                        ))
                        .with_label(variant_seg.span, "not found in this enum"),
                    ),
                }
            }
            Some(_) => diags.push(
                Diagnostic::error(format!("`{}` is not an enum", enum_seg.name))
                    .with_label(enum_seg.span, "expected an enum"),
            ),
            None => diags.push(
                Diagnostic::error(format!("module does not define `{}`", enum_seg.name))
                    .with_label(enum_seg.span, "not found in this module"),
            ),
        }
    }

    // The bundled prelude (`stdlib/mod.nt`) re-exports its own top-level
    // `use` names to every file with no `use` of their own — see
    // `Definitions::promote_to_prelude`.
    if let Some(prelude_file) = prelude_file {
        for item in &module.items {
            let Item::Use(use_decl) = item else { continue };
            if use_decl.span.file != prelude_file {
                continue;
            }
            let Some(imported_name) = use_decl.path.segments.last() else {
                continue;
            };
            if let Some(id) = defs.lookup_in(prelude_file, &imported_name.name) {
                defs.promote_to_prelude(imported_name.name.clone(), id);
            }
        }
        for item in &module.items {
            let Item::Use(use_decl) = item else { continue };
            if use_decl.span.file != prelude_file
                || !module.variant_imports.contains_key(&use_decl.id)
            {
                continue;
            }
            let Some(name) = use_decl.path.segments.last() else {
                continue;
            };
            if let Some(target) = defs.lookup_variant_in(prelude_file, &name.name) {
                defs.promote_variant_to_prelude(name.name.clone(), target);
            }
        }
    }

    for item in &module.items {
        if let Item::Impl(impl_block) = item {
            match defs.lookup_in(impl_block.span.file, &impl_block.target.name) {
                Some(id) if matches!(defs.get(id).kind, DefKind::Type | DefKind::Enum) => {
                    let names: Vec<Symbol> = impl_block
                        .methods
                        .iter()
                        .map(|m| m.name.name.clone())
                        .collect();
                    for name in names {
                        if !defs.defs[id.0 as usize].methods.contains(&name) {
                            defs.defs[id.0 as usize].methods.push(name);
                        }
                    }
                }
                Some(_) => {
                    diags.push(
                        Diagnostic::error(format!(
                            "`{}` is not a type or enum and cannot have an `impl` block",
                            impl_block.target.name
                        ))
                        .with_label(impl_block.target.span, "here"),
                    );
                }
                None => {
                    diags.push(
                        Diagnostic::error(format!(
                            "cannot find type `{}` for this `impl` block",
                            impl_block.target.name
                        ))
                        .with_label(impl_block.target.span, "here"),
                    );
                }
            }
        }
    }

    (defs, diags)
}
