# Nether Compiler Rewrite — Roadmap

This document tracks the staged migration from the pre-rewrite MVP compiler
to the language described in
[`../spec/language-spec.md`](../spec/language-spec.md). It exists so that
"not yet implemented" in the spec always has a concrete home, and so a
contributor picking up any later stage has the classification work already
done instead of needing to re-derive it.

---

## 1. Component classification (keep / refactor / rewrite / delete)

Established by a full audit at the start of Stage 1. Nothing in the
workspace was classified as pure `delete` — even the most MVP-specific
pieces (e.g. casing-derived allocation) turned out to be small, well
isolated, and worth refactoring in place rather than removing and
rebuilding.

| Crate | Classification | Notes |
|---|---|---|
| `compiler/diagnostics` | **keep** | Zero-dependency, hand-rolled, no coupling to old language semantics. Untouched by Stage 1. A future swap to `codespan-reporting`/`ariadne` remains a documented, optional upgrade, not a requirement. |
| `compiler/lexer` | **keep + minor extension** | Generic hand-rolled scanner. Stage 1 adds one keyword (`struct`); no other lexer change needed for Stage 1's grammar. |
| `compiler/ast` | **refactor** | Data-only node shapes are a good foundation. Stage 1 adds `TypeExpr::{Unique,Ref,MutRef}`, a `StructDecl`/`Item::Struct` split from the alias-only `Item::Type`, `TypeAliasDecl`, extended `SelfParam`, and `=`-based struct-literal fields. |
| `compiler/parser` | **refactor** | Recursive-descent/Pratt mechanics are reusable; the *grammar rules* encoded old syntax (colon-before-return-type, `mut name: Type`, `type` for structs, `:` struct-literal fields, bare-block expressions). Stage 1 rewrites these rules without touching the parsing mechanism. |
| `compiler/resolver` | **keep, minor threading** | Flat single-namespace, file=module design is sound. Stage 1 threads through the new `Item::Struct`/`Item::TypeAlias` items; the current compiler also enforces default-private declarations and explicit `pub` imports/re-exports. |
| `compiler/hir` | **refactor** | Straightforward AST→HIR desugaring. Stage 1 adds `Unique`/`Ref`/`MutRef` lowering (mirroring the existing `Weak` handling), alias substitution with a cycle guard, and moves fn-body tail-expression suppression here. |
| `compiler/typecheck` | **largest refactor** | The structural `Type` enum was already good design; Stage 1's biggest, riskiest change is extending it with `Unique`/`Ref`/`MutRef` and replacing casing-as-mechanism with casing-as-validation in `alloc.rs`. |
| `compiler/monomorphization` | **keep** | Release builds monomorphize; development builds share dictionary-safe generic bodies through existential witness tables, with a monomorphized fallback for layout-dependent shapes. |
| `compiler/mir` | **keep, pass-through only** | The ARC-insertion pass (`arc.rs`) already keys off a single `AllocKind` chokepoint — decoupling from casing turned out to need no logic change, only new pass-through match arms. |
| `compiler/llvm` | **keep** | Thin, well-isolated `inkwell` facade. The dev/release generic strategy is selected before MIR; LLVM consumes either form uniformly. |
| `compiler/codegen` | **refactor** | Real LLVM IR generation, drop-shim generation reused as-is. Stage 1 adds reference codegen for heap-category types only (§3.1 of the spec). |
| `compiler/driver` | **refactor** | `module_loader.rs` gets the `.nr`-only migration and legacy-`.nt` diagnostics; the rest of the pipeline wiring (`lib.rs`) is largely unaffected by Stage 1. |
| `runtime/{arc,string,array,io}` | **keep for Stage 1** | Clean, minimal, but explicitly single-threaded/non-atomic today — the spec requires atomic ARC eventually (Stage 6). No change needed until concurrency lands. |
| `cli` | **keep, minor scope** | Hand-parsed args, `check`/`build`/`ast` only. Subcommand overhaul (`run`/`test`/`--release`/`--emit-*`) is Stage 6, alongside the package manifest that gives those commands something to build against. |

---

## 2. Stage sequence

**Implementation snapshot (2026-08-27).** Stages 1 through 7 are complete.
Stage 2 has
flow-sensitive move checking, reference parameters and receiver calls,
lexically scoped stored heap borrows, plus the first three `to(value)`
ownership-domain conversions. Returned-reference origins are checked and
propagated through named-function call chains, including storage in `let`;
default-private visibility with explicit `pub` is implemented for declarations,
imports/re-exports, fields, and methods;
Method/closure origin summaries, loop/local-closure-sensitive last-use
shortening, inline-reference codegen, explicit `move` closures and `to<T>`
are implemented. Unique heap values use an unrefcounted runtime allocation
and promote explicitly into ARC for `:T -> T`. Stage 6 added atomic ARC,
compiler-derived `Send`/`Sync`, raw OS threads, spawned-borrow diagnostics,
the `Nether.toml` package manifest, and the CLI overhaul. Stage 7 added
newline-aware optional semicolons, block comments, the fixed-array vs.
growable-array literal split, mutable (`mut (...) => {...}`) closure
captures, and confirmed variadics-as-fixed-array was already correct.

**Stage 1 (this rewrite's first slice) — done.** Four value forms
(`T`/`:T`/`t`/`:t`) as explicit, structural syntax; casing demoted from
allocation mechanism to validated naming convention; `struct`/`type`
keyword split; `.nt` → `.nr` migration; no implicit tail-expression return;
no bare block expressions. Explicitly **not** included: any flow-sensitive
move/borrow analysis, reference-to-inline-value codegen, existential/opaque
types, associated types/const generics/specialization, async, unsafe/FFI,
atomic ARC, package manifest.

**Stage 2 — real borrow checker — done.**

*Move checking — done.* Flow-sensitive use-after-move/double-move
detection for `:T`/`:t` local bindings (spec §3, §8.6.1) — exactly the
diagnostic the spec's own §3 walkthrough shows, now enforced. Implemented
as a dataflow pass integrated directly into `nether_typecheck::check`'s
existing per-function `Checker` walk (not a separate post-pass over HIR/MIR
as originally sketched — HIR carries no `Span`s and MIR only exists per
monomorphized instantiation, neither of which suits per-source-location
diagnostics; the AST-level `Checker` already had live per-`LocalId`
type/mutability state and is where method-call domain resolution already
happens, so extending it in place was the smaller, more correct change).
No lifetime/region inference needed — see
[`../spec/language-spec.md`](../spec/language-spec.md) §3's own explanation
of why moves alone don't need one. Bundled alongside it: fixed a Stage 1
gap where `foo(self)` and `foo(: self)` on one type hard-conflicted as
"defined more than once" instead of coexisting as the two domain-specific
overloads spec §8.4 documents (`ReceiverDomain`, threaded through
`nether_typecheck::sig::Signatures::methods`,
`nether_hir::HirModule::methods`, and monomorphization's method-set
lookups) — move-checking a `: self`-consuming method needed this to exist
at all.

*Callable `:&T`/`:&mut T` parameters — done.* Investigating "borrow-
exclusivity" as the natural next item found it wasn't implementable in any
testable form yet: there was still no expression-level `&expr` syntax
anywhere in the grammar, and `Type::compatible()` had no coercion letting
an owned `:T` value satisfy a `:&T`/`:&mut T` *parameter* (only self-
receivers were reachable, via slice 1's `ReceiverDomain` work). This slice
closed that gap: an owned local now satisfies a reference parameter by its
bare name (no new syntax — the parameter's own declared type triggers the
borrow, the same implicit mechanism an ordinary ARC parameter already
uses), with call-scoped exclusivity (no double-`:&mut`, or mixed `:&`/
`:&mut`, of the same local within one call's argument list — see spec
§3.1/§8.1). Concentrated almost entirely in `nether_typecheck` (a new
`check_call_args` branch in `compiler/typecheck/src/check/construct.rs`)
and `nether_hir` (`Type::strip_indirection`, a `strip_unique`-alongside
helper in `compiler/typecheck/src/ty.rs`, swapped in wherever HIR already
stripped `Unique` for the same "shares the referent's runtime
representation" reason) — confirmed empirically (a driver end-to-end test,
not just structural inspection) that **zero MIR/codegen changes were
needed**: `Type::Ref`/`Type::MutRef` already mapped to `AllocKind::Stack`
and were already excluded from `Signatures::has_managed_content`, so
`mir::insert_arc`'s existing `needs_drop`-gated retain/release skip and
`codegen`'s existing pointer layout for these types already did the right
thing, unfed. Also fixed, discovered along the way: `Checker::check_fn_decl`
only granted mutation permission for the *ARC* mutable-receiver/parameter
forms (`mut self`, `name mut Type`) — `: &mut self` and a `:&mut T`
parameter's own, always-exclusive mutability were both silently ignored,
which would have made mutating through either one a false "cannot assign
to an immutable binding" error.

*Method calls through a `:&T`/`:&mut T` receiver — done.* Turned out
smaller than slice 2's own sketch of the fix ("split `ReceiverDomain` into
a consuming vs. borrowing distinction, threaded through every touchpoint
slice 1 built") — no new `ReceiverDomain` variant was needed at all.
`ReceiverDomain::of_receiver_ty` (`compiler/typecheck/src/sig.rs`) now maps
`Type::Ref`/`Type::MutRef` into the *same* `Owned` bucket a bare owned
value already used (a `:&Dog` is a reference to an owned `Dog` — the same
declarations apply), and `check_method_call_on`
(`compiler/typecheck/src/check/method.rs`) rejects the one unsound case
with a dedicated post-lookup check: a `: self` (consuming) body found
through a reference receiver, rather than a bare owned one. Because
`of_receiver_ty` is the single function `nether_hir`'s domain-resolution
helpers and `nether_monomorphization`'s method dispatch both already
traced back to (by design, across slices 1–2), changing it there was
sufficient — no HIR or monomorphization changes were needed, confirmed by
a driver end-to-end test calling a `: &mut self` method through a
`:&mut T` parameter and observing the mutation. Also widened, for the same
reason slice 2 had to fix `check_fn_decl`'s mutability flag: the existing
`mut self`-through-immutable-receiver check now also covers
`: &mut self`, reusing the same `receiver_mutable` computation slice 2
already made correct for both an owned `mut` binding and a `:&mut T`
parameter.

*Stored heap borrows with lexical origins — done.* An explicitly typed
`let view: &T = owned;` or `let view: &mut T = owned;` now forms a stored
borrow from a bare `:T` local without adding an `&expr` operator. The
typechecker mirrors resolver block scopes, records each reference local's
origin, prevents moves/direct mutation/conflicting call or stored borrows
while it is live, and releases exclusivity when the reference leaves its
lexical block. Heap representation is verified end-to-end through
HIR/MIR/codegen. Last-use shortening releases ordinary stored borrows after
their final use, after a final loop expression, or after the last use of a
locally bound closure that captures them. Non-local closure lifetimes retain a
safe lexical fallback. Inline references use explicit address-of/dereference
nodes through HIR and MIR and addressable stack locals in codegen.

*Returned-reference origin inference and function summaries — done.* A `:&T`/`:&mut T`
return expression is traced through bare reference locals and `if`/`match`/
block result branches. Exactly one incoming reference-parameter origin is
accepted; a reference borrowed from a local owned value is rejected as an
escape, and multiple possible parameter origins receive a dedicated
ambiguous-origin diagnostic. A direct single-parameter return is covered
end-to-end through native execution. Function signatures carry parameter-index
origin summaries computed to a fixed point, so origin is preserved through
declaration-order-independent call chains. A returned reference can be stored
in `let`; its ultimate owned-local origin is then registered as a lexical
borrow and prevents moves, mutation, and conflicting borrows until scope exit.
Instance/static methods carry the same summaries, with `self` represented as a
distinct origin, and closure summaries preserve parameter and captured origins.

*`to(value)` domain conversion — Stage 2 delivered 3 of 4 transitions.* Checked each of
the spec's four transitions (spec §9) against what's actually sound without
`Clone` (`Copy`/`Clone` don't exist yet — spec §10, Stage 3): `:T -> T`,
`:t -> t`, and `t -> :t` are supported. Inline transitions are pure
type-system relabeling. `:T -> T` transfers the unique payload into a fresh
ARC allocation without cloning its fields. These three are implemented as a
`println`/`print`-style compiler builtin
(`nether_resolver`'s `def/collect.rs` registers `"to"` as a builtin `Fn`
the same way; `nether_typecheck::check::call::resolve_fn_value` and
`nether_hir`'s `lower_def_value` both special-case the name). `T -> :T`
stayed rejected during Stage 2 with a dedicated diagnostic: the source may have other live
ARC aliases, so relabeling it `:T` without an actual deep copy would
produce a "uniquely owned" value that isn't — genuinely needs `Clone`, not
a shortcut worth taking early. Stage 3 now implements that fourth transition
for structural `Clone` structs without unique fields. HIR/MIR carry an explicit
unique-promotion node for the opposite heap transition. Both target-inferred `to(value)` and explicit
`to<T>(value)` forms are implemented, including ownership-qualified inline
targets such as `to<:i32>(value)`.

*Stage 2 completion.* Loop-contained uses keep a borrow live through the loop
expression and release it afterward when no later use exists. Locally bound
closures transfer captured-reference use counts to the closure binding, so
the borrow ends after the closure's last use; non-local/escaping closure
values retain a conservative lexical fallback. `move () => {}` is explicit
and required for unique captures. Inline shared/mutable references lower to
addressable stack locals and are verified through native reads, returned
references, and writes. `:T` uses `unique_alloc`/`unique_free`, moves without
retain, and `to<T>` promotes it into a fresh ARC allocation. The reverse
`T -> :T` operation is no longer Stage 2 work: it is a deep-copy operation
and belongs with `Clone` in Stage 3.

**Stage 3 — trait system extensions + dev-mode generics — done.** The
narrow `#[allow_pascal_case]` type-alias escape hatch is implemented. The
compiler-known `Clone` marker also enables structural `T -> :T` conversion for
user structs: codegen allocates an independent unique outer object, retains
copied ARC/weak fields, and recursively clones direct unique heap fields whose
inner types also implement `Clone`. Prefix trait derivation syntax such as
`Clone + Eq struct Value` lowers through the same trait pipeline. User-written
clone overrides with signature `clone(: &self): T` are dispatched directly by
`to<:T>`. `Eq struct Value` now derives structural `==`/`!=` for both ARC and
unique values, recursively comparing primitive, tuple, nested `Eq`, and unique
fields; unsupported or cyclic field graphs are diagnosed. Derived `Hash`
is implemented for integer/bool/char, `String`, tuple, nested `Hash`, and unique fields;
the compiler builtin `hash(value)` returns a deterministic `u64`, while
unsupported and cyclic graphs are diagnosed. Strings use a deterministic hash
of their UTF-8 bytes. Floating-point fields remain excluded until `Eq`/hash
semantics for signed zero and NaN are specified. Associated types/constants,
const generics and fixed arrays, `any Trait`/`some Trait` with
existential-safety checks, and runtime witness dispatch are implemented.
At `-O0`, dictionary-safe generic free functions whose constrained type is
passed directly as a value share one existentially erased body; functions
whose ABI/layout depends on the type (and type-only associated-constant calls)
fall back to monomorphization. `-O1` through `-O3` retain full
monomorphization/devirtualization/specialization. Variadics now lower to a
hidden const-sized fixed array.

**Stage 4 — async/await + Tokio runtime bridge — done.** `async fn`
(including generic and method forms) and prefix `await expr` are real
compiler-generated state machines, not eager execution wrapped in a
completed-looking value: `nether_mir::split_await_points` turns every
`await` into a `Terminator::Await` suspension point in the CFG (run
immediately after `insert_arc`, so already-inserted retain/release
instructions relocate as a unit rather than needing rederiving), and
`nether_codegen`'s `frame`/`function::async_fn` modules compile each
`is_async` function into two LLVM functions — a `start` function that
heap-allocates a frame and returns it as `Task<T>`, and a `poll` function
carrying the translated MIR body, dispatching by a discriminant stored in
the frame to resume at the right suspension point. Since this compiler's
MIR is not SSA (every local already lives in its own persistent slot for
a function's whole lifetime), turning per-function storage into per-frame
storage is the entire state-machine transform — no liveness analysis was
needed, only a flat "every local gets a frame field" layout. `task.spawn`
schedules real, independent background progress via `tokio::spawn`
(`runtime/task`'s `nether_rt_task_spawn`), restricted by typecheck to
`async fn` bodies (mirroring `await`'s own restriction), since a runtime
must already be active for `tokio::spawn` to resolve. `timer.sleep(...)`
remains a native, deadline-polled task. See
[`../architecture/arc-model.md`](./arc-model.md) §7 for the frame's own
ARC drop-glue design and §3.3's `TransientRelease` split, both real
correctness fixes found only by running generated code against a real
allocator (see that document's own §5 note on why MIR-shape assertions
alone are not enough for this pass) — not by inspecting MIR/IR shape.

*Known, documented v1 gap, not silently left for someone to hit later:*
a stack-kind aggregate local (`Tuple`/enum/stack `struct`) with nested
heap-kind fields, live across an `await`, isn't covered by the frame's
own null-after-release invariant if its inner field was already released
in-scope before a later suspend — `nether_codegen::frame::frame_drop_shim`'s
own doc comment flags this precisely; closing it needs real per-field
liveness this stage deliberately didn't build (see arc-model.md §7).
Real waker/reactor integration is likewise deferred — a spawned task is
driven by cooperative re-polling (`runtime/task`'s existing busy-poll
convention, matching `nether_rt_task_block_on`'s own `yield_now` loop),
not woken only on the specific native event it's blocked on; worth
revisiting once real OS-thread parallelism (Stage 6) makes the busy-poll
cost worth removing.

**Stage 5 — unsafe, raw pointers, C ABI FFI — done.** `unsafe fn`/`unsafe
{ ... }` (a genuine block-scoped, save/restored `in_unsafe` context —
unlike Stage 4's function-scoped-only `in_async` — but with zero runtime
representation: HIR lowering erases `unsafe { ... }` entirely, reusing
`lower_block_as_expr` directly). `*const T`/`*mut T` raw pointer types
and their owned forms `:*const T`/`:*mut T` (free — the parser's existing
generic `:`-prefix wrapping and HIR's existing inline-`Unique`-erasure
rule for stack-kind inner types both already applied unchanged, no new
code needed for the owned form specifically). `&raw const expr`/`&raw
mut expr` address-of, computing the address of an arbitrary *place*
(any local/field/index/deref chain) with no borrow-checker involvement —
and prefix `*expr` dereference, both reusing the *existing*
`HirExprKind::Borrow`/`Deref` → `Rvalue::AddressOf`/`Deref` MIR pipeline
unchanged, since `lower_place` already recursed through arbitrary
projection chains generically, just never reachable for anything but a
whole local before this stage. `extern "C" { ... }` declarations with
FFI-safety checking (a numeric/`bool` primitive, a raw pointer of any
pointee type, or `()` only — aggregates rejected outright, sidestepping
the real System V AMD64/AAPCS64 struct-register-classification gap
confirmed absent from this codebase, rather than risking a silent
miscompile) and `#[link(name = "...")]`, threaded into the real `cc`
invocation as `-l<name>` (`link::link`, extended with a `link_libs`
parameter collected by flat-walking the already-loaded module graph's
`Item::Extern` blocks — no new IR-level plumbing needed, since this is
purely a linker-invocation concern, not program semantics).

The original design question — whether to reuse the existing `to<T>`
builtin as a reference-to-pointer conversion, on the condition that it
target a borrowed *place* (a field/element), not just a whole local — was
resolved against `to<T>` after directly confirming `:&T`/`:&mut T`
borrow-formation is gated by `Checker::bare_local_of`, entangled with
`LocalId`-keyed exclusivity tracking with no field/place variant;
extending that would mean a real disjoint-field-borrow project in its own
right, not a Stage 5 detail. Per the pointer-syntax design's own stated
fallback, `&raw const`/`&raw mut` was used instead — arguably the more
correct model on its own terms, since forming a raw pointer carries no
aliasing guarantee to violate (Rust's own reasoning for the same split).

An `extern "C"` declaration doesn't fit `HirFunction`/`MonoFunction`/
`MirFunction`'s existing "has a real body" shape — rather than invent a
wholly separate `ExternFunction` record threaded through four crates'
worth of call-target resolution, each of those three structs instead
gained a plain `is_extern: bool` flag: HIR gives it a trivial placeholder
body (never read), `nether_mir::build_mir`'s `FnBuilder::build` returns a
minimal `MirFunction` early for it (declared params, one `Unreachable`
block, no real instructions), and `nether_codegen` special-cases the flag
twice — declaring under the function's own real (unmangled) name with a
direct scalar/pointer C-ABI type mapping instead of Nether's internal
mutable/aggregate-by-pointer convention, and skipping body codegen
entirely. This reuses the entire ordinary call-graph reachability walk,
`HirFnId`/`MonoFnId` addressing, and `CallTarget::Fn` call-site codegen
unchanged — an `extern "C" fn` is called exactly like any other function,
except `gen_call` widens/narrows `bool` arguments/return to/from `i8` at
the boundary specifically for `is_extern` targets, the same class of fix
Stage 4 needed for the runtime poll ABI (`i1` is never safe to use across
a real C ABI).

**Stage 6 — concurrency safety + package manager + CLI — done.**
`runtime/arc`'s strong/weak counts are now `AtomicI64`, mirroring
`std::sync::Arc`/`Weak`'s own dealloc design exactly (the weak count
starts at 1, an implicit reference released by whichever `release` call
drops the strong count to zero — funnels the dealloc decision through a
single counter's fetch-sub instead of two independent counters racing
each other, which a naive port of the old single-threaded "check both
counts, dealloc if both zero" logic would double-free under real
concurrency). Verified with real multi-thread stress tests in
`runtime/arc`'s own suite (several `std::thread::spawn` threads
hammering retain/release/weak-upgrade on one shared allocation), not
just a code-shape check — this stage's own version of Stage 4/5's
established lesson that ARC correctness bugs only surface by actually
running contended code.

`Send`/`Sync` (`compiler/typecheck/src/send_sync.rs`) are auto-derived,
not opt-in like `Eq`/`Hash`/`Clone` — computed structurally, true by
default, false only where a field makes it unsafe, overridable only
through `unsafe impl TypeName Send { }`/`unsafe impl TypeName Sync { }`
(reuses `ImplBlock`'s existing AST shape with one new `is_unsafe: bool`
field; a plain, non-`unsafe` `impl Send`/`Sync` is rejected outright).
`Sync` is far stricter than a naive Rust-mirroring reading would suggest
and is where the real design work was: an ordinary `T` can have any
number of live ARC handles, any one of which can mutate a field in place
with no cross-alias exclusivity the language enforces (unlike
`std::sync::Arc<T>`, which only ever exposes `&T`) — proving a nominal
type is never mutated anywhere would need real whole-program analysis,
which this design deliberately avoids by instead making every nominal
type (`struct`/`enum`, `Array<T>`) `Sync` *only* via `unsafe impl`,
never auto-derived, full stop. `Send` for an ARC-domain type needs
fields both `Send` *and* `Sync` (mirrors `Arc<T>: Send` needing `T: Send
+ Sync` — other aliases may remain live on the origin thread); a unique
(`:T`) type needs only `Send` fields (mirrors `Box<T>: Send`, no
aliasing hazard).

`thread.spawn(move () => { ... })` (new `runtime/thread` crate, plain
`std::thread::spawn`, no Tokio dependency — architecturally independent
of `task.spawn`'s cooperative single-runtime model) requires a
zero-parameter `move` closure literal and a `()`-returning body (a
documented v1 scope decision — an arbitrary return type would need a
per-call-site result-boxing wrapper function in codegen, mirroring
`Task<T>`'s own frame machinery, judged not worth the complexity for
this stage). Reuses the ordinary closure-construction/dynamic-call ABI
unchanged (`env`'s own field 0 is the code pointer, exactly
`CallTarget::Dynamic`'s existing technique) — no new MIR `Rvalue`
needed. `Type::Thread(Box<Type>)` mirrors `Type::Task`'s own footprint
at every touch point (a plain heap pointer, `AllocKind::Heap`) but
carries none of `Task`'s async-frame machinery, since `.join()` is a
plain blocking call, never a suspension point.

Spawned-borrow diagnostics (`task.spawn` *and* `thread.spawn`) reuse the
exact origin-classification rule `check_returned_reference_origin`
already proved sound for return statements — a captured reference whose
origin is `BorrowOrigin::Local` (owned by the spawning function itself)
is rejected exactly as if it had been returned; `BorrowOrigin::Parameter`
remains allowed. Applied to `task.spawn` too, not just the new
`thread.spawn` — before this stage `task.spawn` had *zero* capture
restriction beyond "argument has type `Task<T>`," a real, documented gap
the roadmap itself had already named, not scope creep.

`Nether.toml` local-path dependencies generalize a mechanism that
already existed for the bundled stdlib: `mod std;`/`use stdlib.*` were
two hardcoded special cases mounting an external root with its own
independent `crate_root`; both became one `external_roots: HashMap<String,
PathBuf>` map, seeded with the stdlib's two names plus every
manifest-declared dependency — no new resolution mechanism, just the
existing one keyed generically. No version resolution, no registry, no
lockfile, purely local paths, per this stage's own explicit scope.

CLI overhaul (`run`, `test`, `--release`, `--emit-ast`/`--emit-hir`/
`--emit-mir`/`--emit-llvm`) extended the existing hand-rolled argument
parser rather than adding a `clap` dependency — `nether-cli`'s
dependency footprint stayed at just `nether-diagnostics`/`nether-driver`,
consistent with this workspace's existing minimal-dependency posture
elsewhere (`runtime/thread` similarly took no new dependency beyond
`nether-rt-arc`). `--emit-ast`/`--emit-hir`/`--emit-mir` needed no driver
change at all — `CheckResult` already exposed `module`/`hir`/`mir`
publicly, just missing `Debug` impls on `HirFunction`/`MirFunction`/two
of their own field types (a small, contained gap, fixed directly);
`--emit-llvm` was the one flag needing a real `CompileOptions` field,
since `CheckResult` deliberately never keeps the LLVM module itself
around. `test`'s pass/fail framing is real process-exit-code plumbing,
verified against a genuine crash (an array out-of-bounds access, which
aborts the process) as well as the ordinary success path — not just the
happy path.

*A real bug, found only by running compiled code, not by inspecting
types:* `thread.spawn`'s own typecheck path called `check_closure`
directly instead of going through `check_expr`'s wrapping, which is what
actually records the closure expression's type into `expr_types` for
HIR's `ty_of` to read back later. The closure's own env pointer
silently defaulted to `Type::Error`'s LLVM representation (`i1`) instead
of `ptr`, produced no compiler error at all, and only surfaced as an
`inkwell` panic ("found IntValue, expected PointerValue") the first time
a `thread.spawn` program was actually compiled and run — the exact same
class of "MIR/IR-shape-correct but semantically wrong" bug Stage 4/5's
own retrospectives already flagged as only catchable by real execution,
now with its own Stage 6 instance.

*Sequencing note — resolved.* The question above (atomic ARC arguably
wanting to land before or alongside Stage 4's async runtime, since a
Tokio bridge implies cross-thread references) is resolved by keeping
Stage 4 strictly single-threaded: every Nether task, spawned or not, runs
on the one `current_thread` Tokio runtime `nether_rt_task_block_on`
builds — Tokio's current-thread scheduler never migrates a spawned task
to a different OS thread, so `runtime/task`'s `SpawnedTask` adapter can
soundly be `unsafe impl Send` without needing atomic ARC at all (the impl
never actually crosses a real thread boundary; only Tokio's own
type-system requirement is being satisfied, not a genuine concurrency
need). This is a real invariant, documented at the `unsafe impl` site
itself — if Stage 6 ever moves this runtime to a multi-thread scheduler,
that `Send` impl becomes unsound and must be revisited together with
atomic ARC, exactly the dependency this note originally flagged. Until
then, Stage 4 delivers real *concurrency* (interleaved progress on one
thread) without *parallelism* (simultaneous execution across threads),
which needs no atomic ARC.

**Stage 7 — syntax cleanup — done.** Formalizes what had been an
"unstaged" grab-bag into a real stage, once it turned out most of its
five items needed real design work rather than being drive-by fixes.

*Variadics-as-fixed-array — already done, no code change needed.* Turned
out Stage 3 had already made variadic parameters lower to a hidden
const-generic fixed array (`nether_hir::lower::expr::lower_variadic_aware_args`),
confirmed by a pre-existing, already-passing typecheck test. Only a stale
doc comment (still describing the old `Array<T>` desugaring) needed fixing.

*Block comments — done.* `/* ... */`, non-nesting (first `*/` closes it
regardless of any `/*` seen since — the simpler C/Go/JS/Kotlin convention,
not Rust/Swift's nesting one), reported as an unterminated-comment
diagnostic on EOF. No parser/HIR/MIR changes — purely a `compiler/lexer`
addition mirroring the existing `//`/`///` scanning.

*Fixed-array vs. growable-array literal split — done.* `[1, 2, 3]` now
unconditionally builds `Array<T>` and `{1, 2, 3}` unconditionally builds
`{T, N}` — before this stage, `[...]` ambiguously coerced into either
representation depending on expected-type context. New
`ExprKind::FixedArray` in the AST, a `{`-based `parse_fixed_array_expr`
mirroring `parse_array_expr`, and a typecheck split (`check_array`/
`check_fixed_array` sharing a `check_array_elems` helper) were the only
new surface needed — confirmed by direct inspection that
`nether_mir::build::expr`'s `MonoExprKind::Array` handling already
branched on `expr.ty`'s `Type::FixedArray`-ness to choose
`Rvalue::Tuple` vs. `Rvalue::Array`, so HIR/MIR/codegen needed zero new
code, only the new AST/parser/typecheck surface. Verified end-to-end: a
real compiled/executed test proving `.push()` works on `[...]` and direct
indexing works on `{...}`.

*Newline-aware optional semicolons — done.* A Go-style ASI: the lexer
synthesizes a real `;` token when the last-emitted token is one of a
fixed trigger set (identifier, literal, `self`, `return`/`break`/
`continue`, or `)`/`]`) and a newline follows, gated by three suppression
conditions — inside `(...)`/`[...]` (`paren_bracket_depth`), inside a
`struct`/`enum` declaration's field list (`brace_suppresses_asi`, keyed
off the preceding `struct`/`enum` keyword), and whenever the very next
non-whitespace character is `}` (protects Nether's tail-expression block
semantics — inserting a semicolon right before a block's own closing
brace would silently discard its value, not just risk a parse error).
`]` closing a `#[...]` attribute is excluded from the trigger set too
(`bracket_is_attribute`), so an attribute is never separated from its
item by a synthesized semicolon. Deliberately **not** Go's own full
trigger list: Go also triggers after a closing `}`, but Nether's parser
has nowhere that tolerates a stray/empty statement, so this is a
documented, deliberate divergence — see spec §2.5 for the resulting gap
(a `let` binding whose value is a struct/fixed-array literal ending in
`}`, on its own line, still needs its `;`).

Needed five iterative rounds — implement, run the *full* workspace test
suite, find a real regression against actual existing source, fix,
repeat — before reaching a stable design; each of the five real bugs
found was caught by the full suite, not a synthetic example, matching
this project's own established "real bugs only surface by actually
running things" lesson (Stage 4/5/6's own retrospectives above):
inserting a semicolon inside a multi-line parameter/argument list (fixed
by `paren_bracket_depth`); inserting one after an `impl` block's inner
method's closing `}`, read by the item parser as a stray token before
the `impl` block's own closing `}` (fixed by excluding `}` from the
trigger set entirely, rather than Go's own choice to include it); inserting
one inside a struct declaration's field list, breaking the canonical
`struct Lang { name String }` spec example (fixed by
`brace_suppresses_asi`); inserting one right before a block's own closing
`}`, which would have silently changed a closure body's tail-expression
value from a string to `()` (fixed by the unconditional "next
non-whitespace is `}`" suppression, confirmed against a real
closure-capture codegen test); and inserting one between a `#[link(...)]`
attribute's closing `]` and the `extern "C"` block it decorates (fixed by
`bracket_is_attribute`, confirmed against a real FFI end-to-end test).

*Two real, pre-existing miscompilations, found only by running compiled
code — neither caused by Stage 7, both only surfaced by its own
end-to-end tests, both since fixed in a follow-up session.*

**Match/pattern double-release.** Verifying Stage 7's driver-level test
(a loop concatenating strings) crashed non-deterministically on repeated
calls. Root cause: `nether_mir::build::pattern::lower_match` tracked a
heap-typed scrutinee's one retained credit in *two* places — the match's
own `match_scope` (so an arm that never names it still releases it) and,
redundantly, the matching arm's own bindings scope — because a top-level
`HirPattern::Binding` (every `for`-loop desugaring's single arm, among
other shapes) doesn't materialize a fresh local at all; it hands the
arm its binding via the scrutinee's own local, unchanged. Both scopes'
release logic then fired for the *same* local: one retain paying for two
releases, freeing a still-referenced string one call early and
corrupting whatever allocation reused that freed memory on a later call.
Fixed by skipping the redundant arm-scope entry whenever a binding's own
local is literally the scrutinee's — see
`compiler/mir/src/build/pattern.rs`'s `lower_match` and the (formerly
`#[ignore]`d, now real, passing) regression test in
`compiler/driver/tests/driver_tests/stdlib_and_specialization.rs`.

**Non-entry-block `alloca` on AArch64.** A struct field read after two
`mut self` method calls went stale (reflected only one mutation)
whenever the *same function* also called any other separate function
anywhere in its body — unrelated to the struct entirely, merely being
present was enough; confirmed unrelated to ASI by reproducing it against
the pre-Stage-7 baseline with fully explicit semicolons. Root cause,
found with `otool -tv` disassembly rather than by inspecting LLVM IR
(already unoptimized and correct on paper): several `nether_codegen`
helpers for a scratch value only known to be needed partway through
building a block — a discarded `()` statement value, an aggregate call
result, a struct/tuple/variant/array-literal construction slot — called
`self.m.alloca(...)` directly at whatever block the builder happened to
be positioned in. LLVM only treats an `alloca` in the function's *entry*
block as a fixed-offset stack slot; anywhere else it must lower to a
genuine dynamic stack adjustment, never freed before the function
returns. On this project's own AArch64 target, that lowering emits a
spill store to the (unmoved) current stack pointer even for a
zero-sized `{}` — which landed exactly on the bottom word of the
function's fixed frame, silently clobbering whatever local happened to
live there (here, a cached `&mut self` receiver address, cached once to
reuse across two `counter.increment()` calls — the second call read
back garbage instead of `counter`'s real address, losing the mutation).
Fixed by hoisting every such scratch slot into the entry block: a new
`nether_llvm::ModuleCx::entry_alloca` (saves the builder's position,
repositions before the entry block's first instruction, allocates,
restores) backs both a single shared `FnCodegen::unit_slot` (sound
because `{}` carries no data for two "instances" to ever conflict over)
and a per-call-site `FnCodegen::entry_alloca` helper used everywhere
else a scratch aggregate slot had been allocated at the current block.

*Mutable closure captures — done, with two documented v1 scope limits.*
`mut (...) => {...}` (a new closure-level modifier occupying `move`'s own
keyword slot — the two can't combine) captures every `mut`-declared outer
local the body touches by reference instead of by value: `HirCapture`/
`MonoCapture` gained a `CaptureMode::ByValue`/`ByRef` field (new shared
`nether_typecheck::CaptureMode`, since no single existing crate's local-id
space spans HIR through codegen). Classification happens twice,
independently, in the two places that already track per-local
mutability: `nether_hir::Lowerer` (a new `mutable_locals: HashSet<HirLocalId>`
accumulated top-down as a function body lowers, exactly mirroring how
`locals_map` itself accumulates) and `nether_typecheck::check_closure`
(reusing `Checker.locals`'s existing mutability flag) — the latter purely
to drive the spawn-borrow-style escape check below, since typecheck has
no access to HIR's own id space to consult HIR's decision directly.

The by-reference mechanism itself needed no new MIR/codegen primitive:
`MonoExprKind::Borrow`/`Rvalue::AddressOf` — the same machinery `&raw
mut`/stored `:&mut T` borrows already use — already produces an
ARC-exempt `Type::MutRef` pointer value for free, so the only new
codegen work was teaching the closure-body prologue to `load` that
pointer once and use *it* as the captured local's own storage (instead
of aliasing the environment field's own slot, by-value capture's
existing behavior) — transparent to the local's own declared type, which
stays the plain element type throughout typecheck/HIR/the callee's own
body. A new `nether_mir::FnBuilder::declare_local_aliased` forces
`needs_drop = false` on such a local regardless of its type's own
managed-content-ness, since its storage aliases the *outer* local's own
memory and never owns a fresh reference to anything.

Two real, disclosed v1 scope limits, not silent unsoundness:

- **Heap-category captures stay by-value.** `nether_codegen::function::
  address_of_place`'s existing, load-bearing rule for a bare heap-typed
  local deliberately returns the value's *own* pointer (an ARC reference
  already IS a handle to the shared object — that's what makes ordinary
  field mutation through a `:&mut T` work) rather than the address of the
  *variable's own storage slot* a by-reference capture needs in order to
  let the closure reassign the outer binding itself. Reusing it for a
  heap capture would alias the object, not the binding — this was caught
  by a real segfault (a `mut`-capturing closure reassigning a captured
  `String`) during Stage 7's own verification, not a synthetic example,
  the same "run it for real" lesson as every earlier stage's own
  retrospective. Fixed by gating promotion on `alloc_kind(ty) ==
  AllocKind::Stack` in both classification sites — a heap-category `mut`
  capture silently falls back to today's by-value behavior instead
  (still fully correct for field mutation/mutating method calls, since
  those already worked through the shared pointer before this stage;
  only *reassigning* the captured binding itself silently doesn't write
  back). Closing this needs a real "address of a variable's own slot"
  primitive distinct from "address of what a reference already points
  to."
- **Escape checking covers only a directly returned closure literal.**
  A `mut`-capturing closure holds the raw address of each by-reference
  capture's own storage for as long as the closure value is alive — sound
  only while it never outlives the stack frame that storage lives in.
  `check_return` rejects `return mut () => {...};` directly (a new
  `Checker.closure_mut_captures: HashMap<NodeId, Vec<LocalId>>`,
  populated by `check_closure`, consulted unconditionally — unlike the
  existing returned-reference-origin check, which only fires when the
  function's own return type is itself `Ref`/`MutRef`, this fires for
  *any* returned closure regardless of return type, since a captured
  plain local's own address is never safe to hand back — no `Parameter`
  vs `Local` origin distinction applies the way it does for an ordinary
  reference parameter, because even a plain *value* parameter's own
  stack slot lives in the callee's own frame). Storing the closure in a
  `let` and returning that binding instead, or smuggling it out through a
  struct field, an array, or a plain function argument, is not tracked —
  a real, disclosed gap the same shape as this project's other
  documented v1 slices (Stage 4's frame-liveness gap, Stage 5's aggregate
  FFI gap), not a silent one.

---

## 3. Verifying a stage is complete

Each stage should leave `cargo test --workspace` fully green, or every
exception explicitly `#[ignore]`d with a comment naming the specific stage
expected to fix it — never a silently broken test. Development and release
build modes must always produce identical *behavior* for any given program,
even where Stage 3 onward makes their *codegen strategy* diverge (witness
tables vs. monomorphization) — this should be covered by tests that compile
and run the same program both ways once dev/release modes actually diverge.
