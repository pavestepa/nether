# Nether

Nether is an experimental statically typed native language. It combines
Rust-like static typing and traits with Swift-like ARC reference semantics
*and* Rust-like unique ownership in one coherent type system — the
programmer chooses per binding, not the language globally — and compiles
through LLVM to a native executable. See
[`docs/spec/language-spec.md`](docs/spec/language-spec.md) for the full
design and [`docs/architecture/roadmap.md`](docs/architecture/roadmap.md)
for what's implemented today versus staged for later.

The compiler implements:

- lexer, parser, diagnostics, name resolution, and type checking;
- the four-value-form ownership model (`T`/`:T`/`t`/`:t` — ARC reference,
  uniquely owned heap value, inline value, uniquely owned inline value);
- traits with static dispatch, multiple inheritance, defaults, and
  generic monomorphization;
- HIR, closure conversion, CFG-based MIR, and ARC insertion;
- structs, tuples, enums/match, arrays, weak references, and closures;
- local multi-file modules through `use`;
- LLVM object emission, optimization levels, and native linking;
- a small Nether-source Option/Result/Array standard library.

Flow-sensitive move checking for uniquely owned locals is implemented.
Borrowing is currently enforced at call scope (`:&T`/`:&mut T` parameters
and receivers, including exclusivity within one argument list); stored and
returned borrows still await Stage 2's lifetime/origin inference. See the
roadmap for the exact boundary.

## Requirements

- Rust stable;
- LLVM 18;
- a C linker available as `cc`.

The checked-in `.cargo/config.toml` points at Homebrew's ARM macOS LLVM 18:

```sh
brew install llvm@18
```

On another platform, set `LLVM_SYS_180_PREFIX` to that platform's LLVM 18
installation and adjust/remove the macOS-specific linker search path in
`.cargo/config.toml`.

## Build and test

```sh
cargo build -p nether-cli
cargo test --workspace
```

Build the runtime libraries once before producing linked executables:

```sh
cargo build --release \
  -p nether-rt-arc \
  -p nether-rt-string \
  -p nether-rt-array \
  -p nether-rt-io
```

Compile a program:

```sh
cargo run -p nether-cli -- build examples/hello_world/main.nr
./examples/hello_world/main
```

Useful options:

```text
--target <llvm-triple>
-O0 | -O1 | -O2 | -O3
-o <output-path>
--emit-object
```

For example, emit an optimized object without linking:

```sh
cargo run -p nether-cli -- build examples/hello_world/main.nr -O2 --emit-object
```

## Examples

`examples/hello_world/main.nr` is the checked-in compilable showcase: a
PascalCase heap (`struct`) type, a lowercase inline `struct`, all four
`let`-binding value forms, an owned struct literal consumed by an owned
parameter, and methods. The driver integration suite compiles, links and
executes it, so it is kept in sync with the language. See
[`docs/spec/language-spec.md`](docs/spec/language-spec.md) §23 for a
similarly-shaped, fully annotated reference example (that one also covers
`Into<String>` and traits, which the checked-in example does not).

Earlier multi-file showcases (`shelter`, `metrics`, `adventure`) used
pre-rewrite MVP syntax and were removed rather than migrated; see
[`docs/architecture/roadmap.md`](docs/architecture/roadmap.md) for the
rewrite's staging.

## Modules and standard library

A file is a module. Declare a child similarly to Rust:

```nether
mod lang;
use self.lang.Lang;

fn main() {
    let value = Lang.new("Nether");
}
```

`mod lang;` looks for `lang.nr` or `lang/mod.nr` (the legacy `.nt`
extension is rejected with a migration diagnostic — language-spec §2.1). A
non-root `foo.nr` may declare nested children below `foo/`. Import roots
have their Rust meanings: `self` is the current
module, `super` is its parent, and `crate` is the entry module. Nether uses
`.` everywhere instead of Rust's `::`.

Each file has its own top-level namespace, while an `impl` in a child can
extend a parent type after `use super.Type;`. Import aliases, globs, and
inline `mod name { ... }` blocks are not implemented; general per-file
`use` re-export isn't either, except for the one bundled case described
next. Bundled standard-library modules are available without a `mod
stdlib;` declaration:

```nether
use stdlib.result.Result;

fn main() {
    let ok Result<i32, String> = Result.Ok(4);
    println(`${ok.map((x i32) => { x + 1 }).unwrap_or(0)}`);
}
```

`stdlib/mod.nr` is additionally always loaded as a program-wide prelude,
independent of whether anything `use`s it: every top-level `use` written
in that one file is re-exported to every other file with no `use`/`mod`
of its own (a local declaration of the same name is a legal shadow, not
a conflict). `Option`/`Result`/`Array` themselves are ordinary generic
`enum`/`struct` declarations this way (`stdlib/option.nr`/
`stdlib/result.nr`/`stdlib/array.nr`), not compiler builtins — their
hand-written methods use the explicit `impl<T> Option<T> { ... }` form, see
[`docs/generics.md`](docs/generics.md) § "Methods on generic types" — and
the whole thing is available with zero ceremony, no `use` at all:

```nether
fn main() {
    let value Option<i32> = Option.Some(4);
    println(`${value.unwrap_or(0)}`);
}
```

`stdlib/` is also reachable under the name `std`, either explicitly
(`mod std;`, mounting the same tree `mod stdlib;` would if it existed) or
directly in a `use` path (`use std.result.Result;`, equivalent to
`use stdlib.result.Result;`) — no `mod std;` declaration required.

## Architecture

```text
source → lexer → parser → resolver → typecheck
       → HIR → monomorphization → MIR + ARC
       → LLVM IR → object → linker → executable
```

The language reference is in
[`docs/spec/language-spec.md`](docs/spec/language-spec.md). Compiler-stage
boundaries and memory-management rules are described under
[`docs/architecture`](docs/architecture).

For working examples of generic functions, types, enums, methods,
traits, bounds, inheritance and current limitations, see
[`docs/generics.md`](docs/generics.md).

Nether has no garbage collector, macros, or reflection, and — for now —
no lifetime-aware borrow checking, async, threads, unsafe code, or dynamic
trait dispatch; see
[`docs/architecture/roadmap.md`](docs/architecture/roadmap.md) for which
stage adds each of those.

Current deliberate limits are local generic inference (no
where-clauses or associated types; concrete specialization exists but is
scoped to instance methods only, see `docs/generics.md`), one bound per
generic parameter, no general import aliases/globs/re-exports (bundled
`stdlib/mod.nr` is a special-cased exception, see "Modules and standard
library" above), a flattened/non-union enum layout, and native linking
only for the host target. Cross-target object emission is supported.
Variadic parameters (`fn f(args ...String)`) are supported as sugar over
`Array` — see `docs/spec/language-spec.md` §8.7.

**Known bug, not yet fixed:** a function that loops over an `Array`
*parameter* reassigning a `String` local via template-string
concatenation each iteration (`result = \`${result}${x}\`;`) corrupts
memory when that function is called more than once with differently
sized arrays — sometimes wrong output, sometimes a crash, depending on
unrelated heap state. Reproduces with a plain, non-generic, non-variadic
`Array<String>` parameter; unrelated to generics/specialization/variadics.
See `string_concatenation_by_reassignment_inside_a_loop_over_an_array_parameter_corrupts_memory`
in `compiler/driver/tests/driver_tests.rs` (`#[ignore]`d — reproduce with
`cargo test -p nether-driver --test driver_tests -- --ignored <name>` in
isolation; it can crash the whole test process).
