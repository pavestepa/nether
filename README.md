# Nether

Nether is an experimental statically typed native language. It combines a
Rust-like surface syntax with automatic reference counting instead of
ownership/borrowing and compiles through LLVM to a native executable.

The compiler implements:

- lexer, parser, diagnostics, name resolution, and type checking;
- interfaces with static dispatch and generic monomorphization;
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

## Modules and standard library

A file is a module. `use lang.Lang;` loads `lang.nt`/`lang.nr` next to the
importing file. Nested paths map to directories. Each file has its own
top-level namespace, so private declarations with equal names do not
collide across modules. Bundled standard-library modules are available
from the repository root:

```nether
use stdlib.option.option_map;
use stdlib.option.option_unwrap_or;

fn main() {
    let mapped = option_map(Option.Some(4), (x: i32) => { x + 1 });
    println(option_unwrap_or(mapped, 0));
}
```

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

Nether intentionally has no borrow checker, garbage collector, async,
threads, unsafe code, macros, reflection, or dynamic interface dispatch in
its MVP.

Current deliberate MVP limits are local generic inference (no
where-clauses, associated types, or specialization), one bound per generic
parameter, no import aliases, a flattened/non-union enum layout, and native
linking only for the host target. Cross-target object emission is supported.
