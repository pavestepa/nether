# Nether Compiler — Crate Reference

This document describes the implemented workspace. For the pipeline, see
[`overview.md`](./overview.md); for semantic types and ARC rules, see
[`type-system.md`](./type-system.md) and [`arc-model.md`](./arc-model.md).

## Workspace

```text
compiler/
  ast/ diagnostics/ lexer/ parser/ resolver/ typecheck/
  hir/ monomorphization/ mir/ llvm/ codegen/ driver/
runtime/
  arc/ string/ array/ io/ task/ thread/
stdlib/
  mod.nr option.nr result.nr array.nr
cli/
examples/
```

## Internal source layout

Large passes are split by responsibility while keeping their public crate
API unchanged:

- parser expressions are separated into postfix/core, atoms, control flow,
  and precedence helpers;
- resolver separates declaration collection, expression traversal, path
  resolution, and diagnostic helpers;
- typecheck separates declaration signatures, trait assembly, layout
  validation, expression/control/call checking, and generic inference;
- HIR, monomorphization, and MIR separate core context from expression,
  call, pattern, closure, and control-flow lowering;
- LLVM and codegen separate memory access, arithmetic, runtime calls,
  aggregate construction, terminators, and object emission;
- driver module-graph loading owns its traversal state in a dedicated
  loader instead of passing a wide mutable argument list recursively.

Compiler and test Rust files are kept below 500 lines. Integration suites
use small child modules while sharing setup helpers from their root test
crate. These are compile-time module boundaries only and add no runtime
dispatch.

## `compiler/ast`

Defines immutable source-shaped nodes for items, expressions, statements,
patterns and type syntax. Nodes carry `NodeId` and `Span`; semantic results
live in resolver/type-checker side tables. A combined module also records
the exact target `FileId` for each `use` declaration.

Dependencies: `diagnostics`. Consumers: parser and all front-end stages.

## `compiler/diagnostics`

Provides `FileId`, `SourceMap`, `Span`, structured diagnostics, labels,
hints, suggestions and source rendering. Diagnostic construction is
filesystem-independent; only the driver owns source loading.

## `compiler/lexer`

Turns UTF-8 source into spanned tokens. It handles comments, escapes,
numeric/boolean/character/string literals and template-string
interpolation. Invalid input produces diagnostics and recovery tokens
instead of aborting the process.

## `compiler/parser`

Uses recursive descent for declarations/statements/patterns and Pratt
parsing for expressions. It represents closures, generics, traits,
enums, `match`, loops, weak types and `mod`/`use` syntax without attempting
name or type resolution.

Public entry point:

```rust
pub fn parse_module(source: &str, file: FileId)
    -> (ast::Module, Vec<Diagnostic>);
```

## `compiler/resolver`

Builds built-in and per-file namespaces, lexical local scopes and
`NodeId -> Resolution` side tables. The driver first follows `mod`
declarations, loads the transitive file graph, and records every `use`
edge. Imports through `self`, `super`, and `crate` therefore resolve to an
exact declaration in the target file. Equal private top-level names in
different files do not collide.

The resolver also disambiguates:

- bindings from unqualified unit enum variants in patterns;
- static functions/constructors from instance member paths;
- declaration and function generic parameters.

Current module limits are file modules only, explicit single-name imports,
and no inline modules, aliases, globs, re-exports or package manager.

## `compiler/typecheck`

Collects declaration signatures and assigns a structured `Type` to every
expression/local needed by HIR. It checks:

- calls, returns, assignments and mutable-argument rules;
- structs, tuples, arrays, enums, patterns and match exhaustiveness;
- weak references and implicit weak reads as `Option<T>`;
- inferred and explicit call-site generic arguments, declaration bounds
  and generic trait arguments;
- trait inheritance, implementation completeness, default conflicts
  and method signatures;
- `Into<String>` for interpolation and variadic print calls;
- finite value layouts, legal entry-point signatures and loop control.

Front-end-invalid programs stop here. In particular, mismatched pattern
kind/arity/enum and recursively infinite stack layouts are diagnostics,
not late MIR/codegen failures.

Current generic limits: no blanket implementations, static/trait
specialization, higher-kinded types, or first-class unspecialized generic
function values. Associated items, const generics, multiple bounds, `where`
clauses, and concrete instance-method overrides of an explicit `default impl`
are supported.

## `compiler/hir`

Lowers a successful typed AST to a smaller typed representation. It:

- resolves calls to declaration IDs;
- desugars method calls, interpolation, weak upgrades and `for`;
- performs closure capture analysis and closure conversion;
- copies declaration-opted-in trait default bodies, including
  inherited and specialized defaults, into concrete implementations;
- preserves generics and resolved call-site specializations for the
  monomorphization pass.

Capturing closures carry a generated environment; non-capturing closures
and named function values use the same callable ABI.

## `compiler/monomorphization`

Walks reachable code from the entry `main`, memoizes each concrete
function/method instantiation and substitutes generic types throughout
HIR. Generic methods infer substitutions from both their receiver type and
ordinary arguments. The resulting `MonoModule` contains no unresolved
generic calls.

## `compiler/mir`

Lowers monomorphic HIR to explicit basic blocks and terminators, then
inserts ownership operations. MIR supports:

- arbitrary nested field/index places;
- direct and indirect closure calls;
- enum construction, discriminants and payload projections;
- weak upgrades and array primitives;
- retain/release and weak-retain/weak-release instructions.

`insert_arc` is mandatory before codegen. It handles scope exits, mutation,
temporaries, call arguments, returned values, managed aggregate contents
and mutable parameters.

## `compiler/llvm`

The only compiler crate that imports Inkwell/LLVM. `Codegen` creates a
target machine for an LLVM triple and `O0`–`O3`; `ModuleCx` is the
instruction-building facade used by codegen. It verifies, optimizes and
emits object files.

```rust
pub fn Codegen::with_target(
    triple: &str,
    opt_level: u8,
) -> Result<Codegen, String>;
```

## `compiler/codegen`

Translates ARC-annotated MIR into LLVM IR. It owns:

- primitive, struct, tuple, generic-instantiation and enum layouts;
- calls to the runtime C ABI;
- generated deep retain/drop and array element callback shims;
- weak-storage operations and `Option<T>` construction on upgrade;
- closure code-pointer/environment representation and indirect calls;
- the platform C-ABI `main` wrapper.

Enums currently use a simple flat tagged layout instead of an overlapping
union. This is correct but can waste space.

## `compiler/driver`

Loads an entry file, follows Rust-style `mod child;` declarations and
relative `use self`/`super`/`crate` imports, gives every file a namespace,
loads bundled stdlib roots, sequences all compiler stages, emits an object
and optionally links a host executable.

```rust
pub struct CompileOptions {
    pub target_triple: String,
    pub opt_level: u8,
    pub output_path: Option<PathBuf>,
    pub link: bool,
    pub emit_llvm_path: Option<PathBuf>,
}

pub fn compile(
    entry_file: &Path,
    options: &CompileOptions,
) -> io::Result<CheckResult>;
```

The driver never runs a semantic stage after an earlier error diagnostic.
Cross-target object emission is supported; linking is intentionally
limited to the host target. Runtime static libraries must already have
been built for linking. An optional `Nether.toml` next to (or in an
ancestor of) the entry file declares local-path dependencies
(`module_loader.rs`'s `external_roots`) — no version resolution, no
registry, purely local paths; absent is not an error.

## `runtime/arc`

Implements the strong/weak reference-counted allocation header and C
ABI, atomic since Stage 6 (`AtomicI64` counts, mirroring
`std::sync::Arc`/`Weak`'s own dealloc design — see `arc-model.md`):

- allocation with an optional generated payload-drop callback;
- strong retain/release;
- weak retain/release;
- upgrade that retains only while the strong object is alive.

The payload is destroyed when the strong count reaches zero; the header
remains until the last weak observer is released.

## `runtime/task`

The `task.spawn`/`await` runtime bridge (Stage 4): a `current_thread`
Tokio runtime, a `TaskHeader` ABI convention (poll fn pointer, output-drop
fn pointer, output) every `async fn`'s compiler-generated frame follows,
and `timer.sleep`. Deliberately single-threaded — see `roadmap.md`'s
Stage 4 sequencing note on why `SpawnedTask`'s `unsafe impl Send` stays
sound without needing this crate to change when `runtime/thread` (below)
was added.

## `runtime/thread`

`thread.spawn`/`.join()`'s runtime half (Stage 6): plain
`std::thread::spawn`, no Tokio dependency, architecturally independent of
`runtime/task`. A spawned closure's own code pointer/environment reuse
the same ABI `nether_codegen`'s ordinary dynamic-closure-call path
already produces; v1 requires a `()`-returning closure, so no
result-boxing is needed.

## `runtime/string`

Stores valid UTF-8 strings in ARC allocations and implements creation,
concatenation, primitive conversion and read-only byte/length accessors.
The I/O runtime uses those accessors rather than depending on the internal
layout.

## `runtime/array`

Implements growable contiguous storage through a type-erased C ABI.
Codegen passes element size plus generated retain/drop callbacks, allowing
arrays of primitives, heap values and managed stack aggregates to share
one runtime implementation. `pop` transfers element ownership to the
caller.

## `runtime/io`

Implements the `print` and `println` built-ins for Nether `String` values.

## `stdlib`

`Option`, `Result`, and `Array` are ordinary generic Nether declarations in
`stdlib/option.nr`, `stdlib/result.nr`, and `stdlib/array.nr`. Their methods
compile through the same module graph as user code; there are no native
option/result runtime crates.

## `cli`

The `nether` binary exposes `check`, `build`, `ast`, `run`, and `test`
(Stage 6 added the last two — `run` executes the linked binary with
stdout/stderr/exit code passed straight through; `test` does the same,
additionally framed as `test <path> ... ok`/`FAILED` by the process's
own exit code, but always propagates that exit code either way), with:

```text
--target <llvm-triple>
-O0 | -O1 | -O2 | -O3 | --release  (sugar for -O3)
-o <output-path>
--emit-object
--emit-ast | --emit-hir | --emit-mir | --emit-llvm
```

The `--emit-*` flags each write a debug dump to a sibling file
(`<output-or-source>.ast`/`.hir`/`.mir`/`.ll`), mirroring
`--emit-object`'s own sibling-naming convention. Hand-rolled argument
parsing throughout — no `clap` dependency, consistent with this
workspace's minimal-dependency posture elsewhere. It renders structured
diagnostics and returns a nonzero status for front-end or toolchain
failures.

## Testing and remaining extension points

Stage-specific tests live with their crates. Driver integration tests
exercise parse-to-object, compile-link-run, module isolation, stdlib,
closures, weak references, generic traits/types/methods, aggregates,
arrays, mutable parameters, diagnostics, options and every checked-in
example.

Incremental compilation, package management, import aliases, a richer
standard library, compact enum layouts and non-host linking remain future
work. They are deliberate feature boundaries rather than parser-only
syntax that fails in a later compiler stage.
