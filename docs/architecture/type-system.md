# Nether Compiler — Internal Type Representation

This document describes the representation implemented in
`compiler/typecheck`. Types are structural data, never diagnostic strings.
Named declarations use the resolver's program-wide `DefId`; generic
arguments are part of the type and therefore part of layout/cache keys.

## 1. `Type`

```rust
pub enum Type {
    Primitive(PrimitiveKind),
    Struct(DefId, Vec<Type>),
    TupleStruct(DefId, Vec<Type>),
    Tuple(Vec<Type>),
    Enum(DefId, Vec<Type>),
    Array(Box<Type>),
    String,
    Function(Vec<Type>, Box<Type>),
    Interface(DefId),
    Generic(Symbol),
    Weak(Box<Type>),
    Never,
    Error,
}
```

`Struct`/`TupleStruct`/`Enum` carry their concrete type arguments.
`Option<T>` and `Result<T, E>` are represented exactly as `Enum` values;
there are no special `Option` or `Result` type variants. `Never` supports
control-flow expressions such as `return`; `Error` is a poison type used
after a diagnostic to suppress cascades. Neither may reach codegen in a
successful compilation.

Named definitions are module-qualified by resolver identity. Two modules
may declare the same source name and still receive different `DefId`s.

## 2. Generic signatures and substitution

Declaration field/payload types may contain `Type::Generic(Symbol)`.
`Signatures::type_fields` and `Signatures::enum_payload` substitute the
concrete arguments from a `Type` before HIR/MIR/codegen use them. This is
the invariant that prevents `Option<Dog>` or `Boxed<String>` from reaching
layout code with an unresolved generic payload.

A bound retains its interface arguments:

```rust
pub struct GenericBound {
    pub interface: DefId,
    pub args: Vec<Type>,
}
```

Consequently `Convert<String>` and `Convert<i32>` are distinct. Generic
interface method signatures and inherited default bodies are specialized
with these arguments. Generic type parameters are also in scope
implicitly inside `impl Boxed { ... }`, so `self` remains `Boxed<T>` until
monomorphization.

Interface bounds form a transitive graph. `Signatures` stores each
interface's generic parameters and direct parent templates; satisfaction
recursively substitutes the concrete child arguments into those templates.
Thus `Child<String>: Parent<String>` allows a concrete `Child<String>`
implementation wherever `Parent<String>` is required. The type checker
flattens inherited method signatures and rejects cycles, incompatible
signatures, and unresolved default-body conflicts before HIR.

Inference is intentionally local: it walks parameter/argument and expected
return types structurally. Explicit `call<Type, ...>(...)` arguments seed
the same substitution map before expected-return and ordinary-argument
inference; receiver-fixed owner parameters are excluded from the explicit
method list. The fully resolved arguments are recorded by call `NodeId` so
HIR and monomorphization do not need to re-infer an otherwise opaque
parameter. There are no higher-kinded types, associated types,
where-clauses, specialization, or blanket implementations.

## 3. Allocation classification

`alloc_kind(ty, definitions)` implements the language's naming rule:

- PascalCase `type`/tuple-struct declarations are heap/ARC values;
- camelCase declarations, primitives, tuples, and every enum are values;
- `String`, `Array<T>`, and function/closure environments are heap/ARC;
- `weak T` is a stack-sized observer, legal only when `T` is heap-kind.

Classification happens after substitution. A generic stack aggregate can
still contain managed fields, so `Signatures::has_managed_content` is the
broader ownership predicate used by MIR. It recursively detects heap
fields, closure values, strings, arrays, and weak observers inside
tuples/value structs/enums and selects generated deep retain/drop shims.

Directly recursive value layouts such as `type node { next: node }` or
`enum List { Next(List) }` are rejected by typecheck. A PascalCase heap
type is an indirection and therefore breaks such a layout cycle.

## 4. Interfaces

Interfaces are constraints, not runtime value types. `Type::Interface`
appears while checking declarations/default bodies, but a source
expression, field, or ordinary parameter cannot have an interface as its
dynamic type. Dispatch is resolved statically and erased during
monomorphization; there are no vtables or `dyn Interface` values.

## 5. Concrete layout

Typecheck defines shapes, not byte offsets. Codegen computes and caches
LLVM layouts per concrete `Type`:

- stack structs/tuples are addressed in local storage;
- heap structs are pointers to ARC payloads;
- enums use a tag plus flattened payload fields;
- weak/function/string/array values use pointer representation.

The current enum representation favors simple, verifiable code over space:
payload fields for all variants coexist rather than overlap as a union.

## 6. Extension points

Const generics would require a non-type substitution value alongside
`Type::Generic`. Associated types would extend interface signatures.
Neither change requires LLVM-specific data in the front-end type model.
