# Nether

Nether is an experimental statically typed native language. It combines a
Rust-like surface syntax with automatic reference counting instead of
ownership/borrowing and compiles through LLVM to a native executable.

The compiler implements:

- lexer, parser, diagnostics, name resolution, and type checking;
- interfaces with static dispatch, multiple inheritance, defaults, and
  generic monomorphization;
- HIR, closure conversion, CFG-based MIR, and ARC insertion;
- structs, tuples, enums/match, arrays, weak references, and closures;
- local multi-file modules through `use`;
- LLVM object emission, optimization levels, and native linking;
- a small Nether-source Option/Result standard library.

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
cargo run -p nether-cli -- build examples/some/first_example.nt
./examples/some/first_example
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
cargo run -p nether-cli -- build examples/some/enums.nt -O2 --emit-object
```

## Larger examples

Three multi-file projects under `examples/` are intended as compilable
language showcases:

- `shelter` — heap structs, `weak` fields, `Option`, enums and `match`,
  arrays, closures, templates, methods and `Into<String>`;
- `metrics` — value structs, higher-order functions, named function
  values, generic functions, tuples, arrays, loops and mutable scalar
  parameters;
- `adventure` — nested file modules, `self`/`super` imports, enum payloads,
  mutation through methods, closures and generic interface bounds.

Compile and run them from the repository root:

```sh
cargo run -p nether-cli -- build examples/shelter/main.nt
./examples/shelter/main

cargo run -p nether-cli -- build examples/metrics/main.nt
./examples/metrics/main

cargo run -p nether-cli -- build examples/adventure/main.nt
./examples/adventure/main
```

The driver integration suite compiles, links and executes all three
projects, so these examples are kept in sync with the language.

## Modules and standard library

A file is a module. Declare a child similarly to Rust:

```nether
mod lang;
use self.lang.Lang;

fn main() {
    let value = Lang.new("Nether");
}
```

`mod lang;` looks for `lang.nt`, `lang.nr`, `lang/mod.nt`, or
`lang/mod.nr`. A non-root `foo.nt` may declare nested children below
`foo/`. Import roots have their Rust meanings: `self` is the current
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
    let ok: Result<i32, String> = Result.Ok(4);
    println(`${ok.map((x: i32) => { x + 1 }).unwrap_or(0)}`);
}
```

`stdlib/mod.nt` is additionally always loaded as a program-wide prelude,
independent of whether anything `use`s it: every top-level `use` written
in that one file is re-exported to every other file with no `use`/`mod`
of its own (a local declaration of the same name is a legal shadow, not
a conflict). `Option`/`Result` themselves are ordinary generic `enum`s
declared this way (`stdlib/option.nt`/`stdlib/result.nt`), not compiler
builtins — their hand-written methods use the explicit
`impl<T> Option<T> { ... }` form, see
[`docs/generics.md`](docs/generics.md) § "Methods on generic types" — and
the whole thing is available with zero ceremony, no `use` at all:

```nether
fn main() {
    let value: Option<i32> = Option.Some(4);
    println(`${value.unwrap_or(0)}`);
}
```

`stdlib/` is also reachable under the name `std`, either explicitly
(`mod std;`, mounting the same tree `mod stdlib;` would if it existed) or
directly in a `use` path (`use std.result.result_map;`, equivalent to
`use stdlib.result.result_map;`) — no `mod std;` declaration required.

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
interfaces, bounds, inheritance and current limitations, see
[`docs/generics.md`](docs/generics.md).

Nether intentionally has no borrow checker, garbage collector, async,
threads, unsafe code, macros, reflection, or dynamic interface dispatch in
its MVP.

Current deliberate MVP limits are local generic inference (no
where-clauses or associated types; concrete specialization exists but is
scoped to instance methods only, see `docs/generics.md`), one bound per
generic parameter, no general import aliases/globs/re-exports (bundled
`stdlib/mod.nt` is a special-cased exception, see "Modules and standard
library" above), a flattened/non-union enum layout, and native linking
only for the host target. Cross-target object emission is supported.
Variadic parameters (`fn f(args: ...String)`) are supported as sugar over
`Array` — see `docs/spec/language-spec.md` §5.2.

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
