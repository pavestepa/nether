# The Nether Language — Reference (MVP)

Status: **resolved draft** — this document is the single source of truth for
the language. It supersedes any earlier scratch notes or example files
(including anything under a `.nr`/`.nt` file predating this document, and any
notes from outside this repository). Where an example elsewhere in the repo
disagrees with this document, this document wins.

This is not a tutorial. It is written the way a language reference is
written: one section per concept, precise enough that the parser, resolver,
and type checker can each be implemented directly from it without guessing.

---

## 1. Design summary

Nether looks like Rust at a distance, but makes different foundational
choices:

| Concern | Rust | Nether |
|---|---|---|
| Memory management | ownership + borrow checker | ARC (automatic reference counting) |
| Polymorphism | traits + optional dynamic dispatch | interfaces, static dispatch only |
| Generics | monomorphization | monomorphization |
| Concurrency | threads + async | none (single-threaded, synchronous) |
| Unsafe code | `unsafe` blocks, raw pointers | none — no raw pointers, no unsafe |

Everything below assumes: no ownership system, no borrow checker, no async,
no multithreading, no macros, no reflection, no garbage collector. These are
permanent MVP constraints, not omissions to fill in later.

---

## 2. Lexical structure

### 2.1 Comments

```
// line comment
/// doc comment (attaches to the following item)
```

Block comments are not part of the MVP grammar (only line comments and doc
comments — kept intentionally minimal; block comments are a possible future
extension, see §14).

### 2.2 Identifiers and casing

Identifiers are ASCII `[A-Za-z_][A-Za-z0-9_]*`. Casing is not merely a style
convention in Nether — it is **load-bearing grammar** that the type checker
reads to decide allocation strategy (see §3.3). This is a deliberate
divergence from Rust, where casing is only a lint.

### 2.3 Literals

- Integers: `123`, `0`. Suffixes are inferred from context (expected type),
  not written (no `123i32` suffix syntax in the MVP).
- Floats: `1.0`, `0.5`.
- Booleans: `true`, `false`.
- Char: `'a'` — a single Unicode scalar value.
- Plain string: `"text"` — no interpolation is scanned inside a plain
  string. `\n`, `\t`, `\\`, `\"` escapes are recognized.
- Template string: `` `text ${expr} text` `` — the lexer scans for `${` and
  emits an interpolation segment; everything else is literal text, with the
  same escapes as a plain string. **These are two distinct token kinds at
  the lexer level**, not one string kind with optional interpolation. This
  was chosen over a single always-interpolating `"..."` form so that:
  - the common case (a plain string) never pays for interpolation scanning,
  - interpolation is visually explicit at the call site,
  - it matches the existing example corpus (`first_example.nr`).
- Arrays: `[]` (empty), `[1, 2, 3]` (literal elements).

### 2.4 Semicolons and tail expressions

Nether follows Rust's rule exactly: a block is a sequence of
semicolon-terminated statements followed optionally by one final expression
*without* a trailing semicolon, which becomes the block's value. A
statement-position expression followed by more statements **must** have a
`;`. (An earlier scratch example omitted a `;` on a non-tail statement; that
was a bug in the example, not a language rule — see the corrected version in
§6.)

---

## 3. Types

### 3.1 Primitives

```
bool char
i8 i16 i32 i64
u8 u16 u32 u64
usize isize
f32 f64
```

All primitives are stack/value types (see §3.3): copied on assignment, no
identity, no ARC involvement.

### 3.2 Named types: `type`

`type` declares a struct, tuple-struct, or unit-like type. `struct` is not a
keyword in Nether.

```
type Dog {
    name: String
}

type Point(i32, i32);   // tuple-struct
type EmptyType;         // unit-like type — no braces, no parens, one semicolon
```

### 3.3 Heap vs. stack: the naming rule

This is the central, distinguishing rule of Nether's memory model, and it is
about **type declaration casing**, not variable casing:

- A `type` declared with a **PascalCase** name (`String`, `Array`, `Dog`,
  `User`, `Point`) is **heap-allocated and ARC-managed**. Assigning a value
  of this type shares the same underlying object (retain), it does not
  copy.
- A `type` declared with a **camelCase** name (`point`, `vector3`) is a
  **stack/value type**. Assigning a value of this type clones it.
- Primitives (`i32`, `bool`, `usize`, ...) are always stack/value types.

**Enums and tuples are exempt from this rule.** An `enum` — regardless of
its name's casing — and an anonymous tuple type `(A, B, ...)` are *always*
stack/value types: a tag plus an inline payload, copied on assignment, with
no independent heap allocation for the enum/tuple value itself. This
exemption exists because enum type names are conventionally PascalCase
(`Color`, `Option`, `Result`) and the built-in `Option`/`Result` types are
used pervasively — applying the naming rule literally to enums would ARC-heap
-allocate every `Option`/`Err` in the program, which contradicts this same
spec's statement that enum representation is "tag + union, similar to Rust"
(i.e., a zero-cost value representation). If a variant holds a field of a
heap type (e.g. `Custom(String)`), that field is independently ARC-managed
through its own type's rule — only the enum's own tag+payload shell is
exempt.

So, precisely:

| Kind | Allocation |
|---|---|
| `type` with PascalCase name | heap, ARC |
| `type` with camelCase name | stack, clone |
| primitive (`i32`, `bool`, ...) | stack, clone |
| `enum` (any name) | stack, clone (tag + inline payload); heap fields inside a payload follow their own type's rule |
| tuple `(A, B, ...)` | stack, clone; each element follows its own type's rule |
| tuple-struct (PascalCase) | heap, ARC — this is a `type`, not a bare tuple |
| tuple-struct (camelCase) | stack, clone |

### 3.4 Weak references

`weak T` is only meaningful where `T` is a heap/ARC type. `weak` on a
stack/value type or an enum is a compile error (there is no reference count
to weakly reference). A `weak T` does not keep its referent alive; there is
no cycle collector — cycles are the programmer's responsibility to break
explicitly, exactly as in Swift.

### 3.5 Built-in heap types

`String` (UTF-8) and `Array` (contiguous, growable) are heap-allocated,
ARC-managed types. The compiler knows only their semantic shape (a `String`
is a UTF-8 byte sequence, an `Array<T>` is a sequence of `T`); the actual
storage layout is a runtime concern.

`Array<T>`'s core operations — `push`, `pop`, `len`, and indexing (`a[i]`) —
are implemented in the runtime (§9), not in Nether source, for performance.
Beyond that core, `Array<T>` is an ordinary generic type declared in the
bundled prelude (`stdlib/array.nt`): user code can add further methods with
`impl<T> Array<T> { ... }` the same way it extends `Option<T>`/`Result<T, E>`
(§8), and those methods see an ordinary `self: Array<T>` receiver — indexing
and the runtime methods above are all usable from inside them. `String`
stays fully closed — it has no generic parameter and does not support user
`impl` blocks.

### 3.6 Tuples

Tuples support literal construction, `.0`/`.1`/... field access, and
destructuring, matching Rust semantics:

```
let pair = (1, "a");
let (x, y) = pair;
println(pair.0);
```

Tuple-structs (`type Point(i32, i32);`) use the same `.0`/`.1` access and
also support destructuring via their constructor pattern.

---

## 4. Visibility

Everything (fields, methods, modules) is public by default. A name is
private if it starts with `_`, or if declared with the `private` keyword —
both spellings are equivalent and interchangeable:

```
private field
_field
```

There is no third, intermediate visibility level (e.g. no `pub(crate)`) in
the MVP.

---

## 5. Variables and bindings

```
let a = 4;        // immutable binding
let mut a = 4;     // mutable binding
```

- Assigning a stack/value type clones it.
- Assigning a heap/ARC type shares the same object (retain).

### 5.1 Mutation requires `mut`

A binding must be declared `mut` to mutate through it — directly
(`x = ...`), through a field at any depth (`x.a.b = ...`), or by calling a
`mut self` method anywhere along that chain — for **both stack and heap
types**, with no exemption for either:

```
type Dog { name: String }

impl Dog {
    set_name(mut self, new_name: String) {
        self.name = new_name;
    }
}

fn main() {
    let dog = Dog { name: "Rex" };
    dog.name = "Buddy";      // error: `dog` is not `mut`
    dog.set_name("Buddy");   // error: `set_name` needs a `mut self` receiver
}
```

```
type Dog { name: String }

impl Dog {
    set_name(mut self, new_name: String) {
        self.name = new_name;
    }
}

fn main() {
    let mut dog = Dog { name: "Rex" };
    dog.name = "Buddy";      // ok
    dog.set_name("Buddy");   // ok
}
```

Ordinary function/method parameters are pass-by-value for stack types (the
callee gets a clone) and pass-by-shared-reference for heap types (ARC
retain/release around the call, per §8). To let a callee mutate through a
parameter in the caller's own storage — a stack type's own value, or a
field/`mut self` method reached through a heap type's shared reference —
the parameter is declared `mut`, mirroring `mut self`:

```
fn increment(mut n: i32) {
    n = n + 1;   // mutates the caller's variable, not a clone
}

fn rename(mut d: Dog, new_name: String) {
    d.set_name(new_name);   // mutates the caller's shared Dog, not a copy
}

fn main() {
    let mut x = 4;
    increment(mut x);   // caller must also mark the argument `mut`
    println(x);         // prints 5

    let mut dog = Dog { name: "Rex" };
    rename(mut dog, "Buddy");
    println(dog.name);  // prints "Buddy"
}
```

The parameter is a genuine mutable alias to the caller's storage, not a
copy. Unlike Rust's `&mut`, there is **no borrow-checker enforcement** — no
exclusivity/aliasing rules are checked, so multiple `mut` aliases to the
same heap object can coexist and each mutate through it; this is a bare
capability to mutate through an alias, consistent with Nether having no
borrow checker at all. The caller must write `mut` at the call site as well
as the callee declaring `mut` on the parameter, so that mutation is visible
at both ends without requiring alias analysis to prove it.

### 5.2 Variadic parameters

A function or method's **last** parameter may be declared variadic —
`...ElementType` instead of a plain type — to accept zero or more
trailing arguments:

```
fn sum(items: ...i32): i32 {
    let mut total = 0;
    for item in items {
        total = total + item;
    }
    total
}

fn main() {
    println(`${sum()}`);         // 0
    println(`${sum(1)}`);        // 1
    println(`${sum(1, 2, 3)}`);  // 6
}
```

This is sugar over `Array<ElementType>`, not a distinct calling
convention: the callee's own body sees an ordinary `Array<ElementType>`
value (`items.len()`, `for item in items`, ... all work exactly as they
would on any other array), and a call site's trailing arguments —
whatever is left over after the fixed parameters — are collected into an
`Array<ElementType>` literal automatically. Only one variadic parameter is
allowed, and it must be the last one declared.

A variadic parameter whose element type is `String` additionally accepts
any `Into<String>` value at each trailing position, not just literal
`String`s — the same conversion string-template interpolation (§2.3) and
`println`/`print` (§12) already apply:

```
fn show(args: ...String) {}

fn main() {
    show(1, true, "text");   // each argument converted through Into<String>
}
```

---

## 6. Types, `impl`, methods

```
type Dog {
    name: String
}

impl Dog {
    new(name: String): Dog {
        Dog { name }
    }

    set_name(mut self, new_name: String) {
        self.name = new_name;
    }
}
```

- `impl` blocks contain methods. There is no `fn` keyword inside `impl` —
  the method name starts the declaration directly.
- A method with `self` or `mut self` as its first parameter is an instance
  method; a method with no `self` parameter is a static method, called as
  `Dog.new(...)`. A static method may also be selected through a value
  (`dog.new(...)`); the receiver expression is evaluated for its side
  effects but is not passed to the method.
- **There is no `Self` type.** A constructor or method that needs to refer
  to the enclosing type names it explicitly:

```
type Lang {
    name: String
}

impl Lang {
    new(name: String): Lang {
        Lang { name }
    }

    set_name(mut self, new_name: String) {
        self._check();
        self.name = new_name;
        println(self.name);
    }

    // a name starting with '_' is private
    _check(self) {
        print("changed from: ", self.name, " to: ");
    }
}
```

(This corrects a bug in the original scratch file, where `self.new_name;`
appeared as a no-op statement instead of `self.name = new_name;`.)

Tuple-struct and unit-like `impl`s follow the same rules:

```
type Point(i32, i32);
impl Point {
    new(x: i32, y: i32): Point {
        Point(x, y)
    }
}

type EmptyType;
impl EmptyType {
    // static-only methods are legal here
}
```

(This corrects a second scratch bug, where a tuple-struct constructor
returned `SomeType(String)` — the type name — instead of the constructor's
own parameter.)

### 6.1 Standalone functions

Functions outside any `impl` block use `fn`:

```
fn main() {
    let a = 4;
    foo(a);
}

fn foo(a: i32) {
    println(a);
}
```

Nether has **no nested functions** — only top-level `fn` declarations and
closures (§11).

---

## 7. Interfaces

`interface` replaces `trait`. Dispatch is always static (monomorphized);
there are no vtables and no dynamic dispatch.

```
interface Sound {
    // A declaration such as `type Dog: Sound` opts into this default.
    sound(): String {
        "..."
    }
}

impl Dog: Sound {
    sound(): String {
        "Woof! Ruff!"
    }
}
```

An interface listed on a declaration opts into its default implementations:

```
type Dog: Sound, Clone {
    name: String
}

enum State: Sound {
    Ready,
    Waiting
}
```

By contrast, `impl Type: Interface { ... }` is an explicit implementation:
every interface method must have a user-written implementation, including
methods for which the interface declares a default. Several interfaces may
be named in one block (`impl Dog: Sound, Clone { ... }`).

Methods and interface declarations may be freely split and mixed across
multiple blocks. All methods written for one owner are collected before
interface conformance is checked, so a method in `impl Dog { ... }` may
satisfy an interface named by another `impl Dog: Sound { ... }` block.

Interfaces support multiple inheritance:

```
interface Pet: Sound, Named {
    play(self);
}
```

Implementing `Pet` also satisfies `Sound` and `Named` and requires their
methods transitively. A child declaration overrides a same-named parent
method. Incompatible inherited signatures are an error; conflicting
default bodies require the concrete type to provide an explicit method.
Inheritance cycles are rejected.

Because dispatch is always resolved at compile time, an interface name may appear
**only as a generic bound** (`fn f<T: Sound>(x: T)`) — it can never be used
as a standalone value type (no `dyn Interface`, no heterogeneous
`Array<Sound>` holding mixed concrete types). This is a direct consequence
of "static dispatch only, no vtables," not an extra restriction.

### 7.1 The `Into<T>` convention

`Into<T>` is the conversion interface used by `println`/`print` and other
stdlib functions that accept "anything convertible to `T`". Its required
method is named `into_<t>` in snake_case — for `Into<String>`, the method is
`into_string`:

```
impl Dog: Into<String> {
    into_string(self): String {
        `name: ${self.name}`
    }
}
```

---

## 8. Generics

Generic types, interfaces, and functions are supported and implemented via
Rust-style monomorphization: one specialized copy of the code is generated
per concrete instantiation between HIR and MIR. There is no generic code
left in the final binary — every call site resolves to a concrete,
non-generic function.

Generic parameters are supported on functions, methods, named types, tuple
structs, enums, and interfaces:

```nether
fn identity<T>(value: T): T { value }
type Boxed<T> { value: T }
type Pair<T, U>(T, U);
enum Maybe<T> { Some(T), None }
interface Read<T> { read(self): T; }
```

The parameters declared by a type or enum are implicitly in scope in all
of its `impl` blocks. Methods may declare additional parameters of their
own. Function and method type arguments are inferred locally from the
receiver, ordinary arguments, and expected return type, or written
explicitly in declaration order:

```nether
let number = identity<u32>(1);
let text = box.replace<String>("ready");
```

The explicit list is written directly after the callable name. Rust's
`identity::<u32>(1)` syntax is not part of the grammar. For an instance
method, owner parameters are fixed by the receiver and the list supplies
all remaining method parameters. For a standalone function it supplies
the complete declared parameter list; partial explicit lists are rejected.
Explicit types do not bypass ordinary argument compatibility or bounds.

Each generic parameter may declare one inline interface bound, including a
generic interface application:

```nether
fn make_noise<T: Sound>(value: T): String { value.sound() }
fn read_text<T: Read<String>>(value: T): String { value.read() }
type SpeakerBox<T: Sound> { value: T }
```

Multiple effective requirements are expressed by inheriting several
interfaces and using the child as the single inline bound. Interface
inheritance and generic arguments are checked transitively.

Every generic parameter must be explicit or inferable at a call site. A
payload-free variant such as `Option.None` likewise needs a type annotation
or other context when its arguments cannot be inferred.

The current implementation deliberately has no partial explicit argument
lists, Rust-style turbofish, `where` clauses, multiple inline bounds,
associated types, specialization, blanket/conditional implementations,
const generics, higher-kinded types, or first-class unspecialized generic
functions.

See [`../generics.md`](../generics.md) for the complete example-driven
guide and the supported/unsupported feature matrix.

Heap-vs-stack classification (§3.3) is resolved **after** substitution:
`Array<i32>` and `Array<Dog>` are both heap/ARC (because `Array` itself is
PascalCase), independent of whether their type argument is heap or stack.

---

## 9. Enums and match

```
enum Color {
    Red,
    Green,
    Blue,
    White,
    Black,
    Custom(String),
}

enum Outcome<T, E> {
    Ok(T),
    Error(E),
}
```

Variant payloads use tuple-call syntax, `Ok(T)` / `Error(E)` — not a colon
form. (An older scratch file used `Ok: O`; that syntax is retired in favor
of the form actually specified here.) Internally, an enum is represented as
a tag plus flat payload storage sized for all variant fields (§3.3);
construction never allocates independently of what its payload types
themselves require. `Option<T>` and `Result<T, E>` are bundled generic
enums available without redeclaration.

`match` is Rust-like, MVP scope only: no guards in the initial version.

```
fn describe(color: Color) {
    match color {
        Color.Red => println("is red!"),
        Color.Green => println("is green!"),
        Color.Blue => println("is blue!"),
        Color.White => println("is white!"),
        Color.Black => println("is black!"),
        Color.Custom(name) => println(name),
    }
}
```

Variant access uses `.`, consistent with the rest of the language never
using `::`.

---

## 10. Modules

The file-module model follows Rust. A `mod` item declares a child and
`use` imports one declaration from a module:

```
mod user;
use self.user.User;

fn main() {
    let u = User.new("Ada");
}
```

For `mod user;`, the driver looks for `user.nt`/`user.nr` or
`user/mod.nt`/`user/mod.nr` relative to the declaring file. A child of
`user.nt` is normally stored below `user/`. Missing declared module files
are source-anchored compilation errors.

Relative roots are:

- `self` — the current file module;
- `super` — its declaring parent;
- `crate` — the entry file/module.

For example, a child can extend a type from its parent:

```
// main.nt
mod cat_conversions;
type Cat { name: String }

// cat_conversions.nt
use super.Cat;
impl Cat {
    into_i32(self): i32 { 1 }
}
```

Nether uses `.` where Rust uses `::`. Module paths occur in `mod`/`use`;
after import, source code refers to the imported final declaration by its
name. Each file retains a separate namespace. `stdlib.*` is an external
bundled root and does not require `mod stdlib;`.

The MVP does not support inline module bodies, aliases, glob imports or
re-exports. For compatibility, a direct `use user.User;` may still load a
nearby module, but new code should declare local children with `mod`.

---

## 11. Closures

```
let add = (a: i32, b: i32) => {
    a + b
};
```

Closures are the only anonymous/local callable construct — Nether has no
nested named functions. A closure captures each referenced outer variable
following the same rule as ordinary assignment: a stack/value type is
captured by clone (the closure gets its own independent copy — mutating it
inside the closure does not affect the outer variable), a heap/ARC type is
captured by shared reference (retain). There is no `mut`-capture / `FnMut`
-style mutable-by-reference capture in the MVP; see §14 for this as a future
extension point.

---

## 12. Built-in symbols

Available without any `use`:

```
println print
Option Some None
Result Ok Error
Into
```

Everything else in the standard library requires an explicit `use`.

---

## 13. Memory model summary

Full retain/release insertion rules live in
[`../architecture/arc-model.md`](../architecture/arc-model.md); the
language-level contract is:

- Heap assignment retains; scope exit releases.
- Passing a heap value into a Nether function retains it before the call;
  the callee's own scope exit (or, if it hands that same value straight
  back out as its return value, the return itself) is what releases it —
  never both (`arc-model.md` §3.3 has the precise rule and the real bug an
  earlier version of it had).
- Returning a freshly constructed heap value directly from a function skips
  the redundant retain/release pair (Return Value Optimization) — the
  caller receives the object that was already constructed, at +1, without
  an intermediate retain/release round-trip.
- `weak T` never affects the retain count of its referent.
- Mutable-reference parameters (§5.1) never retain/release — they alias a
  stack slot directly; no refcount is involved because they only ever apply
  to stack/value types.

---

## 14. Explicitly out of scope for the MVP (and why they're listed here)

These are permanent-for-now constraints repeated here because they interact
with rules above and a reader should not need to cross-reference the
project brief to know they're intentional:

- No async, no `await`, no multithreading, no `go`/goroutine-style
  concurrency. (An earlier, unrelated scratch note explored an
  async/ownership/concurrency design; it predates this spec and is
  superseded by it in full.)
- No ownership system, no borrow checker, no lifetimes, no raw pointers, no
  `unsafe` blocks.
- No dynamic dispatch, no vtables, no reflection.
- No macros, no `derive`, no proc macros, no const generics.
- No garbage collector, no cycle collector for ARC — cycles are broken
  manually with `weak`.
- No package manager — only local modules.

Documented future extension points (not MVP, but designed to not be
foreclosed by MVP decisions):

- Mutable closure captures (`FnMut`-style capture-by-reference).
- FFI marshaling of heap types (`String`, `Array`) across `extern "C"`
  boundaries — MVP FFI covers primitive types only.
- Block comments (`/* ... */`).
- A cycle-assistance tool (e.g. a lint that flags likely retain cycles)
  without introducing a runtime collector.

---

## 15. Canonical example

A single example exercising most of the surface above, with the bugs from
the original scratch files corrected:

```
mod lang;
use self.lang.Lang;

fn main() {
    let mut a = Lang.new("Bobby");
    a.set_name("Husky");
    println(a.into_string());
}

type Lang {
    name: String
}

impl Lang {
    new(name: String): Lang {
        Lang { name }
    }

    set_name(mut self, new_name: String) {
        self.name = new_name;
    }
}

impl Lang: Into<String> {
    into_string(self): String {
        `name: ${self.name}`
    }
}

interface Sound {
    sound(): String {
        "..."
    }
}

impl Lang: Sound {
    sound(): String {
        "Woof! Ruff!"
    }
}
```
