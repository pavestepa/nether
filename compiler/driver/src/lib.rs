//! # nether_driver
//!
//! Purpose: orchestrate the compiler pipeline end to end for a set of
//! source files (see `docs/architecture/crates.md` § `compiler/driver`).
//!
//! The implemented pipeline is lexer → parser → resolver → typecheck →
//! HIR → monomorphization → MIR/ARC → codegen → object emission and,
//! for the host target, linking. Per
//! `docs/architecture/overview.md` §2, a stage never runs on input the
//! previous stage already flagged as invalid: resolution is skipped when
//! parsing produced an error, type checking is skipped when resolution
//! produced one, and HIR lowering is skipped when type checking produced
//! one (lowering itself cannot fail — `nether_hir`'s own invariant — so
//! it runs unconditionally once type checking succeeds). Monomorphization
//! (and, transitively, MIR/codegen) needs a program entry point: it only
//! runs when the module defines a `main` function, since a file with no
//! `main` (a library-style module of just `type`/`fn` declarations) has no
//! root to walk the reachable call
//! graph from — this is not an error, just nothing to emit.
//! `codegen`'s own invariant is that a failed LLVM module verification is
//! an internal compiler error, not a user diagnostic (`overview.md` §2's
//! front-end/back-end split) — [`check`] `panic!`s rather than reporting
//! one through `diagnostics` for exactly that reason. Linking (the last
//! pipeline stage, per `crates.md`'s `compiler/driver` section) is wired
//! up too — see [`link`]'s own module docs for why it can gracefully
//! produce no executable (`runtime/*` not yet built locally) without that
//! being an error.
//!
//! Dependencies: `nether_diagnostics`, `nether_ast`, `nether_lexer`
//! (transitively, via `nether_parser`), `nether_parser`, `nether_resolver`,
//! `nether_typecheck`, `nether_hir`, `nether_monomorphization`, `nether_mir`,
//! `nether_llvm`, `nether_codegen`.
//!
//! Consumers: `cli`.

mod link;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use nether_ast::{Module, Symbol};
use nether_diagnostics::{Diagnostic, SourceMap};
use nether_hir::HirModule;
use nether_mir::MirFunction;
use nether_monomorphization::MonoModule;
use nether_resolver::ResolvedNames;

/// Everything produced by running the pipeline over one entry module and
/// its transitive imports.
pub struct CheckResult {
    pub module: Module,
    /// `None` when parsing produced an error diagnostic — resolution
    /// assumes syntactically well-formed input and is skipped rather than
    /// run against a partial/malformed tree.
    pub resolved: Option<ResolvedNames>,
    /// `None` when parsing or resolution produced an error diagnostic —
    /// type checking assumes every name is already resolved.
    pub hir: Option<HirModule>,
    /// `None` when lowering didn't run, or when it ran but the module
    /// defines no `main` (see this crate's module docs).
    pub mono: Option<MonoModule>,
    /// `None` under the same conditions as `mono` — built from it in
    /// lockstep, with ARC already inserted (`nether_mir::insert_arc`
    /// always runs immediately after `nether_mir::build_mir`, per that
    /// crate's own invariant that `codegen` never sees MIR without it).
    pub mir: Option<Vec<MirFunction>>,
    /// The emitted object file's path, under the same conditions as
    /// `mir` — the LLVM module itself isn't kept around (its `inkwell`
    /// types borrow from a `Context` this function creates and drops
    /// internally; only the finished `.o` on disk outlives `check`).
    pub object_path: Option<PathBuf>,
    /// The linked native executable's path — `None` under the same
    /// conditions as `object_path`, or if `runtime/*`'s static libraries
    /// haven't been built locally yet (see [`link`]'s own module docs;
    /// not an error).
    pub executable_path: Option<PathBuf>,
    pub source_map: SourceMap,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone)]
pub struct CompileOptions {
    pub target_triple: String,
    pub opt_level: u8,
    /// Final executable path. The intermediate object is written next to
    /// it with an `.o` extension. `None` keeps the historical
    /// `<source>.o` / `<source-without-extension>` defaults.
    pub output_path: Option<PathBuf>,
    pub link: bool,
}

impl Default for CompileOptions {
    fn default() -> Self {
        CompileOptions {
            target_triple: nether_llvm::Codegen::host_triple(),
            opt_level: 0,
            output_path: None,
            link: true,
        }
    }
}

/// Loads `path` and runs parse → resolve → typecheck → lower →
/// monomorphize → build MIR → insert ARC → codegen → emit object → link,
/// stopping at the first stage that reports an error.
///
/// # Errors
/// Returns `Err` if `path` cannot be read from disk, the object file
/// cannot be written, or `cc` cannot run/fails while linking. Missing
/// prebuilt `runtime/*` static libraries simply leaves
/// `executable_path == None` (see [`link`]'s own module docs).
/// Parse/resolve/typecheck errors are reported through
/// `diagnostics`, not this `Result`, since every front-end stage here is
/// total (each crate's own invariants).
///
/// # Panics
/// Panics if the LLVM module `nether_codegen` builds fails to verify —
/// per this crate's own module docs, that is always an internal compiler
/// bug, never a user-facing diagnostic.
pub fn check(path: &Path) -> Result<CheckResult, std::io::Error> {
    compile(path, &CompileOptions::default())
}

/// Compiles a source module graph using explicit target/output options.
/// Front-end errors remain in `CheckResult::diagnostics`; invalid backend
/// configuration and filesystem/toolchain failures use the outer
/// `io::Result`.
pub fn compile(path: &Path, options: &CompileOptions) -> Result<CheckResult, std::io::Error> {
    let mut source_map = SourceMap::new();
    let (module, mut diagnostics, prelude_file) = load_module_graph(path, &mut source_map)?;
    let mut resolved = None;
    let mut hir = None;
    let mut mono = None;
    let mut mir = None;
    let mut object_path = None;
    let mut executable_path = None;

    if !diagnostics.iter().any(Diagnostic::is_error) {
        let (r, resolve_diagnostics) = nether_resolver::resolve_with_prelude(&module, prelude_file);
        diagnostics.extend(resolve_diagnostics);
        if !diagnostics.iter().any(Diagnostic::is_error) {
            let (tables, check_diagnostics) = nether_typecheck::check(&module, &r);
            diagnostics.extend(check_diagnostics);
            if !diagnostics.iter().any(Diagnostic::is_error) {
                let lowered = nether_hir::lower(&module, &r, tables);
                let entry_main = r
                    .definitions
                    .lookup_in(module.file, &Symbol::new("main"))
                    .and_then(|def| lowered.fn_by_def.get(&def))
                    .copied();
                if let Some(main_id) = entry_main {
                    let m = nether_monomorphization::monomorphize(&lowered, main_id);
                    let mut functions =
                        nether_mir::build_mir(&m, &r.definitions, &lowered.signatures);
                    nether_mir::insert_arc(&mut functions);

                    let cg = nether_llvm::Codegen::with_target(
                        &options.target_triple,
                        options.opt_level,
                    )
                    .map_err(|error| {
                        std::io::Error::new(std::io::ErrorKind::InvalidInput, error)
                    })?;
                    let module_name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("nether_module");
                    let llvm_module = nether_codegen::generate(
                        &cg,
                        module_name,
                        &functions,
                        &r.definitions,
                        &lowered.signatures,
                    );
                    llvm_module.verify().unwrap_or_else(|e| {
                        panic!(
                            "nether_codegen produced an invalid LLVM module:\n{}\n\nerror: {e}",
                            llvm_module.print_to_string()
                        )
                    });
                    let out = options
                        .output_path
                        .as_ref()
                        .map(|output| output.with_extension("o"))
                        .unwrap_or_else(|| path.with_extension("o"));
                    llvm_module.emit_object(&out)?;
                    let executable = options
                        .output_path
                        .clone()
                        .unwrap_or_else(|| path.with_extension(""));
                    if options.link && options.target_triple == nether_llvm::Codegen::host_triple()
                    {
                        executable_path = link::link(&out, &executable)?;
                    }
                    object_path = Some(out);

                    mir = Some(functions);
                    mono = Some(m);
                }
                hir = Some(lowered);
            }
        }
        resolved = Some(r);
    }

    Ok(CheckResult {
        module,
        resolved,
        hir,
        mono,
        mir,
        object_path,
        executable_path,
        source_map,
        diagnostics,
    })
}

/// The bundled standard library's directory — `stdlib/` at the workspace
/// root, addressed both as `use stdlib.*`/`use std.*` (an external root,
/// not a child any user crate must `mod`-declare) and, unconditionally,
/// as the program-wide prelude loaded by [`load_module_graph`] below.
fn bundled_stdlib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stdlib")
}

/// Loads the entry file and its module graph. `mod user;` declares a
/// child at `user.nt`/`user.nr` or `user/mod.nt`/`user/mod.nr`; `use`
/// imports a declaration from a loaded relative module. `self`, `super`
/// and `crate` may start a use path. Legacy direct `use user.User`
/// loading remains accepted, and `stdlib.*`/`std.*` resolve from the
/// workspace root — `mod std;` is the same external root under an
/// alternate name (`resolve_use_module`/`load_module_recursive`'s `mod`
/// loop both special-case it).
///
/// `stdlib/mod.nt` (the bundled prelude) is additionally always loaded,
/// independent of whether the entry module graph references it, so that
/// hand-written `impl` blocks on builtin owners such as `Option`
/// (`stdlib/option.nt`) always register regardless of whether any file
/// `use`s them. Its own `FileId` is returned so `resolve_with_prelude`
/// can re-export its top-level `use` names everywhere with no `use` of
/// their own (`nether_resolver::Definitions::promote_to_prelude`).
/// Absent — no bundled `stdlib/mod.nt` on disk — is not an error, just no
/// prelude.
///
/// Loaded module items share one code-generation unit, while resolver
/// namespaces remain separated by `FileId`. NodeIds are unique across the
/// graph and every Span keeps its source file for precise diagnostics.
fn load_module_graph(
    path: &Path,
    source_map: &mut SourceMap,
) -> std::io::Result<(
    Module,
    Vec<Diagnostic>,
    Option<nether_diagnostics::FileId>,
)> {
    let mut visited = HashMap::new();
    let mut items = Vec::new();
    let mut diagnostics = Vec::new();
    let mut imports = HashMap::new();
    let mut variant_imports = HashMap::new();
    let mut next_node_id = 0;
    let entry_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let entry_file = load_module_recursive(
        &entry_path,
        source_map,
        &mut visited,
        &mut items,
        &mut diagnostics,
        &mut imports,
        &mut variant_imports,
        &mut next_node_id,
        None,
        None,
    )?;

    let prelude_file = bundled_stdlib_root()
        .join("mod.nt")
        .canonicalize()
        .ok()
        .and_then(|prelude_path| {
            load_module_recursive(
                &prelude_path,
                source_map,
                &mut visited,
                &mut items,
                &mut diagnostics,
                &mut imports,
                &mut variant_imports,
                &mut next_node_id,
                None,
                None,
            )
            .ok()
        });

    Ok((
        Module {
            file: entry_file,
            items,
            imports,
            variant_imports,
        },
        diagnostics,
        prelude_file,
    ))
}

fn load_module_recursive(
    path: &Path,
    source_map: &mut SourceMap,
    visited: &mut HashMap<PathBuf, nether_diagnostics::FileId>,
    all_items: &mut Vec<nether_ast::Item>,
    diagnostics: &mut Vec<Diagnostic>,
    imports: &mut HashMap<nether_ast::NodeId, nether_diagnostics::FileId>,
    variant_imports: &mut HashMap<nether_ast::NodeId, nether_diagnostics::FileId>,
    next_node_id: &mut u32,
    parent: Option<(PathBuf, nether_diagnostics::FileId)>,
    crate_root: Option<(PathBuf, nether_diagnostics::FileId)>,
) -> std::io::Result<nether_diagnostics::FileId> {
    let normalized = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if let Some(&file) = visited.get(&normalized) {
        return Ok(file);
    }
    let source = std::fs::read_to_string(&normalized)?;
    let file = source_map.add_file(normalized.clone(), source.clone());
    visited.insert(normalized.clone(), file);
    let (module, mut parse_diagnostics, next) =
        nether_parser::parse_module_with_node_id_start(&source, file, *next_node_id);
    *next_node_id = next;
    diagnostics.append(&mut parse_diagnostics);

    let crate_root = crate_root.unwrap_or_else(|| (normalized.clone(), file));

    // Declare children before resolving imports, matching Rust's
    // `mod child;` model and making a child's `use super.Name;` point
    // back at this exact file namespace.
    for item in &module.items {
        let nether_ast::Item::Mod(mod_decl) = item else {
            continue;
        };
        // `mod std;` is an alias for the bundled `stdlib/` root rather
        // than an ordinary sibling child — same external-root treatment
        // as the `use stdlib.*`/`use std.*` fallback in
        // `resolve_use_module` below, so it gets no `parent`/inherited
        // `crate_root` of its declaring file.
        if mod_decl.name.name.as_str() == "std" {
            let candidate = bundled_stdlib_root().join("mod.nt");
            if candidate.is_file() {
                load_module_recursive(
                    &candidate,
                    source_map,
                    visited,
                    all_items,
                    diagnostics,
                    imports,
                    variant_imports,
                    next_node_id,
                    None,
                    None,
                )?;
                continue;
            }
        }
        match resolve_child_module_file(&normalized, &mod_decl.name) {
            Some(module_path) => {
                load_module_recursive(
                    &module_path,
                    source_map,
                    visited,
                    all_items,
                    diagnostics,
                    imports,
                    variant_imports,
                    next_node_id,
                    Some((normalized.clone(), file)),
                    Some(crate_root.clone()),
                )?;
            }
            None => diagnostics.push(
                Diagnostic::error(format!("cannot find child module `{}`", mod_decl.name.name))
                    .with_label(
                        mod_decl.name.span,
                        "expected a sibling module file or module directory",
                    )
                    .with_hint(format!(
                        "create `{}.nt` or `{}/mod.nt` next to this module",
                        mod_decl.name.name, mod_decl.name.name
                    )),
            ),
        }
    }

    for item in &module.items {
        let nether_ast::Item::Use(use_decl) = item else {
            continue;
        };
        if use_decl.path.segments.len() < 2 {
            diagnostics.push(
                Diagnostic::error("a `use` path must contain a module and an imported name")
                    .with_label(use_decl.path.span, "expected `module.Name`"),
            );
            continue;
        }
        let total = use_decl.path.segments.len();
        let module_segments = &use_decl.path.segments[..total - 1];
        let first_attempt = resolve_use_module(
            &normalized,
            file,
            parent.as_ref(),
            &crate_root,
            module_segments,
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
                    &normalized,
                    file,
                    parent.as_ref(),
                    &crate_root,
                    &use_decl.path.segments[..total - 2],
                ),
                &use_decl.path.segments[..total - 2],
            )
        } else {
            (first_attempt, module_segments)
        };
        match result {
            Ok(ModuleTarget::Loaded(target_file)) => {
                if variant_form {
                    variant_imports.insert(use_decl.id, target_file);
                } else {
                    imports.insert(use_decl.id, target_file);
                }
            }
            Ok(ModuleTarget::File {
                path: module_path,
                parent: target_parent,
            }) => {
                let target_file = load_module_recursive(
                    &module_path,
                    source_map,
                    visited,
                    all_items,
                    diagnostics,
                    imports,
                    variant_imports,
                    next_node_id,
                    target_parent,
                    Some(crate_root.clone()),
                )?;
                if variant_form {
                    variant_imports.insert(use_decl.id, target_file);
                } else {
                    imports.insert(use_decl.id, target_file);
                }
            }
            Err(message) => diagnostics.push(
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
    }
    all_items.extend(module.items);
    Ok(file)
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

    // Bundled modules are an external root rather than children that each
    // user crate must redeclare with `mod stdlib;`.
    if first == "stdlib" {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        if let Some(path) = resolve_module_file_from_root(&workspace, segments) {
            return Ok(ModuleTarget::File { path, parent: None });
        }
    }
    // `std` is an alias for the same bundled root under the name a
    // `mod std;` declaration mounts it as (`std.option.Option` reaches
    // the same file as `stdlib.option.Option`) — dropping the leading
    // `std` segment itself before resolving under `stdlib/`.
    if first == "std" {
        if let Some(path) = resolve_module_file_from_root(&bundled_stdlib_root(), &segments[1..]) {
            return Ok(ModuleTarget::File { path, parent: None });
        }
    }
    Err("declare local children with `mod name;` and check the module file path".to_string())
}

fn resolve_child_module_file(declaring_module: &Path, name: &nether_ast::Ident) -> Option<PathBuf> {
    let segment = std::slice::from_ref(name);
    resolve_module_file(declaring_module, segment)
}

fn resolve_module_file(importer: &Path, segments: &[nether_ast::Ident]) -> Option<PathBuf> {
    let parent = importer.parent()?;
    let stem = importer.file_stem().and_then(|stem| stem.to_str())?;
    let mut roots = Vec::new();
    if !matches!(stem, "main" | "lib" | "mod") {
        roots.push(parent.join(stem));
    }
    roots.push(parent.to_path_buf());
    for root in roots {
        if let Some(path) = resolve_module_file_from_root(&root, segments) {
            return Some(path);
        }
    }
    None
}

fn resolve_module_file_from_root(root: &Path, segments: &[nether_ast::Ident]) -> Option<PathBuf> {
    let mut extensions = Vec::new();
    // Prefer the source extension used by the path's surrounding tree,
    // then accept both Nether extensions.
    if let Some(ext) = root.extension().and_then(|ext| ext.to_str()) {
        extensions.push(ext);
    }
    extensions.extend(["nt", "nr"]);
    extensions.sort();
    extensions.dedup();
    let mut base = root.to_path_buf();
    for segment in segments {
        base.push(segment.name.as_str());
    }
    for extension in &extensions {
        let flat = base.with_extension(extension);
        if flat.is_file() {
            return Some(flat);
        }
        let nested = base.join(format!("mod.{extension}"));
        if nested.is_file() {
            return Some(nested);
        }
    }
    None
}
