use std::collections::HashMap;

use nether_ast::{Item, Module, Symbol, TypeExpr};
use nether_diagnostics::{Diagnostic, FileId};

use crate::resolve::variant_index;

mod collect;

pub use collect::collect;

/// Identifies one top-level definition (a primitive, a `type`, an `enum`,
/// an `trait`, or a `fn`) for the lifetime of one [`resolve`](crate::resolve)
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
    /// A struct, tuple-struct, or unit `struct` declaration, or a
    /// built-in heap type (`String`, `Array`).
    Type,
    /// A `type Name = TypeExpr;` alias declaration (language-spec §4.3).
    /// Transparent at the type level — `typecheck` substitutes through to
    /// the alias's underlying [`TypeExpr`] rather than this ever becoming
    /// its own `Type` variant.
    TypeAlias,
    /// An `enum` declaration — including `Option`/`Result`, ordinary
    /// generic enums declared in the bundled prelude, not builtins.
    Enum,
    Trait,
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
/// built-in, and user-declared `type`/`enum`/`trait`/`fn`, keyed by
/// name.
#[derive(Default)]
pub struct Definitions {
    defs: Vec<Def>,
    by_name: HashMap<Symbol, DefId>,
    builtins: HashMap<Symbol, DefId>,
    by_file_name: HashMap<(FileId, Symbol), DefId>,
    imports: HashMap<(FileId, Symbol), DefId>,
    /// Names re-exported by the bundled prelude file (`stdlib/mod.nr`'s
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
