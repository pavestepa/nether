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
Unique heap `:T` locals are outside ARC alias accounting: they use a
count-free allocation, move one ownership credit, and receive only a final
`unique_free`-backed release.

## 2. MIR instructions this pass introduces

```rust
pub enum Instr {
    // ... ordinary instructions (Assign, Call, ...) ...
    Clear(Local),
    Retain(Local),
    Release(Local),
    TransientRelease(Local),
    WeakRetain(Local),
    WeakRelease(Local),
}
```

`Release` and `TransientRelease` both emit the same runtime release call
— the distinction is purely about what it means for the *local*
afterward, and exists for `nether_codegen`'s own async-frame bookkeeping
(§7) to consume, not for any difference in the release itself. `Release`
means the local's ownership genuinely ends here — nothing later in this
function still expects a live value in that slot (scope exit, an
overwritten place's old value, a purpose-built temporary's single use).
`TransientRelease` is exclusively §3.3's call-argument retain/release
pair around a `CallBuiltin`/`CallArrayMethod` call: a purely transactional
bump for the call's own duration that leaves the argument's own ownership
completely unaffected — `println(x); println(x);` retains and releases
`x` around *each* call without ending its life either time. Conflating
the two was a real bug, not just a naming nicety: `nether_codegen`'s
async-frame codegen nulls a local's frame field immediately after a
`Release` (§7), and before this split it did so after *every* release,
including the transactional kind — nulling a still-live local's field the
moment its first `println` call returned, corrupting the second one.

Ownership insertion is split inside MIR. CFG construction emits
path-sensitive scope, mutation, weak, and temporary cleanup while source
structure is known. `insert_arc` then adds uniform alias/call binding
retains over the finished CFG. Nothing upstream of MIR reasons about
reference counts.

`Clear` is the ownership-transfer marker for a unique heap move. Codegen
stores null into the moved-from slot after its pointer has reached the
destination. Lexical cleanup can therefore remain structural: releasing a
moved-from slot is harmless, while the destination remains the sole owner.

### 2.1 Unique allocation and ARC promotion

Constructing `:T` calls `nether_rt_unique_alloc`, whose header stores only
payload size and the drop callback—no strong or weak counters. Moves insert no
`Retain`; final destruction dispatches to `nether_rt_unique_free`. The explicit
`:T -> T` conversion calls `nether_rt_unique_promote`, which moves payload bytes
into a fresh ARC block, deallocates the unique block without dropping the
moved fields, and clears the source slot. Stage 3's structural `T -> :T`
clone allocates a distinct unique outer payload, byte-copies its fields, then
runs a generated field-retain shim so copied ARC/weak fields own independent
credits. A direct unique heap field is recursively cloned when its inner type
also implements `Clone`; a non-Clone or cyclic clone graph is rejected.

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

Retain before the call — per spec §21 — and, for a call to another Nether
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
the native callee nothing lasting. The release half of this pair is
emitted as `TransientRelease`, not `Release` — see §2's own note on why
that distinction exists and matters. Where a native callee genuinely needs
to keep what it was handed (`runtime/array`'s `push`, storing an element
into the array's own long-lived buffer), it performs that retain itself,
internally — see `crates.md`'s `runtime/array` section.

### 3.4 Returning a value

**General case:** the returned local is retained before the `Return`
terminator (the caller receives a fresh, independently-owned reference),
and every *other* still-live heap local in the returning scope is released
as normal (§3.2) before control actually leaves.

**Return Value Optimization (RVO):** if the returned expression is a
freshly constructed object *directly* in the explicit `return`
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
-retain-if-alive operation atomically — a real `compare_exchange` CAS loop
as of Stage 6 (`runtime/arc`'s strong/weak counts are `AtomicI64`, mirroring
`std::sync::Arc`/`Weak`'s own dealloc design exactly: the weak count starts
at 1, an implicit weak reference held collectively by the strong count and
released by whichever `release` call drops it to zero, funneling the
dealloc decision through a single counter's fetch-sub instead of two
independent counters racing each other). Before Stage 6 this needed no
atomics at all (no actual concurrency to race against — the referent could
only have been released earlier in the same function via an explicit scope
exit); now a `weak T` observer can genuinely be read from one
`thread.spawn`ed thread while another thread concurrently drops the last
strong reference, and the runtime's atomics are what make that race safe
rather than a plain check-then-retain being merely a convenient
description of ordinary sequential control flow. This still produces an
`Option<T>`-shaped result at the MIR/codegen
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
struct Dog { name String }

fn describe(d Dog) String {
    let tag = d.name;   // String is heap-kind
    tag                 // tail expression: returned directly
}

fn main() {
    let a = Dog { name = "Rex" };  // fresh construction
    let msg = describe(a);         // pass a heap value into a call
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

## 7. Async frames: ARC on heap-allocated, persistent locals

Stage 4's real state-machine transform (`nether_codegen`'s `frame`/
`function::async_fn` modules — see `roadmap.md`'s Stage 4 entry) turns an
`is_async` function's locals from per-call `alloca`s into fields of one
heap-allocated frame that outlives any single `poll()` invocation. This
section documents where that broke assumptions §1–§6 above take for
granted in an ordinary function, and how each was fixed — every fix here
was found by actually running compiled async programs, not by inspecting
MIR/IR shape (the same lesson §5 already draws from §3.3's own history,
now repeated twice more).

**Why an ordinary function's ARC bookkeeping doesn't automatically work
for a suspended frame.** Several of this document's own rules rely,
implicitly, on a local's storage disappearing the moment its owning
function returns: RVO (§3.4) deliberately leaves a moved-from source
local un-cleared, because nothing ever reads it again before the stack
frame is discarded. The same is true whenever a "constructing rvalue"
(`Construct`/`ConstructVariant`/`Tuple`/a closure's captures) or a plain
reassignment absorbs an already-owned value from an existing local
without a retain (`nether_mir::build`'s own `prepare_new_binding` doc
comment: "already carries exactly the right credit... needs no further
action") — the *source* local is left holding a stale, duplicate
reference to a value someone else now owns, harmless only because that
source local's storage is about to vanish. A persistent async frame's
fields don't vanish that way, so a later, generic "release whatever's
still in every local" walk over the frame would release that same
reference a second time, on top of its new owner's own eventual release.

`nether_mir::build::expr::clear_moved_into_container` is the fix: at
every one of these "absorbed without a retain" sites — a `Construct`
field, a `Tuple`/`ConstructVariant` element, a closure capture, or a
reassignment's incoming value — the source local (if it names one at
all, i.e. it wasn't already a trivial alias that got its own real
retain) is `Instr::Clear`ed immediately after the absorbing instruction.
This generalizes `Instr::Clear`'s pre-existing role (nulling a
moved-from *unique* local, §2) to ordinary ARC values whenever they're
folded into a new container this way. It's a pure MIR-level fix, with no
`is_async` conditional anywhere: the extra `Clear` is free/dead-store
overhead in an ordinary function (whose `alloca` is discarded on return
regardless) and load-bearing for an async frame. The escaping-return case
(`return`'s own operand) is handled the same way but at the codegen
level instead, in `FnCodegen::gen_async_return` — see that function's own
doc comment.

**The frame's own drop-while-pending invariant.** A `Task<T>` frame can
be released while its `poll()` has never reached `Return` — dropped
after being polled zero or more times and found not-ready each time. The
frame's `nether_rt_arc_alloc` drop callback (`frame::frame_drop_shim`,
generated once per `is_async` function) walks every *directly* heap-kind,
non-alias-parameter local and unconditionally releases whatever pointer
its field currently holds — `nether_rt_arc_release`/`nether_rt_unique_free`
are themselves null-safe (§2's `Clear` reasoning again), so this is safe
as long as a field that's *not* still owning a live reference is
reliably null. `FnCodegen::null_frame_field_after_release` maintains that
invariant going forward from any point in a frame's lifetime: it nulls a
local's own frame field immediately after a genuine `Release` of it
(never after a `TransientRelease` — see §2's own note on why that
distinction is exactly what makes this safe), and the frame's *start*
function zero-initializes every eligible field up front, so a local never
yet assigned by the time an early suspend drops the frame reads back as
"nothing to release" rather than whatever bytes the allocator happened to
return.

**Documented v1 gap.** The null-after-release invariant above only
covers *directly* heap-kind locals (a bare pointer slot). A stack-kind
aggregate local (`Tuple`/`enum`/non-`Pascal`-cased `struct`) with a
nested heap-kind field is not covered: if that inner field was already
released in-scope (via the aggregate's own generated shim, `crate::shims`
in `nether_codegen`) before a later suspend point, `frame_drop_shim`'s
walk has no nested walk of its own to skip it safely, and could
double-release it. Closing this needs extending the null invariant to
nested fields, which needs real per-field liveness this stage
deliberately doesn't build (the roadmap's own "flat frame, no liveness
analysis" scoping decision for Stage 4). Accepted for v1; revisit if a
real program hits it.

**Idempotent completion.** Real concurrent scheduling (`task.spawn`'s
background polling and an explicit `await` of the same task can race to
poll it) means a `poll()` can legitimately be called again after it
already returned "ready" once. `frame::FrameLayout::completed_state`
reserves one discriminant value no real suspension point ever uses;
`Terminator::Return`'s codegen stores it right before returning, and the
poll function's own dispatch checks for it first, ahead of every real
per-state branch, returning "ready" immediately without touching
anything else. Without this, a second poll would re-dispatch to whatever
await-check block the frame was last suspended at and re-run everything
from there — a real bug, caught by an actual double-`println` from a
concurrently-scheduled task, not a hypothetical.
