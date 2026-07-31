//! Invokes the system linker to turn the object file `nether_codegen`
//! emits into a native executable, against the real `runtime/*` static
//! libraries (`docs/architecture/crates.md` § `compiler/driver`'s own
//! "linker invocation" responsibility).
//!
//! `nether_codegen` already emits a C-ABI `main` calling the program's own
//! `nether_main` (see its own module docs), so this module's only job is
//! to invoke `cc` — no separate hand-written entry stub is needed.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The `runtime/*` static libraries every emitted object needs, in link
/// order (a library must come *after* whatever references its symbols —
/// `io` calls into `string`, `string`/`array` call into `arc`).
const RUNTIME_LIBS: [&str; 4] = [
    "nether_rt_io",
    "nether_rt_string",
    "nether_rt_array",
    "nether_rt_arc",
];

/// This crate's own workspace root, computed from its compiled-in
/// manifest directory (`compiler/driver`) rather than the current working
/// directory at run time, which callers of `nether_driver` (like `cli`)
/// have no reason to guarantee is the workspace root.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The prebuilt `runtime/*` static library paths, preferring a `release`
/// build over `debug` if both exist, or `None` if they haven't been built
/// at all yet.
///
/// `runtime/*` is deliberately not a Cargo dependency of `nether_driver`
/// (those crates are meant to be *linked into* a compiled Nether program,
/// never called into from inside the compiler itself), so nothing builds
/// them automatically as a side effect of building `nether_driver` — run
/// `cargo build --release -p nether-rt-arc -p nether-rt-string -p nether-rt-array -p nether-rt-io`
/// once first.
fn locate_runtime_libs() -> Option<Vec<PathBuf>> {
    let root = workspace_root();
    for profile in ["release", "debug"] {
        let dir = root.join("target").join(profile);
        let paths: Vec<PathBuf> = RUNTIME_LIBS
            .iter()
            .map(|name| dir.join(format!("lib{name}.a")))
            .collect();
        if paths.iter().all(|p| p.is_file()) {
            return Some(paths);
        }
    }
    None
}

/// Links `object` into a native executable at `out`, against the
/// prebuilt `runtime/*` static libraries. Returns `Ok(None)` — not an
/// error — if those haven't been built yet (see [`locate_runtime_libs`]):
/// this mirrors [`crate::check`]'s own "nothing further to do yet" style
/// for a `main`-less module, since a missing, buildable-on-demand runtime
/// isn't a defect in the source file being compiled.
///
/// # Errors
/// Returns `Err` if `cc` itself can't be spawned, or exits unsuccessfully
/// (a real linker error — mismatched symbols, an unsupported target,
/// etc. — always unexpected at this stage, since every symbol the object
/// file leaves undefined is one of the four `runtime/*` libraries' own,
/// per `nether_codegen`'s `runtime.rs`).
pub fn link(object: &Path, out: &Path) -> std::io::Result<Option<PathBuf>> {
    let Some(libs) = locate_runtime_libs() else {
        return Ok(None);
    };
    let status = Command::new("cc")
        .arg(object)
        .args(&libs)
        .arg("-o")
        .arg(out)
        .status()?;
    if !status.success() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("cc exited with {status} linking {}", object.display()),
        ));
    }
    Ok(Some(out.to_path_buf()))
}
