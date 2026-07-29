# Nether Compiler — ARC Insertion Model

This document specifies the ARC (Automatic Reference Counting) insertion
pass precisely enough to implement and test against: what MIR looks like
before the pass, what instructions it inserts, and why each rule is what it
is. It assumes familiarity with [`overview.md`](./overview.md) (why this
pass runs on MIR, not HIR) and [`type-system.md`](./type-system.md)
(`AllocKind`, which this pass reads for every local).

## 1. Scope

Every type for which `Signatures::has_managed_content` is true participates
in lifetime insertion. This includes direct heap values and stack/value
tuples, structs, and enums containing managed fields; aggregates use
generated deep retain/drop shims. `weak T` has separate weak
retain/release operations and never increments its referent's strong count.

## 2. MIR instructions this pass introduces

```rust
pub enum Instr {
    // ... ordinary instructions (Assign, Call, ...) ...
    Retain(Local),
    Release(Local),
    WeakRetain(Local),
    WeakRelease(Local),
}
```

Ownership insertion is split inside MIR. CFG construction emits
path-sensitive scope, mutation, weak, and temporary cleanup while source
structure is known. `insert_arc` then adds uniform alias/call binding
retains over the finished CFG. Nothing upstream of MIR reasons about
reference counts.

## 3. Insertion rules

### 3.1 Assignment (`let x = y;` where `y`'s type is heap-kind)

```
Assign(x, Use(y))
Retain(x)
```

The retain happens immediately after the assignment, on the destination —
`x` and `y` now both reference the same object, and both need their own
accounted-for strong reference for their own eventual releases to balance.

### 3.2 Scope exit

Every local of heap-kind gets exactly one `Release` inserted at each point
control flow leaves its owning scope (block end, `return`, `break`,
`continue` — anywhere the CFG lowering already inserts a scope-exit edge).
Because MIR's block structure already makes scope boundaries explicit,
this is a mechanical walk: for each local live at a scope's exit edges,
insert a `Release(local)` on that edge, in reverse declaration order
(mirroring Rust/Swift drop order, which is the least-surprising choice for
anyone coming from either language).

### 3.3 Passing a heap value into a function call

```
Retain(arg_local)
Call(callee, [arg_local, ...])
```

Retain before the call — per spec §13 — and, for a call to another Nether
function, **no matching release after it returns**. This is a correction
to this document's own original design (caught only by actually running
generated code against a real allocator, not by inspecting MIR/IR shape:
adding a release here *on top of* the callee's own guaranteed scope-exit
release for its parameter double-frees the argument the moment the callee
returns, if the caller goes on to use it again — a completely ordinary
thing to do with a heap value in a reference-counted language). The
correct accounting: the pre-call retain hands the callee's parameter
binding its own independent credit; the callee's own scope exit (§3.2)
*always* releases it — or, if the callee returns that exact parameter
directly with no intervening binding, that same credit passes straight
through as the returned value's own (§3.4 explains why no *additional*
retain is needed for this case either, for the same reason). Either way,
exactly one of {callee's scope-exit release, the returned value's own
eventual release} consumes the pre-call retain's credit — never both, and
never neither.

`CallBuiltin`/`CallArrayMethod` (a call into `runtime` C-ABI code, not
another Nether function) are different and keep the *original*
retain-before/release-after shape unconditionally: a native callee has no
MIR body of its own to guarantee a scope-exit release, so the pair here is
the argument's *entire* transaction for the call, nets to zero, and grants
the native callee nothing lasting. Where a native callee genuinely needs
to keep what it was handed (`runtime/array`'s `push`, storing an element
into the array's own long-lived buffer), it performs that retain itself,
internally — see `crates.md`'s `runtime/array` section.

### 3.4 Returning a value

**General case:** the returned local is retained before the `Return`
terminator (the caller receives a fresh, independently-owned reference),
and every *other* still-live heap local in the returning scope is released
as normal (§3.2) before control actually leaves.

**Return Value Optimization (RVO):** if the returned expression is a
freshly constructed object *directly* in the `return`/tail-expression
position (a struct literal, a direct call to a function whose own result is
itself freshly constructed, etc. — i.e., there is no intervening local that
already holds an independent, retained reference to it), the pass skips the
extra retain: the object was already created at refcount 1 for this
purpose, and hands that same +1 to the caller without an intermediate
retain/release pair. Concretely, this is implemented as a peephole check
immediately before applying the general rule: if the `Rvalue` being
returned is a construction expression evaluated directly into the return
slot (never bound to an intermediate named local at all), no `Retain` is
inserted for it. If the value first went through an intermediate `let`
binding, the general case applies (a retain is inserted) — the RVO window
is specifically "constructed-and-immediately-returned," matching the
spec's own wording ("if returning a freshly constructed object directly").

**Returning a bare parameter directly** (`fn identity(d: Dog): Dog { d }`)
needs no special case at all, on either side: `d`'s only credit is the
caller's own pre-call retain (§3.3); this scope's normal "skip the
escaping local's own release" handling already leaves that credit
un-released here, so it simply passes straight through as the returned
value's own. An earlier version of this pass gave a bare returned
parameter a *second*, additional retain here, under the mistaken
assumption that its incoming credit was merely "borrowed" and would be
reclaimed by a caller-side release right after the call — since §3.3 no
longer does that, this extra retain would now leak a reference instead of
preventing a double-release, so it was removed along with that release.

### 3.5 `weak T`

Reading through a `weak T` (to get a usable, strong `T` for the duration of
some use) requires the runtime (`runtime/arc`) to perform a check-and
-retain-if-alive operation atomically with respect to the *single-threaded*
execution model (no actual concurrency to race against, but the referent
could have been released earlier in the same function via an explicit
scope exit) — this produces an `Option<T>`-shaped result at the MIR/codegen
level (a `weak` access desugars, in `hir`, to a call that returns
`Option<T>`; by the time this pass sees it, it is already ordinary
`Option`/enum handling plus, if the `Some` case is taken, an ordinary
retained `T` local following all the rules above). Copying/storing a weak
observer emits `WeakRetain`; scope exit or overwrite emits `WeakRelease`.
Neither operation changes the referent's strong count.

### 3.6 Mutable-reference parameters are borrowed

A `mut` parameter aliases caller storage and receives no caller-side
ownership transfer or callee scope release. This applies to stack and
managed values. If a borrowed managed parameter escapes through a return,
MIR snapshots and retains it so the result owns an independent credit.
Reassignment snapshots and drops the overwritten managed value before
storing the replacement.

## 4. Worked example

```
type Dog { name: String }

fn describe(d: Dog): String {
    let tag = d.name;   // String is heap-kind
    tag                 // tail expression: returned directly
}

fn main() {
    let a = Dog { name: "Rex" };  // fresh construction
    let msg = describe(a);        // pass a heap value into a call
    println(msg);
}                                  // scope exit: release `a`, `msg`
```

MIR sketch after this pass (elided non-ARC instructions):

```
fn main() -> () {
    bb0:
        _a  = Dog { name: "Rex" }   ; fresh construction, no Retain (it's the
                                     ; sole owner at construction time)
        Retain(_a)                  ; passing _a into describe (§3.3) — note
                                     ; there is deliberately no matching
                                     ; Release right after the call: describe's
                                     ; own scope exit (below) is what consumes
                                     ; this credit, not the caller
        _msg = call describe(_a)
        ; println is CallBuiltin, not a Nether function, so *that* call still
        ; gets the transactional Retain(_msg)/.../Release(_msg) wrap around it
        ; (elided here) — see §3.3's contrast.
        call println(_msg)
        Release(_msg)               ; scope exit (§3.2)
        Release(_a)                 ; scope exit (§3.2) — the *only* release
                                     ; of _a's own original construction credit;
                                     ; describe's own Release(_d) below already
                                     ; consumed the pre-call retain's credit
        return

fn describe(_d: Dog) -> String {
    bb0:
        _tag = _d.name
        Retain(_tag)                 ; field read binds a new owning local (§3.1)
        ; RVO: _tag is returned directly as the tail expression with no
        ; further intermediate binding, but _tag is itself already a named
        ; local from an assignment (§3.1), not a construction-in-place — so
        ; the general rule (retain already applied at §3.1) stands; RVO's
        ; extra-skip only applies to a *construction* expression sitting
        ; directly in return/tail position, which this is not.
        Release(_d)                  ; scope exit — releases the parameter's
                                     ; own reference (received via §3.3's retain
                                     ; in the caller)
        return _tag
}
```

This example also shows why RVO is narrower than "any tail expression": a
tail expression that's just a named local still needs its already-applied
retain (from when it was bound) to be balanced — RVO only elides an
*extra* retain that the general "retain the returned value" rule would
otherwise add on top of a construction that already started at refcount 1.

## 5. Testing this pass

Per `crates.md`'s testing section, `mir`'s ARC tests assert the exact
sequence of `Retain`/`Release` instructions produced for a given input MIR
(or a given HIR/source snippet run through the full lowering), rather than
only checking end-to-end program behavior — refcount bugs are exactly the
kind of bug that can be "accidentally correct" at low optimization levels
and wrong under different inlining, so the pass's output shape itself is
the thing under test, not merely observed `println` output. A baseline
suite should include: a struct assignment, a struct field read binding a
new local, a function call passing a heap value, RVO's two contrasted
shapes (worked example above), a `weak` read on a live and on an already
-released referent, and a `mut`-parameter case confirming zero
`Retain`/`Release` instructions are generated for it.

That said, MIR/IR-shape assertions alone missed a real double-release in
this pass's own history (§3.3): every existing test happened to never
re-read a call's argument *after* the call, which is exactly the pattern
that crashed once real generated code ran against a real allocator (a
freed `Dog` read back a null field, not a shape mismatch anything short of
execution would catch). The baseline suite above should now also always
include at least one case per call kind (`Call` to a Nether function,
`CallBuiltin`, `CallArrayMethod`) that *does* re-read its own heap-kind
argument after the call returns — and, since MVP-level trust in this pass
means trusting generated code to actually run correctly, not just verify,
periodically exercising it for real (compile → link against `runtime/*` →
execute) remains the strongest check available, MIR-shape assertions are
a fast substitute for the common case, not a replacement for it.

## 6. Future extension points

- **Escape analysis / stack promotion**: proving a heap allocation never
  outlives its creating scope could allow eliding the corresponding
  retain/release pair (and the heap allocation itself) entirely. This
  would be an additional, optional pass between `build_mir` and this one
  (or a refinement of this one) — the instruction set (`Retain`/`Release`)
  defined here does not need to change to support it.
- **Redundant retain/release elision** beyond RVO (e.g. a retain
  immediately followed by a release of the same local with no intervening
  use) is a straightforward peephole pass over this pass's own output and
  is a natural first MIR-level optimization to add post-MVP.
