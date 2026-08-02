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
mod module_loader;

use std::path::{Path, PathBuf};

use nether_ast::{Module, Symbol};
use nether_diagnostics::{Diagnostic, SourceMap};
use nether_hir::HirModule;
use nether_mir::MirFunction;
use nether_monomorphization::MonoModule;
use nether_resolver::ResolvedNames;

use module_loader::load_module_graph;

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
