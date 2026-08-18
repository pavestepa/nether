# The Nether Language — Canonical Reference

Status: **canonical spec** — this document is the single source of truth for
the Nether language as a whole. It supersedes every earlier specification,
memory model, syntax experiment, ownership design, and example file that
predates it, including anything under a stray `.nt` file. Where an example
elsewhere in the repo disagrees with this document, this document wins.

This document describes the **full target language**, not only what is
implemented today. Every section below is marked with its implementation
status:

- **[Stage 1]** — implemented now.
- **[not yet implemented — Stage N]** — syntax/semantics are specified here
  so the whole language is coherent on paper, but the compiler does not
  implement this yet; `N` names the stage in
  [`../architecture/roadmap.md`](../architecture/roadmap.md) expected to add
  it.
- **[current behavior, unchanged by Stage 1]** — this area of the language
  keeps its pre-rewrite MVP syntax/semantics for now; it is not part of the
  new ownership model and has not been touched by this rewrite. It will be
  revisited in a later stage.

This is not a tutorial. It is written the way a language reference is
written: one section per concept, precise enough that the parser, resolver,
and type checker can each be implemented directly from it.

---

## 1. Design summary

Nether combines Rust-like static typing and traits with Swift-like ARC
reference semantics and Rust-like unique ownership, in one coherent type
system — not one or the other.

| Concern | Nether |
|---|---|
| Memory management | **both** ARC (for `T`) and unique ownership + borrow checking (for `:T`) — the programmer chooses per type/binding, not the language globally. Stage 2 enforces moves, borrow exclusivity, returned-reference origins, callable summaries, and ordinary last-use shortening |
| Polymorphism | traits, static dispatch by default; existential (`any Trait`) and opaque (`some Trait`) types for the rest |
| Generics | monomorphized in release builds; dictionary-safe bodies use witness-table dispatch in development builds, with monomorphized fallback for layout-dependent bodies |
| Concurrency | async/await over a Tokio-backed runtime, plus raw OS threads *(not yet implemented — Stage 4)* |
| Unsafe code | explicit `unsafe`, raw pointers, C ABI FFI *(not yet implemented — Stage 5)* |
| Lifetimes | no explicit lifetime syntax; inferred origins, callable summaries, and loop/local-closure-sensitive last-use shortening power borrow checking |

Everything below assumes these are the only permanent design constraints:
no explicit lifetime syntax, no `dyn` keyword, no `::` path separator.
Rust-style implicit tail-expression function returns are rejected (§8.3),
so non-unit functions require an explicit `return`. Everything
else in the "not yet implemented" column above is a staging decision, not
a design decision — see the roadmap.

---

## 2. Lexical structure

### 2.1 File extension **[Stage 1]**

Nether source files use exactly the `.nr` extension. The legacy `.nt`
extension used during early development is rejected with a diagnostic
explaining how to migrate (rename the file). The package manifest is
`Nether.toml` — a TOML file, not a `.nr` source file *(package manifest
itself: not yet implemented — Stage 6)*.

### 2.2 Comments **[current behavior, unchanged by Stage 1]**

```
// line comment
/// doc comment (attaches to the following item)
```

Block comments (`/* ... */`) remain a documented future extension, not yet
part of the grammar.

### 2.3 Identifiers and casing **[Stage 1 changes the mechanism, not the convention]**

Identifiers are ASCII `[A-Za-z_][A-Za-z0-9_]*`. Casing is still
load-bearing, but its role has changed from the pre-rewrite MVP:

- **Before this rewrite:** a type declaration's casing directly *determined*
  whether it was heap/ARC or stack/inline — casing was the allocation
  mechanism itself.
- **As of Stage 1:** allocation category is determined by how a type is
  actually defined (a `struct`/heap type vs. a primitive/inline type) and by
  the explicit ownership sigil (§4) at each use site. Casing becomes a
  **validated naming convention**, checked against a type's *resolved*
  representation category — see §8.

### 2.4 Literals **[current behavior, unchanged by Stage 1]**

- Integers: `123`, `0`.
- Floats: `1.0`, `0.5`.
- Booleans: `true`, `false`.
- Char: `'a'`.
- Plain string: `"text"`.
- Template string: `` `text ${expr} text` `` — interpolation via `${...}`,
  multiline supported, the `${...}` body is parsed as an ordinary Nether
  expression.
- Arrays: `[]` (empty), `[1, 2, 3]` (literal elements). The expected type
  selects the representation: `Array<T>` builds a growable runtime array,
  while `{T, N}` builds an inline fixed array and checks the literal length.

### 2.5 Semicolons **[current behavior, unchanged by Stage 1]**

Semicolons are still required statement terminators today; newline-aware
optional semicolons are specified for the language but not yet implemented
(unstaged — tracked in the roadmap's syntax-cleanup bucket).

### 2.6 No arbitrary block expressions **[Stage 1]**

A bare `{ ... }` is **not** an expression in Nether. Braces are only ever
attached to a known construct — `fn` bodies, `if`/`match`/`while`/`for`/
`loop` bodies, `impl`/`trait` bodies, `unsafe` blocks. This also
simplifies disambiguation with future inline-array literals (§2.4).

```
let x = {
    let y = 1;
    y
};   // compile error — bare blocks are not expressions
```

---

## 3. The four fundamental value forms **[Stage 1 grammar; move checking Stage 2]**

This is the central, distinguishing concept of Nether's type system. Every
type belongs to one of two **representation categories** — heap/reference or
inline/value — and every *use* of a type carries one of two **ownership
qualifiers** — ordinary (ARC/copy) or uniquely owned. The two axes are
orthogonal:

| | Ordinary | Uniquely owned |
|---|---|---|
| **Heap/reference category** (`T`, e.g. `User`) | `T` — ARC reference. Assignment copies the reference (aliasing); mutation needs `mut` permission but not exclusivity. Atomic refcounts are deferred to Stage 6. | `:T` — uniquely owned, unrefcounted heap value. Stage 2 enforces moves, live-borrow exclusivity, returned-reference origins, callable summaries, and last-use shortening. |
| **Inline/value category** (`t`, e.g. `i32`, `color`) | `t` — ordinary inline value. Implicitly copyable, normal value semantics. | `:t` — uniquely owned inline value. Same machine representation as `t`; move-only *semantically* — use-after-move is a compile error *(enforced — Stage 2's move checker, as above)*. |

The representation category comes from **how the type is defined**:
primitives (`i32`, `bool`, `f64`, ...), tuples, and enums are always
inline; `struct` declarations and other heap types (`String`, closures) are
always heap/reference. The ownership qualifier — the leading `:` — is
chosen **at each use site** (a `let` binding, a function parameter, a return
type, a receiver), independent of how the type itself is declared.

The leading `:` is part of the type/value syntax — not generic punctuation.
It appears wherever a type or `self` receiver is described: type
annotations, struct literals, function parameters, receivers, return types,
and reference chains. **Normal expressions never gain special syntax merely
because a value is owned** — `destroy(value)` stays `destroy(value)`, never
`destroy(:value)`. The compiler infers move/consume behavior from the
*declared type* at the use site, not from marking the expression.

```
let a i32 = 10;
let b = a;
println(a);   // ok — i32 is ordinary inline, copied
println(b);   // ok

let c: i32 = 10;
let d = c;
println(d);   // ok
println(c);   // compile error as of Stage 2 — "use of a value after it was moved"
```

### 3.1 Reference chains **[Stage 2; deeper member-access chains remain limited — see below]**

References only exist inside the unique/owned domain — there is no bare
`&T` without a leading `:`. `:&T` (shared borrow) and `:&mut T` (exclusive
borrow) are represented **structurally** in the AST and type system (nested
wrapper nodes), never as strings, so deeper chains (`:&&T`, `:&&mut T`)
compose structurally, though only a single layer is peeled where field/
method access is resolved through one today (Stage 2, slice 2) — a doubly-
nested chain type-checks but isn't yet usable for member access.

**There is still no `&expr` operator anywhere in the grammar.** A
reference is never *formed* by the caller writing anything at an
expression's use site — a `:&T`/`:&mut T` **parameter** is satisfied by
passing an already-owned (`:T`) local by its bare name, the same
implicit-by-declared-type mechanism an ordinary ARC parameter already
uses (§8.1 — no extra caller-side marker, unlike `mut`). Only a bare local
qualifies (there's no other way to name a place to borrow); `:&mut T`
additionally requires the binding be `mut`. Within one call's argument
list, the same local can't be borrowed `:&mut` twice, or `:&` and `:&mut`
together. The same origin machinery also tracks references that outlive a
call because they are returned or stored in a local, as described below.

Reference codegen covers heap and inline categories. A heap reference reuses
the object pointer ABI without retaining it. An inline reference points at an
addressable local stack slot; HIR/MIR contain explicit address-of and implicit
value-context dereference operations. A mutable inline reference writes
through to that same originating slot.

**Calling a method through a `:&T`/`:&mut T` receiver works** (Stage 2,
slice 3), not just reading a field: a borrowing self-form
(`: &self`/`: &mut self`) resolves the same way it would on a bare owned
value, since a reference and an owned value both resolve into the same
`Owned`-domain method set (`ReceiverDomain::of_receiver_ty`). A `: self`
(consuming) method is specifically rejected when found through a
reference receiver — a dedicated diagnostic, not a resolution-time
distinction, since consuming a value through a mere borrow would be
unsound. `: &mut self` additionally requires the receiver be exclusive
(`:&mut T`, or a `mut`-bound owned local), the same mutability check a
`mut self` ARC method already used.

**Stored borrows use last-use shortening:**
`let view: &User = owned_user;` and `let view: &mut User = owned_user;`
record the owned local as their origin, enforce shared/exclusive access, and
release that restriction after `view`'s last ordinary use. A loop keeps the
borrow live through the loop expression. A locally bound closure keeps each
captured borrow live through the closure's last use; closure values without a
provable final local use use lexical scope as the safety fallback. The same
rules apply to inline referents.

**Returned references are origin-checked and propagated through function calls.** A
return traced to exactly one incoming reference parameter is accepted;
returning a reference borrowed from a local owned value is rejected, as is
an `if`/`match` result that may originate from multiple parameters. Named
functions carry parameter-index origin summaries computed to a fixed point.
Consequently, origin survives arbitrary named-function call chains and a
returned reference may be stored in `let`; that binding keeps the ultimate
owned source borrowed until the returned binding's last use. Instance/static
methods carry summaries too (`self` is a distinct origin), while closure
summaries preserve both parameter and captured origins.

### 3.2 Runtime representation of `:T`

`:T` uses a dedicated unrefcounted allocation containing only payload size
and drop metadata. Moves transfer its sole pointer without retaining and
clear the moved-from slot; final destruction calls `unique_free`. Converting
`:T -> T` moves the payload bytes into a fresh ARC allocation without cloning
its fields. `:t` has the same machine representation as `t`, restricted by
move semantics at the type-checker level.

---

## 4. Types

### 4.1 Primitives **[current behavior, unchanged by Stage 1]**

```
bool char
i8 i16 i32 i64
u8 u16 u32 u64
usize isize
f32 f64
```

All primitives are inline/value types (§3): copied on ordinary assignment
(`t`), move-only when explicitly owned (`:t`).

### 4.2 Struct declarations: `struct` **[Stage 1]**

`struct` declares a heap/reference-category type — a struct, tuple-struct,
or unit-like type. This is a change from the pre-rewrite MVP, which
overloaded the `type` keyword for this; `type` is now reserved exclusively
for alias declarations (§4.3), so a struct/alias ambiguity never arises.

```
struct Dog {
    name String
}

struct Point(i32, i32);   // tuple-struct
struct EmptyType;         // unit-like — no braces, no parens, one semicolon
```

Field declarations no longer use a colon between name and type (`name
String`, not `name: String`) — this mirrors the same colon-means-ownership
rule used everywhere else in the grammar (§3): a bare field type is
ordinary/ARC, and there is currently no syntax for an owned field (owned
values do not yet have a place in struct layout beyond what §3 already
covers at binding/parameter granularity).

### 4.3 Type aliases: `type` **[Stage 1, alias declarations only]**

`type Name = TypeExpr;` declares a type alias. This is a genuinely new
construct — the pre-rewrite compiler had no alias mechanism at all.

```
type color = (u32, u32, u32);
```

An alias must not lie about the representation category of its target — see
§8's casing validation.

An **ownership-qualified alias declaration**, `type: Name = :TypeExpr;`, is
specified for the full language but not yet implemented in Stage 1; using
this form today produces a clear "not yet supported" diagnostic rather than
being silently mis-parsed.

### 4.4 Heap vs. inline representation category **[Stage 1]**

The representation category comes from how a type is actually defined, not
from casing:

| Kind | Category |
|---|---|
| `struct` (any name) | heap, ARC (as `T`) / unique-owned (as `:T`) |
| primitive (`i32`, `bool`, ...) | inline (as `t`) / unique-owned inline (as `:t`) |
| `enum` (any name) | inline — tag + flat payload, always, regardless of name casing (§8's exemption) |
| tuple `(A, B, ...)` | inline — each element follows its own type's category |
| `String`, `Array<T>` | heap, ARC |

### 4.5 Weak references **[current behavior, unchanged by Stage 1]**

`weak T` is only meaningful where `T` is a heap/ARC type. A `weak T` does
not keep its referent alive; there is no cycle collector.

### 4.6 Built-in heap types **[current behavior, unchanged by Stage 1]**

`String` and `Array<T>` are heap-allocated, ARC-managed, declared in the
bundled prelude (`stdlib/`). `Array<T>`'s core operations (`push`, `pop`,
`len`, indexing) live in the runtime; everything else is an ordinary
`impl<T> Array<T> { ... }` extension. Distinguishing a growable-`Vector`-style
literal from the fixed-size inline-array literal `{T, N}` of the full target
grammar is not yet implemented (Stage 3).

### 4.7 Tuples **[current behavior, unchanged by Stage 1]**

```
let pair = (1, "a");
let (x, y) = pair;
println(pair.0);
```

---

## 5. Naming/representation validation **[Stage 1]**

PascalCase names normally indicate heap/reference-category types
(`struct User`); lowercase names normally indicate inline-category types
(primitives, and any future lowercase-named inline aliases). This is now a
**validation rule checked against a type's resolved representation
category**, not the mechanism that determines it (§3, §4.4).

**A type alias must not lie about its target's representation category.**
After resolving `type color = SomeType;`, if `SomeType` resolves to a
heap/reference-category type but `color` is lowercase (or vice versa), that
is a compile error. Struct/enum declarations are not separately checked
against this rule — their own casing *is* definitionally where their
category comes from (checking them against themselves would be a
tautology); only aliases can "lie."

**Enums and tuples are exempt.** Regardless of name casing, an `enum` and an
anonymous tuple type are always inline-category (§4.4) — this is carried
forward unchanged from the pre-rewrite compiler specifically so that
`Option`/`Result` (PascalCase, structurally inline) do not fail validation.
Without this carve-out, the entire bundled stdlib would be rejected on day
one.

`#[allow_pascal_case]` is an implemented narrow escape hatch for exceptional
compiler/library types whose physical representation doesn't match the
naming convention (e.g. a future multi-word `Vector` handle) — **[not yet
implemented — Stage 3's first slice]**. It disables only the naming diagnostic; it does
not change ARC behavior, ownership, Copy behavior, ABI, allocation, thread
safety, or borrow semantics.

---

## 6. Visibility

Everything is private by default. Public API is always explicit with `pub`:
`pub mod`, `pub use`, `pub struct`, `pub enum`, `pub type`, `pub trait`, and
`pub fn`. Named struct fields and both instance and static methods follow the
same rule and require their own `pub`; making the containing type public does
not make its fields or methods public. Enum variants inherit the visibility of
their enum. The former `private` modifier and underscore-derived visibility
are not part of the language.

A private declaration remains usable throughout its defining file. Crossing a
file/module boundary requires the referenced declaration to be public. A
`pub use` explicitly re-exports its imported name; a plain `use` only binds it
inside the importing module.

---

## 7. Variables and bindings **[Stage 1]**

```
let a = 3;                        // inferred, ordinary inline
let mut a = 3;                    // mutable binding

let user User = User.new(...);    // explicit ARC type — space, no colon
let user: User = :User { ... };   // explicit owned type — colon, owned literal

let count i32 = 10;                // explicit inline type — space, no colon
let count: i32 = 10;               // explicit owned inline type
```

`mut` before the binding name (`let mut a = ...`) grants mutation
permission on the binding itself, for both ARC and owned/inline forms —
this placement is unchanged from before the rewrite.

### 7.1 ARC mutability is not exclusivity **[current behavior, unchanged by Stage 1]**

```
let mut a User = ...;
let b = a;
a.name = "Bob";
// b now also observes "Bob" — a and b alias the same object.
```

`mut` on an ARC binding means "this binding may mutate," not "this binding
is the only reference." This is intentionally different from `:&mut T`,
where Stage 2 enforces exclusivity for call arguments and stored/returned
heap/inline borrows. Ordinary local borrows end after their last use, loop
borrows after the loop expression, and locally captured borrows after their
closure's last use; non-local closure lifetimes use a lexical fallback.

---

## 8. Functions, parameters, receivers **[Stage 1 grammar; `d`/`e` callable as of Stage 2]**

### 8.1 Parameter forms

```
fn foo(
    a Animal,        // ordinary ARC parameter
    b mut Animal,     // ARC parameter with mutation permission — note: `mut` comes AFTER the name here
    c: Animal,        // uniquely owned, moved into the function
    d: &Animal,        // borrow from an owned value — see §3.1 for exactly what "callable" means here
    e: &mut Animal      // exclusive mutable borrow from an owned value
) {
}
```

The `mut` placement is deliberately asymmetric with `let mut`: `let mut a`
puts `mut` *before* the name (mutating the binding), while `b mut Animal`
puts `mut` *between* the name and type (an ARC parameter with mutation
permission). This is intentional, not an inconsistency to "fix."

An ordinary ARC parameter (`a Animal`) semantically owns a strong ARC
reference during the call — a straightforward implementation performs
retain/release, but the optimizer may eliminate redundant pairs when the
reference is only temporarily borrowed. This must never be observable.

### 8.2 Return types

```
fn create_user() User { ... }        // ARC return — no colon
fn create_user(): User { ... }       // owned return — colon
fn count() i32 { return 10; }        // inline return
fn count(): i32 { return 10; }       // owned inline return
fn get_name(user: &User): &String { return &user.name; }   // borrowed return
```

### 8.3 No implicit tail-expression return **[implemented — Stage 1]**

The function/method body's own tail expression is not implicitly returned.
It is checked and evaluated as a discarded expression; a non-unit function
must reach an explicit `return`. Ordinary block expressions, `if` branches,
`match` arms, and closure bodies keep their value-producing tails.

```
fn foo() i32 {
    return 10;
}
```

There is no `return:` operator — an owned return naturally still contains a
colon because the *value* is an owned construction:

```
return :User { ... };   // parses as return (:User { ... }), not a distinct `return:` form
```

### 8.4 Self receivers **[Stage 1 grammar; overload resolution Stage 2]**

```
fn get_name(self) String { ... }              // ARC receiver
fn set_name(mut self, name String) { ... }    // mutable ARC receiver
fn destroy(: self) { ... }                    // owned, consuming receiver
fn get_name(: &self): &String { ... }         // borrowed unique receiver
fn set_name(: &mut self, name String) { ... } // mutable borrowed unique receiver
```

Receiver ownership mode is part of method overload resolution — a trait or
impl may declare both an ARC-domain and an owned-domain variant of the same
method name (`get_name(self) String` alongside
`get_name(: &self): &String`), resolved at each call site by the receiver's
actual domain (with a `Static` fallback so a static method stays callable
through a value). One is never synthesized from the other via an implicit
conversion (§9). A trait requirement itself still names exactly one domain
per method (an implementing type's overload set can be wider than what any
one trait requires, but a single trait method isn't yet dual-domain-
overloadable on its own) — dual-domain trait *requirements*, and generic-
bound (`<T Sound>`) dispatch across both domains, remain unstaged.

### 8.5 Static methods **[current behavior, unchanged by Stage 1]**

A method with no `self`/receiver parameter is static, called through the
type (`User.new(...)`), not through an instance.

### 8.6 Basic ownership-domain type checking **[Stage 1]**

Independent of move tracking, the type checker enforces ordinary type
compatibility across the ownership dimension: passing an ARC-typed (`T`)
value where an owned parameter (`:T`) is declared, or vice versa, is a type
mismatch and rejected — this is normal type-checking, not borrow-checking.

### 8.6.1 Move checking **[Stage 2]**

A `Type::Unique(_)`-typed local binding — `:T` or `:t` — may be read as a
whole value at most once between the point it's live and the point it's
moved. Flow-sensitive, tracked per local binding, no lifetime/region
inference required (see the move-checking module's own docs,
`compiler/typecheck/src/check/checker.rs`, for exactly what counts as a
move vs. a read-through):

- A bare local reference used as an rvalue (`let`/assignment RHS, a call
  argument, a `return` operand, a `: self`-consuming method receiver)
  moves it.
- Reading a field/element through it (`dog.name`), or calling a
  `: &self`/`: &mut self` (borrowing) method on it, requires it to still be
  live but does **not** consume it — struct/tuple fields are always
  ordinary-typed (§4.2), never themselves `Type::Unique`.
- Reassigning a `mut` binding resets its move-state (a fresh value now
  lives there).
- `if`/`match`: each arm is checked independently from the same pre-branch
  state; a value moved on any one live-reaching (non-diverging) arm counts
  as moved after the merge — a "possibly moved" use is still rejected, not
  only a "definitely moved" one.
- `while`/`for`/`loop`: a value moved unconditionally inside the body is
  rejected even if the *textual* reuse appears to come first, since the
  loop may run more than once — a `let` declared *inside* the loop body is
  unaffected (each pass gets a fresh binding).
- A closure literal moves any free `Type::Unique` variable it references,
  at the closure's own position — matching a `move` closure's semantics
  (captured at creation, regardless of whether/when the closure is later
  called) — independent of whether the closure is ever invoked.

Move diagnostics carry two labels: where the value was moved, and where it
was used again. Borrow exclusivity blocks moves and direct mutation while a
stored borrow is live. Returned-reference origins propagate through named
functions, methods, and closures; loop and locally captured uses participate
in last-use shortening.

### 8.7 Variadic parameters **[Stage 1]**

A trailing `name ...Type` parameter (no colon) accepts zero or more
trailing call arguments of `Type`, collected into an `Array<Type>` visible
under `name` inside the function body:

```
fn show(args ...String) {
    for arg in args {
        println(arg);
    }
}
```

Only one variadic parameter is allowed per function, and it must be the
last parameter. It lowers to a hidden const-generic fixed array whose length
is the number of trailing arguments at each call site; it is not a growable
`Array<Type>` inside the callee.

---

## 9. Universal `to(value)` conversion **[all transitions implemented; structural `Clone` limitations below]**

The full language specifies a universal, explicit domain-conversion
operation (`to(value)`, or `to<T>(value)` with an explicit target) covering
all four `:T -> T`, `T -> :T`, `:t -> t`, `t -> :t` transitions. No implicit
ownership-domain adaptation ever happens at a call site — the programmer
must call `to()` explicitly when crossing domains.

`:T -> T`, `:t -> t`, and `t -> :t` are implemented (Stage 2) as a
compiler builtin (`println`/`print`'s own mechanism — no user-overridable
declaration exists to shadow it). The target may be inferred from context
(`let arc_dog Dog = to(owned_dog);`) or written explicitly
(`to<Dog>(owned_dog)`, `to<:i32>(n)`). Inline transitions are representation-
free relabeling. `:T -> T` explicitly promotes the unrefcounted allocation
into ARC by moving its payload without cloning fields. It consumes the source
the same way passing it to another owned parameter would (§8.6.1); `to(n)`
for an ordinary inline `n` does not consume `n`.

**`T -> :T` is implemented for structurally cloneable user structs.** The
source type must explicitly implement the compiler-known `Clone` marker
(`Clone struct Dog { ... }`). The operation allocates a distinct unique outer
object, copies its fields, and grants copied ARC/weak fields independent
ownership credits. Direct unique heap fields are recursively cloned when their
inner types also implement `Clone`; missing implementations and cyclic clone
graphs receive a diagnostic rather than duplicating unique pointer bits. A
type can replace synthesis with `clone(: &self): T` returning `:T`; `to<:T>`
dispatches directly to that method and validates its receiver, parameters, and
return type.

---

## 10. Copy and Clone **[structural Clone slice implemented — Stage 3]**

`Copy` is reserved for inline-category values; heap/reference-category
types may implement `Clone` but must not implement `Copy` (assignment of an
ordinary heap type already means "copy the ARC reference," not "deep-copy
the object" — conflating the two would be a silent correctness hazard).
The compiler-known `Clone` marker synthesizes the structural clone used by
`T -> :T`. Prefix derivation syntax (traits joined with `+` directly before a
declaration, not Rust's `#[derive(...)]`) lowers into the ordinary trait opt-in
list. Thus `Clone struct Dog` works today, and `Clone + Eq + Hash struct Dog`
combines all three compiler-derived operations. `Eq` recursively compares
supported fields for both ARC and unique values and diagnoses unsupported or
cyclic derive graphs. `Hash` feeds the same structural family into the
compiler builtin `hash(value) u64`, using a deterministic unkeyed fold. Integer,
bool, char, `String`, tuple, nested `Hash`, and unique fields are supported;
strings hash their UTF-8 bytes. Floating-point fields remain rejected until
`Eq`/hash semantics for signed zero and NaN are specified. Generic user-defined
clone overrides remain Stage 3 work.

---

## 11. Struct literals **[Stage 1]**

Struct literal fields use `=`, not `:`:

```
User {
    id = Uuid.generate(),
    name = name,
    age,       // shorthand when the variable name equals the field name
}
```

The owned form prefixes the type name with `:`:

```
let user: User = :User {
    id = Uuid.generate(),
    name,
    age,
};
```

A bare `{ ... }` is never interpreted as a struct literal — the type must
appear immediately before the literal (§2.6).

---

## 12. Traits **[Stage 1 core syntax; extensions deferred]**

Dispatch is static (monomorphized in Stage 1);
there are no vtables in Stage 1.

```
trait Sound {
    sound() String {
        return "...";
    }
}

impl Dog Sound {
    sound() String {
        return "Woof! Ruff!";
    }
}
```

Associated types/constants, const generics, and existential (`any Trait`) and
opaque (`some Trait`) types are implemented. A trait name is not itself a
value type; use `any Trait` for a runtime package or `some Trait` for an opaque
return. Blanket/conditional impls remain unsupported. Multiple traits on one
`impl` are comma-separated (`impl Dog Sound, Clone { ... }`). Generic
parameters accept one or more `+`-separated inline bounds, canonically
`<T Sound + Named>`. The colon form `<T: Sound>` is invalid. Every bound is
checked at instantiation, and methods declared by any bound are available in the generic body. The same
constraints may be moved after the declaration head with
`where T Sound + Named, U Clone`; `where T: Sound` is invalid for the same
reason. `where` is supported on functions/methods, structs,
enums, traits, and explicit generic `impl` blocks. A predicate naming an
undeclared parameter and a method supplied ambiguously by two bounds are
diagnosed.

---

## 13. Generics **[current behavior, unchanged by Stage 1]**

Release generic functions/methods/types/traits are monomorphized between HIR
and MIR. At `-O0`, a generic free function whose constrained type parameters
are direct value parameters and whose signature is layout-independent is
erased to one `any Trait` body and dispatched through witness closures.
Layout-dependent signatures, generic methods, associated-constant selection,
and other non-erasable shapes use the monomorphized fallback. See
[`../generics.md`](../generics.md) for the full example-driven guide.
Development and release strategies are tested with the same source programs
and are required to produce identical behavior.

---

## 14. Enums and match **[current behavior, unchanged by Stage 1]**

```
enum Color {
    Red, Green, Blue, White, Black,
    Custom(String),
}
```

`match` has no guards yet. Variant access uses `.`, never `::`.

---

## 15. Modules **[current behavior, unchanged by Stage 1 except file extension]**

```
mod user;
use self.user.User;
```

For `mod user;`, the driver looks for `user.nr` or `user/mod.nr` relative to
the declaring file (§2.1 — `.nt` is rejected). `self`, `super`, `crate` are
the relative roots. `stdlib.*`/`std.*` resolve from the bundled prelude
root without a `mod stdlib;` declaration. The package manifest
(`Nether.toml`) and a real package graph are **not yet implemented — Stage
6**; today there is only a single entry file and its local module tree.

---

## 16. Closures **[Stage 2 capture semantics]**

```
let add = (a i32, b i32) => {
    return a + b;
};
```

`move () => { ... }` explicitly transfers captured unique values into the
closure environment at creation. Capturing a `:T`/`:t` value from an ordinary
closure is rejected with a diagnostic requiring `move`. Reference captures
remain borrows; for a locally bound closure their origin stays live through
that closure's last use.

---

## 17. Unsafe, raw pointers, FFI **[not yet implemented — Stage 5]**

`unsafe { ... }` blocks, `unsafe fn`, `*mut T`/`*const T` raw pointers (and
their owned forms `:*mut T`/`:*const T`), and `extern "C"` FFI declarations
are specified for the full language but do not exist in the compiler yet.

---

## 18. Async/await, threads **[not yet implemented — Stage 4]**

`async fn`, prefix `await expr`, `task.spawn(...)` (Tokio-backed) and
`thread.spawn(...)` (raw OS threads) are specified for the full language
but do not exist in the compiler yet. `Send`/`Sync` compiler-understood
thread-safety traits and atomic ARC are bundled with this stage in the
roadmap (Stage 6) since a safe async/thread story depends on both.

---

## 19. Error handling **[current behavior, unchanged by Stage 1]**

`Result<T, E>` is a bundled prelude enum; `?` propagation exists. There is
no implicit `String -> Err(String)` coercion on `return`.

---

## 20. Built-in symbols **[current behavior, unchanged by Stage 1]**

Available without `use`: `println`, `print`, `Option`/`Some`/`None`,
`Result`/`Ok`/`Error`, `Into`.

---

## 21. Memory model summary

- Ordinary heap assignment (`T`) retains; scope exit releases.
- `:T` uses an unrefcounted unique allocation; moves do not retain, final
  destruction frees the sole allocation, and `:T -> T` promotes into ARC.
- `weak T` never affects its referent's retain count.
- Atomic ARC (thread-safe refcounting) is specified for the full language
  but not yet implemented — the runtime remains single-threaded until
  Stage 6.

Full retain/release insertion rules live in
[`../architecture/arc-model.md`](../architecture/arc-model.md).

---

## 22. Implementation status at a glance

| Area | Status |
|---|---|
| Four value forms (`T`/`:T`/`t`/`:t`) — grammar & type-checking | Stage 1 |
| Self-receiver domain overload resolution (§8.4) | **Stage 2 — done** |
| Move/use-after-move checking (§8.6.1) | **Stage 2 — done** |
| Callable `:&T`/`:&mut T` parameters, call-scoped exclusivity (§3.1, §8.1) | **Stage 2 — done** |
| Calling a *method* (not just field access) through a `:&T`/`:&mut T` receiver (§3.1) | **Stage 2 — done** |
| `to(value)` / `to<T>(value)`, 3 sound transitions (§9) | **Stage 2 — done** |
| Structural `T -> :T` clone, including recursively `Clone` direct unique fields (§10) | **Stage 3 — done** |
| Prefix trait derivation and structural `Eq` for ARC/unique structs (§10) | **Stage 3 — done** |
| Derived structural `Hash` and `hash(value) u64` (§10) | **Stage 3 — done** |
| Lexically scoped stored heap borrows; interprocedural named-function origin summaries; safe storage of returned references; ambiguous-origin diagnostics | **Stage 2 — done** |
| Method/closure origin summaries and ordinary last-use shortening | **Stage 2 — done** |
| Loop/local-closure-sensitive regions and `move () => {}` closures | **Stage 2 — done** |
| Reference-to-inline-value codegen | **Stage 2 — done** |
| `:T` unrefcounted allocation, move transfer and ARC promotion | **Stage 2 — done** |
| `#[allow_pascal_case]` on type aliases | **Stage 3 — done** |
| Multiple inline generic bounds (`T A + B`) | **Stage 3 — done** |
| `where` clauses on generic declarations | **Stage 3 — done** |
| `default impl` with concrete instance-method specialization | **Stage 3 — done** |
| Associated types/constants, const generics, `any`/`some` | **Stage 3 — done** |
| Development-mode witness-table generics dispatch | **Stage 3 — done** |
| async/await, Tokio runtime bridge | Stage 4 |
| unsafe, raw pointers, C ABI FFI | Stage 5 |
| Atomic ARC, `Send`/`Sync`, `Nether.toml` package manifest, CLI `run`/`test`/`--release`/`--emit-*` | Stage 6 |
| Optional/newline-aware semicolons, block comments, inline-array vs. vector literal distinction, variadics-as-fixed-array, mutable closure captures | unstaged syntax cleanup — see roadmap |

See [`../architecture/roadmap.md`](../architecture/roadmap.md) for the full
staged plan and the keep/refactor/rewrite/delete classification of existing
compiler components.

---

## 23. Canonical example **[Stage 1 syntax]**

```
mod lang;
use self.lang.Lang;

fn main() {
    let mut a = Lang.new("Bobby");
    a.set_name("Husky");
    println(a.into_string());

    let count i32 = 3;
    let owned_count: i32 = 3;
    let dog: Lang = :Lang { name = "Rex" };
    println(dog.name);
}

struct Lang {
    name String
}

impl Lang {
    new(name String) Lang {
        return Lang { name = name };
    }

    set_name(mut self, new_name String) {
        self.name = new_name;
    }
}

impl Lang Into<String> {
    into_string(self) String {
        return `name: ${self.name}`;
    }
}

trait Sound {
    sound() String {
        return "...";
    }
}

impl Lang Sound {
    sound() String {
        return "Woof! Ruff!";
    }
}
```
