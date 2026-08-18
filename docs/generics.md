# Generics in Nether

This document is a practical guide to the generic features currently
implemented by Nether. The normative language definition remains
[`spec/language-spec.md`](spec/language-spec.md); this guide focuses on
complete examples, inference behavior, trait constraints, and current
limitations. Ordinary (ARC/inline-domain) parameter, field, and return
syntax is used throughout — see the language spec §3/§8 for the
unique-ownership (`:T`) forms, which this generics-focused guide does not
otherwise exercise.

## Feature overview

| Feature | Status |
| --- | --- |
| Generic functions | Supported |
| Generic methods | Supported |
| Generic structs and tuple structs | Supported |
| Generic enums | Supported |
| Generic traits | Supported |
| Bounds such as `T Sound` and `T Convert<String>` | Supported |
| Generic trait inheritance | Supported |
| Multiple effective bounds through trait inheritance | Supported |
| Local type-argument inference | Supported |
| Expected-return-type inference | Supported |
| Monomorphization | Supported |
| Explicit call arguments such as `f<i32>()` | Supported |
| Explicit `impl<T> Boxed<T> { ... }` (positional renaming only) | Supported |
| `default impl` + concrete specialization, instance methods only | Supported |
| Non-generic type aliases (`type Name = TypeExpr;`) | Supported |
| `where` clauses | Supported |
| Multiple inline bounds such as `T A + B` | Supported |
| Associated types | Not supported |
| Specialization of static methods or trait conformance | Not supported |
| Blanket implementations | Not supported |
| Const generics | Not supported |
| Higher-kinded types | Not supported |
| Generic type aliases | Not supported |
| First-class unspecialized generic functions | Not supported |
| Dynamic trait dispatch | Deliberately not supported (see `any Trait`, not yet implemented — Stage 3) |

## Generic functions

Declare generic parameters after the function name:

```nether
fn identity<T>(value T) T {
    return value
}

fn choose<T>(condition bool, left T, right T) T {
    if condition {
        return left
    } else {
        return right
    }
}

fn main() {
    let number i32 = identity(42);
    let text String = identity("Nether");
    let selected bool = choose(true, true, false);
}
```

When call arguments are omitted, the compiler must infer every generic
parameter. Inference uses argument types and, when available, the expected
return type:

```nether
fn empty<T>() Option<T> {
    Option.None
}

fn main() {
    // T is inferred as i32 from the declared result type.
    let value Option<i32> = empty();
}
```

Type arguments may instead be written explicitly between the callable name
and `(`:

```nether
fn main() {
    let number = identity<u32>(42);
    let text = identity<String>("ready");
}
```

Nether deliberately does not use Rust's `::` turbofish:

```nether
// Not supported:
// let value = identity::<u32>(42);
```

Explicit arguments are useful when a parameter cannot be inferred:

```nether
fn opaque<T>(value i32) i32 {
    return value
}

fn main() {
    // T does not occur in the ordinary parameter or return type.
    let value = opaque<String>(1);
}
```

For a standalone function, the explicit list must provide all of its
generic parameters in declaration order. Partial explicit lists are not
supported. Ordinary values are still checked against the specialized
signature, and every bound is still enforced.

## Generic structs and tuple structs

Both named-field and tuple structs may be generic:

```nether
struct Boxed<T> {
    value T
}

struct Pair<T, U>(T, U);

fn main() {
    let number = Boxed { value = 42 };
    let text = Boxed { value = "ready" };
    let pair = Pair(number, text);

    let left i32 = pair.0.value;
    let right String = pair.1.value;
}
```

Constructors infer their type arguments from their fields. A type annotation
may instead provide the expected arguments:

```nether
fn main() {
    let number Boxed<i32> = Boxed { value = 42 };
}
```

The number and order of type arguments are checked. For example,
`Boxed<i32, String>` and bare `Boxed` in a type position are invalid.

## Generic enums

Enums may declare any number of type parameters:

```nether
enum Outcome<T, E> {
    Ok(T),
    Error(E)
}

enum Maybe<T> {
    Some(T),
    None
}

fn unwrap_or<T>(value Maybe<T>, fallback T) T {
    return match value {
        Some(item) => item,
        None => fallback,
    }
}

fn main() {
    let result Outcome<i32, String> = Outcome.Ok(7);
    let value = unwrap_or(Maybe.Some("ready"), "missing");
}
```

A payload-free generic variant such as `Maybe.None` needs context:

```nether
fn main() {
    let known Maybe<i32> = Maybe.None;

    // Error: there is no information from which to infer T.
    // let unknown = Maybe.None;
}
```

`Option<T>` and `Result<T, E>` are bundled generic enums and obey the same
inference and substitution rules as user declarations.

## Methods on generic types

The generic parameters declared by a type are automatically in scope in all
of its `impl` blocks. Do not repeat `<T>` after `impl`:

```nether
struct Boxed<T> {
    value T
}

impl Boxed {
    new(value T) Boxed<T> {
        Boxed { value }
    }

    get(self) T {
        self.value
    }
}

fn main() {
    let box = Boxed.new("text");
    let value String = box.get();
}
```

An explicit Rust-like form is also accepted, `impl<T> Boxed<T> { ... }`.
It behaves identically to the implicit form above — `target_args` (the
`<T>` after `Boxed`) must be exactly a permutation of the impl's own
`<...>` names, in the owner's positional order. It is pure renaming, not
specialization: writing `impl<T> Boxed<T>` alongside `impl<T, U> Boxed<T>`
(an unused impl parameter) is rejected, since both still write `<...>`
after `impl`. A concrete argument list with no `<...>` after `impl` at
all — `impl Boxed<i32> { ... }` — is a different, valid form; see
"Concrete specialization" below.

```nether
impl<T> Boxed<T> {
    get(self) T {
        return self.value
    }
}
```

`Option`/`Result` themselves are ordinary generic `enum`s declared this
way — `stdlib/option.nr`/`stdlib/result.nr` — reachable everywhere with no
`use` as part of the bundled prelude (see "Modules and standard library"
in the [README](../README.md)), not compiler builtins:

```nether
// stdlib/option.nr
enum Option<T> {
    Some(T),
    None,
}

impl<T> Option<T> {
    unwrap_or(self, fallback T) T {
        return match self {
            Some(item) => item,
            None => fallback,
        };
    }
}
```

`Array<T>` joins the same pattern — `stdlib/array.nr` declares a bare
`struct Array<T>;` (no fields; its storage is still runtime-managed) and
adds methods the same way:

```nether
// stdlib/array.nr
struct Array<T>;

impl<T> Array<T> {
    map(mut self, iter (T) => ()) {
        let mut i usize = 0;
        while i < self.len() {
            iter(self[i]);
            i = i + 1;
        }
    }
}
```

`len`/`push`/`pop` and indexing (`self[i]`) stay runtime-backed for
performance rather than going through this `impl` block, but they're usable
from inside one exactly as shown — `self` types as an ordinary `Array<T>`.

The explicit form remains the only way to write methods for a target with
*no* declaration reachable at all from the writing site (there is no such
case left in the bundled standard library itself after the above, but a
project embedding Nether's compiler as a library could still seed a
builtin type this way).

Each impl-level parameter may still carry its own bound, in addition to
whatever the target's own declaration already requires — methods in that
one block need the bound, other `impl` blocks for the same type are
unaffected:

```nether
impl<T Sound> Boxed<T> {
    announce(self) String {
        return self.value.sound()
    }
}
```

Methods may introduce additional generic parameters:

```nether
struct Boxed<T> {
    value T
}

impl Boxed {
    replace<U>(self, value U) Boxed<U> {
        return Boxed { value }
    }
}

fn main() {
    let number = Boxed { value = 1 };
    let text Boxed<String> = number.replace<String>("one");
}
```

For an instance method, owner arguments such as `Boxed<i32>` are already
known from the receiver. The explicit list therefore contains the method's
remaining parameters—in this example, only `U`.

Static methods have the same generic rules. They may be called through the
type or through a value. A value receiver is evaluated but is not passed as
`self`:

```nether
struct Factory<T> {
    sample T
}

impl Factory {
    wrap<U>(value U) Factory<U> {
        Factory { sample = value }
    }
}

fn main() {
    let factory = Factory { sample = 1 };
    let value Factory<String> = factory.wrap("ready");
}
```

## Bounds

A generic parameter may have one inline trait bound:

```nether
trait Sound {
    sound(self) String;
}

struct Dog {
    name String
}

impl Dog Sound {
    sound(self) String {
        "woof"
    }
}

fn make_noise<T Sound>(value T) String {
    value.sound()
}

fn main() {
    let dog = Dog { name = "Rex" };
    println(make_noise(dog));
}
```

The bound is checked at each concrete call. It also makes the trait's
methods available inside the generic body.

Bounds may themselves contain generic arguments:

```nether
trait Read<T> {
    read(self) T;
}

struct Boxed<T> {
    value T
}

impl Boxed Read<T> {
    read(self) T {
        self.value
    }
}

fn read_text<T Read<String>>(value T) String {
    value.read()
}

fn main() {
    let text = read_text(Boxed { value = "ready" });
}
```

Trait arguments are part of the implementation identity.
`Read<String>` and `Read<i32>` are different bounds. Because methods are
not overloaded by signature, implementing both applications on one type is
only useful when their resulting method sets do not conflict.

Generic bounds can also be attached to type parameters:

```nether
struct SpeakerBox<T Sound> {
    value T
}

fn open<T Sound>(box SpeakerBox<T>) String {
    box.value.sound()
}
```

The bound is validated after construction inference, so
`SpeakerBox { value = some_value }` is rejected unless the inferred value
type implements `Sound`.

## Multiple requirements through inheritance

Only one bound may be written directly on a generic parameter. Use trait
inheritance to combine several requirements:

```nether
trait Named {
    name(self) String;
}

trait Sound {
    sound(self) String;
}

trait NamedSound Named, Sound {}

fn describe<T NamedSound>(value T) String {
    `${value.name()}: ${value.sound()}`
}
```

Implementing `NamedSound` requires all methods inherited from `Named` and
`Sound`. It also satisfies functions bounded by either parent:

```nether
struct Dog {
    value String
}

impl Dog NamedSound {}

// Methods may be placed in any impl block for Dog.
impl Dog {
    name(self) String { self.value }
}

impl Dog {
    sound(self) String { "woof" }
}

fn only_named<T Named>(value T) String {
    value.name()
}

fn main() {
    only_named(Dog { value = "Rex" });
}
```

Inheritance may preserve or transform generic arguments:

```nether
trait Source<T> {
    read(self) T;
}

trait CachedSource<T> Source<T> {
    cached(self) bool;
}

struct TextSource;

impl TextSource CachedSource<String> {
    read(self) String { "cached text" }
    cached(self) bool { true }
}

fn read_source<T Source<String>>(source T) String {
    source.read()
}

fn main() {
    read_source(TextSource);
}
```

Trait inheritance is static, supports multiple parents, and is
transitive. Cycles and incompatible inherited method signatures are
compile-time errors.

## Default trait methods

Defaults are inherited only when the trait is listed on the `struct` or
`enum` declaration:

```nether
trait Identity<T> {
    identity(self, value T) T {
        value
    }
}

struct Label Identity<String> {
    text String
}

fn main() {
    let label = Label { text = "name" };
    let value String = label.identity("fallback");
}
```

A trait listed on an `impl` block is an explicit implementation.
Every method, including methods with defaults, must then have a
user-written implementation:

```nether
trait Named {
    name(self) String {
        "default"
    }
}

struct User;

// Error: explicit implementations do not inherit Named.name.
// impl User Named {}

impl User Named {}

// This method may be in the same block or any other User impl block.
impl User {
    name(self) String {
        "user"
    }
}
```

Several traits may be implemented in one block, and their methods may
be distributed across any number of blocks:

```nether
trait A { a(self) String; }
trait B { b(self) String; }

struct Both;

impl Both A, B {}
impl Both { a(self) String { "a" } }
impl Both { b(self) String { "b" } }
```

If multiple inherited traits provide different defaults for a method
with the same signature, the concrete type must resolve the ambiguity:

```nether
trait Left {
    value(self) String { "left" }
}

trait Right {
    value(self) String { "right" }
}

struct Resolved Left, Right;

impl Resolved {
    value(self) String {
        "resolved"
    }
}
```

## The `Into<String>` bound

`Into<T>` is a compiler-known generic trait. `Into<String>` enables
generic string conversion and is used by printing and interpolation:

```nether
fn stringify<T Into<String>>(value T) String {
    value.into_string()
}
```

User types implement it explicitly:

```nether
struct User {
    name String
}

impl User Into<String> {
    into_string(self) String {
        self.name
    }
}
```

## Inference model

Inference is deliberately local. At a call site, the compiler:

1. infers owner parameters from a generic method's receiver;
2. applies an explicit `<Type, ...>` list when one was written;
3. uses any expected return type;
4. structurally matches parameter types against argument types;
5. validates every concrete bound;
6. reports an error if any generic parameter remains unknown.

Nether does not perform global constraint solving between unrelated
statements. Add a local type annotation when a payload-free variant or
zero-argument generic call otherwise has no context.

A generic function cannot currently be used as an unspecialized
first-class value:

```nether
fn identity<T>(value T) T {
    value
}

fn main() {
    // Error: no call site exists at which T can be specialized.
    // let function = identity;
}
```

Non-generic named functions and closures remain valid first-class values.
Closures cannot declare their own `<T>` parameter list.

A generic function or method currently cannot be called from inside
*another* still-generic function when the callee's parameter would need
to bind to the caller's own still-abstract type parameter — inference
only binds a generic name to an already-concrete type, never to another
generic placeholder:

```nether
fn identity<T>(value T) T {
    value
}

fn wrap<U>(value U) U {
    // Error: cannot infer generic parameter `T` — `value`'s type is
    // itself the still-abstract `U`, not a concrete type yet.
    identity(value)
}
```

This is independent of [concrete specialization](#concrete-specialization):
specialization resolution itself already accounts for a call site like
this one being monomorphized later (`monomorphization` re-resolves after
substitution, not once at HIR-lowering time), but the call cannot be
*written* at all yet, generic-only or specialized, until this inference
gap is closed.

## Monomorphization and allocation

Generic code is compiled by monomorphization. Each reachable concrete
instantiation receives a specialized function/method body, and no
unresolved generic call remains in MIR or LLVM IR.

For example, calls to `identity(1)` and `identity("one")` produce distinct
`identity<i32>` and `identity<String>`-equivalent native implementations.
There is no source syntax for those generated names.

Allocation behavior belongs to the outer declared type, not its arguments
(language-spec §3/§4.4 — this is the same rule the ordinary, non-generic
case uses; a generic type argument's own category is independent of its
enclosing type's):

- PascalCase structs such as `Boxed<T>` are heap/ARC values;
- lowercase structs such as `pair<T, U>` are stack/value types;
- enums are value types regardless of name casing;
- `Array<T>` is heap/ARC for every `T`.

Classification happens after generic substitution so layouts always see
concrete field and payload types. A generic type argument may itself be
used in the unique-ownership domain at a use site (`Boxed<:T>` is not
directly writable — a struct's own field/generic-argument position doesn't
carry an ownership qualifier in Stage 1; only `let`/parameter/return/
receiver positions do, per language-spec §3).

## Currently unsupported generic features

The following are intentionally absent from the current compiler:

### Rust-style turbofish and partial explicit lists

```nether
// Not supported:
// identity::<i32>(1)
// pair<i32>(1, "text") // pair<T, U> requires both T and U
```

Use `identity<i32>(1)` and provide the complete remaining generic argument
list. Generic arguments on an instance method do not repeat owner
parameters already fixed by its receiver.

### `where` clauses and multiple inline bounds

```nether
fn render<T Named + Sound>(value T) String {
    return `${value.name()}: ${value.sound()}`;
}

fn render_where<T>(value T) String where T Named + Sound {
    return `${value.name()}: ${value.sound()}`;
}
```

Generic bounds never use a colon: `<T: Named>` and `where T: Named` are
syntax errors.

### Associated types

```nether
// Not supported:
// trait Iterator {
//     type Item;
//     next(self): Option<Item>;
// }
```

Use a generic trait such as `Iterator<T>`.

### Concrete specialization

A third `impl` form — no `<...>` after `impl` itself, but a fully
concrete argument list after the target — overrides a generic impl's
method for exactly one instantiation:

```nether
default impl<T> Boxed<T> {
    describe(self) String {
        "generic"
    }
}

impl Boxed<i32> {
    describe(self) String {
        "an int"
    }
}

fn main() {
    println(Boxed { value = 1 }.describe());       // "an int"
    println(Boxed { value = "x" }.describe());      // "generic"
}
```

The concrete impl's method may instead have no generic counterpart at
all — a method that only exists for that one instantiation:

```nether
impl Boxed<i32> {
    doubled(self) i32 {
        self.value * 2
    }
}
```

Resolution always prefers an exact match on the owner's concrete
arguments over the generic fallback, and — since Nether monomorphizes
everything — this applies uniformly whether the call site already has a
concrete receiver or is itself still inside another generic function
that later gets monomorphized for a matching concrete type.

A specialization's method must have the same signature as the generic
version when both exist (only the body may differ), and that generic
fallback must be declared with `default impl`. An ordinary generic `impl`
cannot be overridden. Specialization
overrides behavior, not the type a caller sees, since a still-generic
caller can only ever check against the one generic signature. Static
methods and trait conformance cannot yet be specialized — a
concrete impl block may only contain `self`/`mut self` methods and may
not itself carry a `SomeTrait` list.

A blanket `impl` — one whose target is itself a generic parameter
(`impl<T> T SomeTrait { ... }`) rather than a declared struct/enum —
remains unsupported; `impl`'s target must always name one specific
`struct`/`enum`.

### Const generics

```nether
// Not supported:
// struct Buffer<T, const N usize> { ... }
```

Generic arguments are types only.

### Higher-kinded types and generic type constructors

Parameters such as `F<T>` where `F` itself is a generic type constructor
are not representable.

### Generic type aliases

Plain, non-generic type aliases are supported (`type color = (u32, u32,
u32);` — language-spec §4.3), but a generic alias is not:

```nether
// Not supported:
// type Names<T> = Array<T>;
```

### Dynamic generic traits

Traits are bounds only. There is no `any Trait` yet (not
implemented — Stage 3), no trait-typed local variable, no
heterogeneous `Array<Trait>`, no vtable, no runtime trait cast.
Dispatch is always resolved statically and then monomorphized.
