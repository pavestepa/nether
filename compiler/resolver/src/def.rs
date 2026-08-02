use std::collections::HashMap;

use nether_ast::{Item, Module, Symbol, TypeExpr};
use nether_diagnostics::{Diagnostic, FileId};

use crate::resolve::variant_index;

/// Identifies one top-level definition (a primitive, a `type`, an `enum`,
/// an `interface`, or a `fn`) for the lifetime of one [`resolve`](crate::resolve)
/// call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DefId(pub(crate) u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefKind {
    /// `i32`, `bool`, ... — never user-declared (language-spec §2.2: these
    /// are deliberately not lexer keywords, so they are recognized here
    /// instead, the first point in the pipeline that needs to know they
    /// exist).
    Primitive,
    /// A struct, tuple-struct, or unit `type` declaration, or a built-in
    /// heap type (`String`, `Array`).
    Type,
    /// An `enum` declaration — including `Option`/`Result`, ordinary
    /// generic enums declared in the bundled prelude, not builtins.
    Enum,
    Interface,
    /// A standalone `fn`, or a built-in function (`println`, `print`).
    Fn,
    /// Compatibility placeholder used only when `resolver` is invoked on
    /// a standalone parsed file without the driver's module graph.
    Imported,
}

/// One top-level definition's resolver-relevant facts. Field/parameter
/// *types* and full member-existence-with-type-context checks are
/// `typecheck`'s job (`docs/architecture/crates.md` § `typecheck`) — this
/// crate only tracks enough to resolve names: which enum variants exist,
/// and which method names an `impl` block contributed to a type/enum.
#[derive(Debug)]
pub struct Def {
    pub name: Symbol,
    pub kind: DefKind,
    pub variants: Vec<Symbol>,
    pub methods: Vec<Symbol>,
}

/// The top-level namespace for one resolved module: every primitive,
/// built-in, and user-declared `type`/`enum`/`interface`/`fn`, keyed by
/// name.
#[derive(Default)]
pub struct Definitions {
    defs: Vec<Def>,
    by_name: HashMap<Symbol, DefId>,
    builtins: HashMap<Symbol, DefId>,
    by_file_name: HashMap<(FileId, Symbol), DefId>,
    imports: HashMap<(FileId, Symbol), DefId>,
    /// Names re-exported by the bundled prelude file (`stdlib/mod.nt`'s
    /// own top-level `use` declarations) — visible from every file with no
    /// `use` of their own, like `builtins`, but with lower priority: a
    /// local declaration of the same name is a legal shadow, not a
    /// duplicate-definition error (see [`Self::insert_checked`], which
    /// deliberately does not consult this map). Populated by
    /// [`collect`](super::collect) once name resolution knows the
    /// prelude file's own imports.
    prelude: HashMap<Symbol, DefId>,
    /// A name bound by `use module.Enum.Variant;` — a variant has no
    /// `DefId` of its own (unlike a top-level `type`/`enum`/`fn`), so
    /// this maps straight to its owning enum's `DefId` plus its variant
    /// index, mirroring `imports`/`prelude` one level down.
    variant_imports: HashMap<(FileId, Symbol), (DefId, u32)>,
    /// Like `prelude`, but for a variant name re-exported by the bundled
    /// prelude file's own `use module.Enum.Variant;` (e.g. `Some`/`None`).
    variant_prelude: HashMap<Symbol, (DefId, u32)>,
}

impl Definitions {
    pub fn get(&self, id: DefId) -> &Def {
        &self.defs[id.0 as usize]
    }

    pub fn lookup(&self, name: &Symbol) -> Option<DefId> {
        self.by_name.get(name).copied()
    }

    pub fn lookup_in(&self, file: FileId, name: &Symbol) -> Option<DefId> {
        self.by_file_name
            .get(&(file, name.clone()))
            .or_else(|| self.imports.get(&(file, name.clone())))
            .or_else(|| self.builtins.get(name))
            .or_else(|| self.prelude.get(name))
            .copied()
    }

    pub fn visible_in(&self, file: FileId) -> Vec<DefId> {
        let mut ids = self.builtins.values().copied().collect::<Vec<_>>();
        ids.extend(self.prelude.values().copied());
        ids.extend(
            self.by_file_name
                .iter()
                .filter_map(|((item_file, _), id)| (*item_file == file).then_some(*id)),
        );
        ids.extend(
            self.imports
                .iter()
                .filter_map(|((import_file, _), id)| (*import_file == file).then_some(*id)),
        );
        ids.sort_by_key(|id| id.0);
        ids.dedup();
        ids
    }

    /// Re-exports `name` (resolved as seen from the prelude file itself)
    /// so every other file can see it with no `use`. Never overrides a
    /// true builtin/primitive of the same name — `lookup_in` already
    /// checks `builtins` first, but keeping the map itself conflict-free
    /// avoids depending on lookup order.
    pub(crate) fn promote_to_prelude(&mut self, name: Symbol, id: DefId) {
        if !self.builtins.contains_key(&name) {
            self.prelude.entry(name).or_insert(id);
        }
    }

    /// A variant name bound by `use module.Enum.Variant;`, visible from
    /// `file` — checks a local `use` first, then the prelude's own,
    /// mirroring [`Self::lookup_in`]'s precedence one level down.
    pub fn lookup_variant_in(&self, file: FileId, name: &Symbol) -> Option<(DefId, u32)> {
        self.variant_imports
            .get(&(file, name.clone()))
            .or_else(|| self.variant_prelude.get(name))
            .copied()
    }

    fn import_variant(&mut self, file: FileId, name: Symbol, target: (DefId, u32)) {
        if !self.by_file_name.contains_key(&(file, name.clone())) {
            self.variant_imports.insert((file, name), target);
        }
    }

    /// Re-exports a variant name (resolved as seen from the prelude file
    /// itself) so every other file can see it with no `use` — mirrors
    /// [`Self::promote_to_prelude`] one level down.
    pub(crate) fn promote_variant_to_prelude(&mut self, name: Symbol, target: (DefId, u32)) {
        if !self.builtins.contains_key(&name) {
            self.variant_prelude.entry(name).or_insert(target);
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (DefId, &Def)> {
        self.defs
            .iter()
            .enumerate()
            .map(|(i, d)| (DefId(i as u32), d))
    }

    /// Registers `name` (a `use` path's last segment) as an opaque,
    /// deferred external name, unless a name is already bound (a local
    /// declaration always wins over an otherwise-unverifiable import).
    pub(crate) fn declare_imported(&mut self, file: FileId, name: Symbol) {
        if self.lookup_in(file, &name).is_none() {
            let id = self.insert(name.clone(), DefKind::Imported);
            self.imports.insert((file, name), id);
        }
    }

    fn import(&mut self, file: FileId, name: Symbol, id: DefId) {
        if !self.by_file_name.contains_key(&(file, name.clone())) {
            self.imports.insert((file, name), id);
        }
    }

    fn insert(&mut self, name: Symbol, kind: DefKind) -> DefId {
        let id = DefId(self.defs.len() as u32);
        self.defs.push(Def {
            name: name.clone(),
            kind,
            variants: Vec::new(),
            methods: Vec::new(),
        });
        self.by_name.insert(name, id);
        id
    }

    fn insert_builtin(&mut self, name: Symbol, kind: DefKind) -> DefId {
        let id = self.insert(name.clone(), kind);
        self.builtins.insert(name, id);
        id
    }

    fn insert_checked(
        &mut self,
        name: &nether_ast::Ident,
        kind: DefKind,
        diags: &mut Vec<Diagnostic>,
    ) -> DefId {
        let key = (name.span.file, name.name.clone());
        if let Some(existing) = self
            .by_file_name
            .get(&key)
            .or_else(|| self.builtins.get(&name.name))
        {
            diags.push(
                Diagnostic::error(format!(
                    "the name `{}` is defined more than once",
                    name.name
                ))
                .with_label(name.span, "redefined here"),
            );
            return *existing;
        }
        let id = DefId(self.defs.len() as u32);
        self.defs.push(Def {
            name: name.name.clone(),
            kind,
            variants: Vec::new(),
            methods: Vec::new(),
        });
        self.by_file_name.insert(key, id);
        self.by_name.entry(name.name.clone()).or_insert(id);
        id
    }
}

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
pub fn collect(
    module: &Module,
    prelude_file: Option<FileId>,
) -> (Definitions, Vec<Diagnostic>) {
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
                    Some(idx) => defs.import_variant(
                        use_decl.span.file,
                        variant_seg.name.clone(),
                        (id, idx),
                    ),
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
            if use_decl.span.file != prelude_file || !module.variant_imports.contains_key(&use_decl.id) {
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
