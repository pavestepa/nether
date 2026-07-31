# Generics in Nether

This document is a practical guide to the generic features currently
implemented by Nether. The normative language definition remains
[`spec/language-spec.md`](spec/language-spec.md); this guide focuses on
complete examples, inference behavior, interface constraints, and current
limitations.

## Feature overview

| Feature | Status |
| --- | --- |
| Generic functions | Supported |
| Generic methods | Supported |
| Generic structs and tuple structs | Supported |
| Generic enums | Supported |
| Generic interfaces | Supported |
| Bounds such as `T: Sound` and `T: Convert<String>` | Supported |
| Generic interface inheritance | Supported |
| Multiple effective bounds through interface inheritance | Supported |
| Local type-argument inference | Supported |
| Expected-return-type inference | Supported |
| Monomorphization | Supported |
| Explicit call arguments such as `f<i32>()` | Supported |
| `where` clauses | Not supported |
| Multiple inline bounds such as `T: A + B` | Not supported |
| Associated types | Not supported |
| Specialization and overlapping implementations | Not supported |
| Blanket implementations | Not supported |
| Const generics | Not supported |
| Higher-kinded types | Not supported |
| Generic type aliases | Not supported |
| First-class unspecialized generic functions | Not supported |
| Dynamic interface dispatch | Deliberately not supported |

## Generic functions

Declare generic parameters after the function name:

```nether
fn identity<T>(value: T): T {
    value
}

fn choose<T>(condition: bool, left: T, right: T): T {
    if condition {
        left
    } else {
        right
    }
}

fn main() {
    let number: i32 = identity(42);
    let text: String = identity("Nether");
    let selected: bool = choose(true, true, false);
}
```

When call arguments are omitted, the compiler must infer every generic
parameter. Inference uses argument types and, when available, the expected
return type:

```nether
fn empty<T>(): Option<T> {
    Option.None
}

fn main() {
    // T is inferred as i32 from the declared result type.
    let value: Option<i32> = empty();
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
fn opaque<T>(value: i32): i32 {
    value
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
type Boxed<T> {
    value: T
}

type Pair<T, U>(T, U);

fn main() {
    let number = Boxed { value: 42 };
    let text = Boxed { value: "ready" };
    let pair = Pair(number, text);

    let left: i32 = pair.0.value;
    let right: String = pair.1.value;
}
```

Constructors infer their type arguments from their fields. A type annotation
may instead provide the expected arguments:

```nether
fn main() {
    let number: Boxed<i32> = Boxed { value: 42 };
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

fn unwrap_or<T>(value: Maybe<T>, fallback: T): T {
    match value {
        Some(item) => item,
        None => fallback,
    }
}

fn main() {
    let result: Outcome<i32, String> = Outcome.Ok(7);
    let value = unwrap_or(Maybe.Some("ready"), "missing");
}
```

A payload-free generic variant such as `Maybe.None` needs context:

```nether
fn main() {
    let known: Maybe<i32> = Maybe.None;

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
type Boxed<T> {
    value: T
}

impl Boxed {
    new(value: T): Boxed<T> {
        Boxed { value }
    }

    get(self): T {
        self.value
    }
}

fn main() {
    let box = Boxed.new("text");
    let value: String = box.get();
}
```

This Rust-like form is not Nether syntax:

```nether
// Not supported:
// impl<T> Boxed<T> { ... }
```

Methods may introduce additional generic parameters:

```nether
type Boxed<T> {
    value: T
}

impl Boxed {
    replace<U>(self, value: U): Boxed<U> {
        Boxed { value }
    }
}

fn main() {
    let number = Boxed { value: 1 };
    let text: Boxed<String> = number.replace<String>("one");
}
```

For an instance method, owner arguments such as `Boxed<i32>` are already
known from the receiver. The explicit list therefore contains the method's
remaining parameters—in this example, only `U`.

Static methods have the same generic rules. They may be called through the
type or through a value. A value receiver is evaluated but is not passed as
`self`:

```nether
type Factory<T> {
    sample: T
}

impl Factory {
    wrap<U>(value: U): Factory<U> {
        Factory { sample: value }
    }
}

fn main() {
    let factory = Factory { sample: 1 };
    let value: Factory<String> = factory.wrap("ready");
}
```

## Bounds

A generic parameter may have one inline interface bound:

```nether
interface Sound {
    sound(self): String;
}

type Dog {
    name: String
}

impl Dog: Sound {
    sound(self): String {
        "woof"
    }
}

fn make_noise<T: Sound>(value: T): String {
    value.sound()
}

fn main() {
    let dog = Dog { name: "Rex" };
    println(make_noise(dog));
}
```

The bound is checked at each concrete call. It also makes the interface's
methods available inside the generic body.

Bounds may themselves contain generic arguments:

```nether
interface Read<T> {
    read(self): T;
}

type Boxed<T> {
    value: T
}

impl Boxed: Read<T> {
    read(self): T {
        self.value
    }
}

fn read_text<T: Read<String>>(value: T): String {
    value.read()
}

fn main() {
    let text = read_text(Boxed { value: "ready" });
}
```

Interface arguments are part of the implementation identity.
`Read<String>` and `Read<i32>` are different bounds. Because methods are
not overloaded by signature, implementing both applications on one type is
only useful when their resulting method sets do not conflict.

Generic bounds can also be attached to type parameters:

```nether
type SpeakerBox<T: Sound> {
    value: T
}

fn open<T: Sound>(box: SpeakerBox<T>): String {
    box.value.sound()
}
```

The bound is validated after construction inference, so
`SpeakerBox { value: some_value }` is rejected unless the inferred value
type implements `Sound`.

## Multiple requirements through inheritance

Only one bound may be written directly on a generic parameter. Use interface
inheritance to combine several requirements:

```nether
interface Named {
    name(self): String;
}

interface Sound {
    sound(self): String;
}

interface NamedSound: Named, Sound {}

fn describe<T: NamedSound>(value: T): String {
    `${value.name()}: ${value.sound()}`
}
```

Implementing `NamedSound` requires all methods inherited from `Named` and
`Sound`. It also satisfies functions bounded by either parent:

```nether
type Dog {
    value: String
}

impl Dog: NamedSound {}

// Methods may be placed in any impl block for Dog.
impl Dog {
    name(self): String { self.value }
}

impl Dog {
    sound(self): String { "woof" }
}

fn only_named<T: Named>(value: T): String {
    value.name()
}

fn main() {
    only_named(Dog { value: "Rex" });
}
```

Inheritance may preserve or transform generic arguments:

```nether
interface Source<T> {
    read(self): T;
}

interface CachedSource<T>: Source<T> {
    cached(self): bool;
}

type TextSource;

impl TextSource: CachedSource<String> {
    read(self): String { "cached text" }
    cached(self): bool { true }
}

fn read_source<T: Source<String>>(source: T): String {
    source.read()
}

fn main() {
    read_source(TextSource);
}
```

Interface inheritance is static, supports multiple parents, and is
transitive. Cycles and incompatible inherited method signatures are
compile-time errors.

## Default interface methods

Defaults are inherited only when the interface is listed on the `type` or
`enum` declaration:

```nether
interface Identity<T> {
    identity(self, value: T): T {
        value
    }
}

type Label: Identity<String> {
    text: String
}

fn main() {
    let label = Label { text: "name" };
    let value: String = label.identity("fallback");
}
```

An interface listed on an `impl` block is an explicit implementation.
Every method, including methods with defaults, must then have a
user-written implementation:

```nether
interface Named {
    name(self): String {
        "default"
    }
}

type User;

// Error: explicit implementations do not inherit Named.name.
// impl User: Named {}

impl User: Named {}

// This method may be in the same block or any other User impl block.
impl User {
    name(self): String {
        "user"
    }
}
```

Several interfaces may be implemented in one block, and their methods may
be distributed across any number of blocks:

```nether
interface A { a(self): String; }
interface B { b(self): String; }

type Both;

impl Both: A, B {}
impl Both { a(self): String { "a" } }
impl Both { b(self): String { "b" } }
```

If multiple inherited interfaces provide different defaults for a method
with the same signature, the concrete type must resolve the ambiguity:

```nether
interface Left {
    value(self): String { "left" }
}

interface Right {
    value(self): String { "right" }
}

type Resolved: Left, Right;

impl Resolved {
    value(self): String {
        "resolved"
    }
}
```

## The `Into<String>` bound

`Into<T>` is a compiler-known generic interface. `Into<String>` enables
generic string conversion and is used by printing and interpolation:

```nether
fn stringify<T: Into<String>>(value: T): String {
    value.into_string()
}
```

User types implement it explicitly:

```nether
type User {
    name: String
}

impl User: Into<String> {
    into_string(self): String {
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
fn identity<T>(value: T): T {
    value
}

fn main() {
    // Error: no call site exists at which T can be specialized.
    // let function = identity;
}
```

Non-generic named functions and closures remain valid first-class values.
Closures cannot declare their own `<T>` parameter list.

## Monomorphization and allocation

Generic code is compiled by monomorphization. Each reachable concrete
instantiation receives a specialized function/method body, and no
unresolved generic call remains in MIR or LLVM IR.

For example, calls to `identity(1)` and `identity("one")` produce distinct
`identity<i32>` and `identity<String>`-equivalent native implementations.
There is no source syntax for those generated names.

Allocation behavior belongs to the outer declared type, not its arguments:

- PascalCase types such as `Boxed<T>` are heap/ARC values;
- camelCase types such as `pair<T, U>` are stack/value types;
- enums are value types;
- `Array<T>` is heap/ARC for every `T`.

Classification happens after generic substitution so layouts always see
concrete field and payload types.

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
// Not supported:
// fn render<T>(value: T): String where T: Named, T: Sound { ... }
// fn render<T: Named + Sound>(value: T): String { ... }
```

Define a child interface inheriting all required parents instead.

### Associated types

```nether
// Not supported:
// interface Iterator {
//     type Item;
//     next(self): Option<Item>;
// }
```

Use a generic interface such as `Iterator<T>`.

### Specialization, blanket impls, and conditional impls

```nether
// Not supported:
// impl<T: Sound> Boxed<T>: Into<String> { ... }
// impl<T> T: SomeInterface { ... }
```

An `impl` targets one declared nominal type or enum. The target's own
generic parameters are in scope, but implementations cannot introduce a
new conditional or blanket parameter list.

### Const generics

```nether
// Not supported:
// type Buffer<T, const N: usize> { ... }
```

Generic arguments are types only.

### Higher-kinded types and generic type constructors

Parameters such as `F<T>` where `F` itself is a generic type constructor
are not representable.

### Generic type aliases

Nether does not currently have type aliases, generic or otherwise:

```nether
// Not supported:
// type Names<T> = Array<T>;
```

### Dynamic generic interfaces

Interfaces are bounds only. There is no `dyn Interface`, interface-typed
local variable, heterogeneous `Array<Interface>`, vtable, or runtime
interface cast. Dispatch is always resolved statically and then
monomorphized.
