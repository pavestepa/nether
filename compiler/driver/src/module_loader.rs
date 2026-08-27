use std::collections::HashMap;
use std::path::{Path, PathBuf};

use nether_ast::Module;
use nether_diagnostics::{Diagnostic, SourceMap};

/// The bundled standard library's directory — `stdlib/` at the workspace
/// root, addressed both as `use stdlib.*`/`use std.*` (an external root,
/// not a child any user crate must `mod`-declare) and, unconditionally,
/// as the program-wide prelude loaded by [`load_module_graph`] below.
fn bundled_stdlib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stdlib")
}

/// Every external root a `mod name;`/`use name.Thing;` may address
/// without a local sibling file — the bundled stdlib under both its
/// names, always present, plus every `Nether.toml`-declared local-path
/// dependency (language-spec §20, Stage 6). Generalizes what was, before
/// Stage 6, two hardcoded special cases (`"stdlib"`/`"std"`) into one
/// map both [`ModuleLoader::load`]'s `mod` loop and
/// [`resolve_use_module`] consult identically — a dependency gets
/// exactly the same "external root, own independent `self`/`crate`/
/// `super` namespace" treatment the stdlib already had, just keyed by a
/// manifest-declared name instead of a literal.
fn external_roots(entry_path: &Path) -> HashMap<String, PathBuf> {
    let mut roots = HashMap::new();
    roots.insert("stdlib".to_string(), bundled_stdlib_root());
    roots.insert("std".to_string(), bundled_stdlib_root());
    if let Some(manifest_dir) = find_manifest(entry_path) {
        roots.extend(load_manifest_dependencies(&manifest_dir));
    }
    roots
}

/// Walks up from `entry_path`'s own directory looking for `Nether.toml`,
/// the same "search ancestors" convention Cargo itself uses — a manifest
/// need not sit directly next to the entry file. Absent anywhere is not
/// an error: a manifest-less compile keeps working exactly as it always
/// has (`load_manifest_dependencies`'s own docs).
fn find_manifest(entry_path: &Path) -> Option<PathBuf> {
    let mut dir = entry_path.parent()?;
    loop {
        if dir.join("Nether.toml").is_file() {
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
}

/// Parses `<manifest_dir>/Nether.toml`'s `[dependencies]` table — each
/// entry `name = { path = "..." }`, resolved relative to the manifest's
/// own directory — into `{name: absolute root path}`. No version
/// resolution, no registry, no lockfile: purely local-path dependencies,
/// this stage's own explicit scope. A missing or malformed manifest, or
/// a dependency entry with no `path`, is silently skipped rather than a
/// hard error here — a *missing* dependency root is instead reported
/// where it's actually used, the same "cannot find module" diagnostic
/// [`resolve_use_module`] already produces for any other unresolved
/// module path.
fn load_manifest_dependencies(manifest_dir: &Path) -> HashMap<String, PathBuf> {
    let mut deps = HashMap::new();
    let Ok(source) = std::fs::read_to_string(manifest_dir.join("Nether.toml")) else {
        eprintln!("DEBUG: read_to_string failed");
        return deps;
    };
    let Ok(value) = toml::from_str::<toml::Value>(&source) else {
        return deps;
    };
    let Some(table) = value.get("dependencies").and_then(toml::Value::as_table) else {
        return deps;
    };
    for (name, spec) in table {
        if let Some(path) = spec.get("path").and_then(toml::Value::as_str) {
            deps.insert(name.clone(), manifest_dir.join(path));
        }
    }
    deps
}

/// Loads the entry file and its module graph. `mod user;` declares a
/// child at `user.nr` or `user/mod.nr`; `use` imports a declaration from
/// a loaded relative module. `self`, `super` and `crate` may start a use
/// path. Legacy direct `use user.User` loading remains accepted, and
/// `stdlib.*`/`std.*` resolve from the workspace root — `mod std;` is the
/// same external root under an alternate name
/// (`resolve_use_module`/`load_module_recursive`'s `mod` loop both
/// special-case it).
///
/// `stdlib/mod.nr` (the bundled prelude) is additionally always loaded,
/// independent of whether the entry module graph references it, so that
/// hand-written `impl` blocks on builtin owners such as `Option`
/// (`stdlib/option.nr`) always register regardless of whether any file
/// `use`s them. Its own `FileId` is returned so `resolve_with_prelude`
/// can re-export its top-level `use` names everywhere with no `use` of
/// their own (`nether_resolver::Definitions::promote_to_prelude`).
/// Absent — no bundled `stdlib/mod.nr` on disk — is not an error, just no
/// prelude.
///
/// Loaded module items share one code-generation unit, while resolver
/// namespaces remain separated by `FileId`. NodeIds are unique across the
/// graph and every Span keeps its source file for precise diagnostics.
///
/// Nether source files use exactly the `.nr` extension; the legacy `.nt`
/// extension is rejected with a useful diagnostic rather than silently
/// accepted, both for the entry file (checked here) and for child/`use`
/// modules (checked in [`ModuleLoader::load`] and [`resolve_use_module`]).
pub(super) fn load_module_graph(
    path: &Path,
    source_map: &mut SourceMap,
) -> std::io::Result<(Module, Vec<Diagnostic>, Option<nether_diagnostics::FileId>)> {
    let entry_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut loader = ModuleLoader::new(source_map, external_roots(&entry_path));

    if let Some(diagnostic) = reject_legacy_extension(&entry_path) {
        return Ok((
            Module {
                file: source_map.add_file(entry_path, String::new()),
                items: Vec::new(),
                imports: HashMap::new(),
                variant_imports: HashMap::new(),
            },
            vec![diagnostic],
            None,
        ));
    }

    let entry_file = loader.load(&entry_path, None, None)?;

    let prelude_file = bundled_stdlib_root()
        .join("mod.nr")
        .canonicalize()
        .ok()
        .and_then(|prelude_path| loader.load(&prelude_path, None, None).ok());

    Ok(loader.finish(entry_file, prelude_file))
}

/// Nether source files must use exactly the `.nr` extension. Returns a
/// diagnostic naming the legacy `.nt` extension explicitly when that is
/// what was given, since that is the migration a user is most likely to
/// still be mid-way through.
fn reject_legacy_extension(path: &Path) -> Option<Diagnostic> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("nr") => None,
        Some("nt") => Some(
            Diagnostic::error(format!(
                "`.nt` is no longer a supported Nether source extension: `{}`",
                path.display()
            ))
            .with_hint("rename this file's extension from `.nt` to `.nr`"),
        ),
        _ => Some(
            Diagnostic::error(format!(
                "Nether source files must use the `.nr` extension: `{}`",
                path.display()
            ))
            .with_hint("rename this file to end in `.nr`"),
        ),
    }
}

type FileId = nether_diagnostics::FileId;
type ModuleLocation = (PathBuf, FileId);

struct ModuleLoader<'a> {
    source_map: &'a mut SourceMap,
    visited: HashMap<PathBuf, FileId>,
    items: Vec<nether_ast::Item>,
    diagnostics: Vec<Diagnostic>,
    imports: HashMap<nether_ast::NodeId, FileId>,
    variant_imports: HashMap<nether_ast::NodeId, FileId>,
    next_node_id: u32,
    /// See [`external_roots`]'s own docs.
    external_roots: HashMap<String, PathBuf>,
}

impl<'a> ModuleLoader<'a> {
    fn new(source_map: &'a mut SourceMap, external_roots: HashMap<String, PathBuf>) -> Self {
        Self {
            source_map,
            visited: HashMap::new(),
            items: Vec::new(),
            diagnostics: Vec::new(),
            imports: HashMap::new(),
            variant_imports: HashMap::new(),
            next_node_id: 0,
            external_roots,
        }
    }

    fn finish(
        self,
        entry_file: FileId,
        prelude_file: Option<FileId>,
    ) -> (Module, Vec<Diagnostic>, Option<FileId>) {
        (
            Module {
                file: entry_file,
                items: self.items,
                imports: self.imports,
                variant_imports: self.variant_imports,
            },
            self.diagnostics,
            prelude_file,
        )
    }

    fn load(
        &mut self,
        path: &Path,
        parent: Option<ModuleLocation>,
        crate_root: Option<ModuleLocation>,
    ) -> std::io::Result<FileId> {
        let normalized = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if let Some(&file) = self.visited.get(&normalized) {
            return Ok(file);
        }

        let source = std::fs::read_to_string(&normalized)?;
        let file = self.source_map.add_file(normalized.clone(), source);
        self.visited.insert(normalized.clone(), file);
        let (module, mut parse_diagnostics, next) = nether_parser::parse_module_with_node_id_start(
            self.source_map.source(file),
            file,
            self.next_node_id,
        );
        self.next_node_id = next;
        self.diagnostics.append(&mut parse_diagnostics);
        let crate_root = crate_root.unwrap_or_else(|| (normalized.clone(), file));

        // Declare children before resolving imports, matching Rust's
        // `mod child;` model and making a child's `use super.Name;` point
        // back at this exact file namespace.
        for item in &module.items {
            let nether_ast::Item::Mod(mod_decl) = item else {
                continue;
            };
            // `mod std;` / `mod <dependency-name>;` mounts an external
            // root (the bundled stdlib, or a `Nether.toml`-declared
            // local-path dependency) at its own `mod.nr` — a fresh,
            // independent `crate_root` (`crate_root: None` below), the
            // same treatment `use`-only access already gets in
            // `resolve_use_module`.
            if let Some(root) = self.external_roots.get(mod_decl.name.name.as_str()) {
                let candidate = root.join("mod.nr");
                if candidate.is_file() {
                    self.load(&candidate, None, None)?;
                    continue;
                }
            }
            match resolve_child_module_file(&normalized, &mod_decl.name) {
                Some(module_path) => {
                    self.load(
                        &module_path,
                        Some((normalized.clone(), file)),
                        Some(crate_root.clone()),
                    )?;
                }
                None => self
                    .diagnostics
                    .push(legacy_extension_or_missing_child_diagnostic(
                        &normalized,
                        &mod_decl.name,
                    )),
            }
        }

        for item in &module.items {
            let nether_ast::Item::Use(use_decl) = item else {
                continue;
            };
            if use_decl.path.segments.len() < 2 {
                self.diagnostics.push(
                    Diagnostic::error("a `use` path must contain a module and an imported name")
                        .with_label(use_decl.path.span, "expected `module.Name`"),
                );
                continue;
            }
            self.load_import(use_decl, &normalized, file, parent.as_ref(), &crate_root)?;
        }

        self.items.extend(module.items);
        Ok(file)
    }

    fn load_import(
        &mut self,
        use_decl: &nether_ast::UseDecl,
        importer: &Path,
        importer_file: FileId,
        parent: Option<&ModuleLocation>,
        crate_root: &ModuleLocation,
    ) -> std::io::Result<()> {
        let total = use_decl.path.segments.len();
        let module_segments = &use_decl.path.segments[..total - 1];
        let first_attempt = resolve_use_module(
            importer,
            importer_file,
            parent,
            crate_root,
            module_segments,
            &self.external_roots,
        );
        // A `use module.Enum.Variant;` path (3+ segments) is ambiguous
        // from segment count alone with a genuinely nested module import
        // (`use a.b.C;`) — try the longer, ordinary interpretation
        // (all-but-last segment as the module path) first, since that's
        // strictly more common; only on failure, and only for 3+
        // segments, retry with one fewer trailing segment kept in the
        // module path, treating the trailing *two* segments as
        // (EnumName, VariantName) instead of one plain name.
        // `resolve_use_module` is a pure path-resolution function (no
        // file loads, no diagnostics) — trying it twice can't double-load
        // anything or leak a stray diagnostic from the failed attempt.
        let variant_form = total >= 3 && first_attempt.is_err();
        let (result, module_segments) = if variant_form {
            (
                resolve_use_module(
                    importer,
                    importer_file,
                    parent,
                    crate_root,
                    &use_decl.path.segments[..total - 2],
                    &self.external_roots,
                ),
                &use_decl.path.segments[..total - 2],
            )
        } else {
            (first_attempt, module_segments)
        };
        match result {
            Ok(ModuleTarget::Loaded(target_file)) => {
                self.record_import(use_decl.id, target_file, variant_form);
            }
            Ok(ModuleTarget::File {
                path: module_path,
                parent: target_parent,
            }) => {
                let target_file =
                    self.load(&module_path, target_parent, Some(crate_root.clone()))?;
                self.record_import(use_decl.id, target_file, variant_form);
            }
            Err(message) => self.diagnostics.push(
                Diagnostic::error(format!(
                    "cannot find module `{}`",
                    module_segments
                        .iter()
                        .map(|segment| segment.name.as_str())
                        .collect::<Vec<_>>()
                        .join(".")
                ))
                .with_label(use_decl.path.span, "module not found")
                .with_hint(message),
            ),
        }
        Ok(())
    }

    fn record_import(&mut self, id: nether_ast::NodeId, file: FileId, variant: bool) {
        if variant {
            self.variant_imports.insert(id, file);
        } else {
            self.imports.insert(id, file);
        }
    }
}

enum ModuleTarget {
    Loaded(nether_diagnostics::FileId),
    File {
        path: PathBuf,
        parent: Option<(PathBuf, nether_diagnostics::FileId)>,
    },
}

fn resolve_use_module(
    importer: &Path,
    importer_file: nether_diagnostics::FileId,
    parent: Option<&(PathBuf, nether_diagnostics::FileId)>,
    crate_root: &(PathBuf, nether_diagnostics::FileId),
    segments: &[nether_ast::Ident],
    external_roots: &HashMap<String, PathBuf>,
) -> Result<ModuleTarget, String> {
    let first = segments
        .first()
        .map(|segment| segment.name.as_str())
        .unwrap_or("self");
    let (base_path, base_file, remaining, target_parent) = match first {
        "self" => (
            importer.to_path_buf(),
            importer_file,
            &segments[1..],
            Some((importer.to_path_buf(), importer_file)),
        ),
        "super" => {
            let Some((parent_path, parent_file)) = parent else {
                return Err("the crate root has no parent module".to_string());
            };
            (
                parent_path.clone(),
                *parent_file,
                &segments[1..],
                Some((parent_path.clone(), *parent_file)),
            )
        }
        "crate" => (
            crate_root.0.clone(),
            crate_root.1,
            &segments[1..],
            Some(crate_root.clone()),
        ),
        _ => (
            importer.to_path_buf(),
            importer_file,
            segments,
            Some((importer.to_path_buf(), importer_file)),
        ),
    };

    if remaining.is_empty() {
        return Ok(ModuleTarget::Loaded(base_file));
    }
    if let Some(path) = resolve_module_file(&base_path, remaining) {
        return Ok(ModuleTarget::File {
            path,
            parent: target_parent,
        });
    }

    // An external root (the bundled stdlib under either of its two
    // names, or a `Nether.toml`-declared local-path dependency) rather
    // than a child every user crate must redeclare with `mod name;` —
    // `use name.Thing;` alone is enough, dropping the leading `name`
    // segment itself before resolving under its own root.
    if let Some(root) = external_roots.get(first) {
        if let Some(path) = resolve_module_file_from_root(root, &segments[1..]) {
            return Ok(ModuleTarget::File { path, parent: None });
        }
    }
    Err("declare local children with `mod name;` and check the module file path".to_string())
}

fn resolve_child_module_file(declaring_module: &Path, name: &nether_ast::Ident) -> Option<PathBuf> {
    let segment = std::slice::from_ref(name);
    resolve_module_file(declaring_module, segment)
}

/// The roots a module path is tried against, in order: a directory named
/// after the importer's own file stem (so `user.nr`'s children live under
/// `user/`), then the importer's parent directory directly. Shared between
/// the real `.nr` lookup and the legacy-`.nt`-sibling check used only to
/// improve the "module not found" diagnostic.
fn module_search_roots(importer: &Path) -> Option<Vec<PathBuf>> {
    let parent = importer.parent()?;
    let stem = importer.file_stem().and_then(|stem| stem.to_str())?;
    let mut roots = Vec::new();
    if !matches!(stem, "main" | "lib" | "mod") {
        roots.push(parent.join(stem));
    }
    roots.push(parent.to_path_buf());
    Some(roots)
}

fn resolve_module_file(importer: &Path, segments: &[nether_ast::Ident]) -> Option<PathBuf> {
    for root in module_search_roots(importer)? {
        if let Some(path) = resolve_module_file_from_root(&root, segments) {
            return Some(path);
        }
    }
    None
}

fn resolve_module_file_from_root(root: &Path, segments: &[nether_ast::Ident]) -> Option<PathBuf> {
    find_file_with_extension(root, segments, "nr")
}

/// Looks for a `.nt` file where a `.nr` module was expected, purely to
/// produce a more specific "rename this" diagnostic than a generic
/// "module not found" — never used to actually load a module.
fn resolve_legacy_nt_sibling(importer: &Path, segments: &[nether_ast::Ident]) -> Option<PathBuf> {
    for root in module_search_roots(importer)? {
        if let Some(path) = find_file_with_extension(&root, segments, "nt") {
            return Some(path);
        }
    }
    None
}

fn find_file_with_extension(
    root: &Path,
    segments: &[nether_ast::Ident],
    extension: &str,
) -> Option<PathBuf> {
    let mut base = root.to_path_buf();
    for segment in segments {
        base.push(segment.name.as_str());
    }
    let flat = base.with_extension(extension);
    if flat.is_file() {
        return Some(flat);
    }
    let nested = base.join(format!("mod.{extension}"));
    if nested.is_file() {
        return Some(nested);
    }
    None
}

/// Builds the diagnostic for a `mod name;` declaration whose child file
/// couldn't be found, special-casing the common case of an un-migrated
/// `.nt` sibling still sitting on disk.
fn legacy_extension_or_missing_child_diagnostic(
    declaring_module: &Path,
    name: &nether_ast::Ident,
) -> Diagnostic {
    let segment = std::slice::from_ref(name);
    if let Some(legacy_path) = resolve_legacy_nt_sibling(declaring_module, segment) {
        return Diagnostic::error(format!(
            "`.nt` is no longer a supported Nether source extension: `{}`",
            legacy_path.display()
        ))
        .with_label(name.span, "found only a `.nt` file for this module")
        .with_hint(format!(
            "rename `{}` to end in `.nr`",
            legacy_path.display()
        ));
    }
    Diagnostic::error(format!("cannot find child module `{}`", name.name))
        .with_label(
            name.span,
            "expected a sibling module file or module directory",
        )
        .with_hint(format!(
            "create `{}.nr` or `{}/mod.nr` next to this module",
            name.name, name.name
        ))
}

#[cfg(test)]
mod manifest_tests {
    use super::*;

    #[test]
    fn finds_and_parses_a_path_dependency() {
        let dir = std::env::temp_dir().join("nether_manifest_unit_test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("Nether.toml"),
            r#"
[package]
name = "app"

[dependencies]
mathlib = { path = "../mathlib" }
"#,
        )
        .unwrap();
        assert_eq!(find_manifest(&dir.join("main.nr")), Some(dir.clone()));
        let deps = load_manifest_dependencies(&dir);
        assert_eq!(deps.get("mathlib"), Some(&dir.join("../mathlib")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_manifest_found_returns_none() {
        let dir = std::env::temp_dir().join("nether_manifest_unit_test_absent");
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(load_manifest_dependencies(&dir).len(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
