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
| `compiler/monomorphization` | **keep** | Always-monomorphized generics remain the Stage 1 strategy; dev-mode witness tables are a Stage 3 addition alongside this crate, not a replacement of it. |
| `compiler/mir` | **keep, pass-through only** | The ARC-insertion pass (`arc.rs`) already keys off a single `AllocKind` chokepoint — decoupling from casing turned out to need no logic change, only new pass-through match arms. |
| `compiler/llvm` | **keep** | Thin, well-isolated `inkwell` facade. No dev/release split exists yet (Stage 3). |
| `compiler/codegen` | **refactor** | Real LLVM IR generation, drop-shim generation reused as-is. Stage 1 adds reference codegen for heap-category types only (§3.1 of the spec). |
| `compiler/driver` | **refactor** | `module_loader.rs` gets the `.nr`-only migration and legacy-`.nt` diagnostics; the rest of the pipeline wiring (`lib.rs`) is largely unaffected by Stage 1. |
| `runtime/{arc,string,array,io}` | **keep for Stage 1** | Clean, minimal, but explicitly single-threaded/non-atomic today — the spec requires atomic ARC eventually (Stage 6). No change needed until concurrency lands. |
| `cli` | **keep, minor scope** | Hand-parsed args, `check`/`build`/`ast` only. Subcommand overhaul (`run`/`test`/`--release`/`--emit-*`) is Stage 6, alongside the package manifest that gives those commands something to build against. |

---

## 2. Stage sequence

**Implementation snapshot (2026-08-18).** Stages 1 and 2 are complete. Stage 3 has
started with the narrow `#[allow_pascal_case]` type-alias attribute, including
focused diagnostics for unknown attributes and invalid targets. Stage 2 has
flow-sensitive move checking, reference parameters and receiver calls,
lexically scoped stored heap borrows, plus the first three `to(value)`
ownership-domain conversions. Returned-reference origins are checked and
propagated through named-function call chains, including storage in `let`;
default-private visibility with explicit `pub` is implemented for declarations,
imports/re-exports, fields, and methods;
Method/closure origin summaries, loop/local-closure-sensitive last-use
shortening, inline-reference codegen, explicit `move` closures and `to<T>`
are implemented. Unique heap values use an unrefcounted runtime allocation
and promote explicitly into ARC for `:T -> T`.
Stages 4–6 have not started as staged projects. Beyond Stage 3's first slice,
the pre-existing compiler already has
the baseline trait system and limited concrete instance-method
specialization described elsewhere in the docs.

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

**Stage 3 — trait system extensions + dev-mode generics — in progress.** The
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
is implemented for integer/bool/char, tuple, nested `Hash`, and unique fields;
the compiler builtin `hash(value)` returns a deterministic `u64`, while
unsupported and cyclic graphs are diagnosed. String and floating-point fields
remain excluded until their stable hashing semantics are specified. Remaining:
associated
types/constants, const generics, existential
(`any Trait`) and opaque (`some Trait`) types with existential-safety
checking, and development-mode
witness-table/dictionary generics dispatch (so `nether build` stops
monomorphizing every generic body — release retains full
monomorphization/devirtualization/specialization). Also folds in the
unstaged syntax cleanup this doc tracks below (optional semicolons, inline
array vs. vector literal distinction, variadics-as-fixed-array).

**Stage 4 — async/await + Tokio runtime bridge.** `async fn`, prefix
`await expr`, compiler-generated state-machine lowering, the Nether runtime
ABI boundary to a Rust/Tokio-backed runtime (`task_spawn`, `timer_sleep`,
etc. — Tokio itself never exposed to Nether source), `task.spawn(...)`.

**Stage 5 — unsafe, raw pointers, C ABI FFI.** `unsafe` blocks/functions,
`*mut T`/`*const T` and their owned forms, `extern "C"` declarations with
FFI-safety checking (only ABI-safe types cross the boundary), `#[link]`.

**Stage 6 — concurrency safety + package manager + CLI.** Atomic/thread-safe
ARC, compiler-derived `Send`/`Sync` (+ `unsafe impl` escape hatch),
`thread.spawn(...)` raw OS threads, spawned-borrow diagnostics (rejecting a
short-lived borrow captured by a detached thread/task without requiring
`'static` syntax), `Nether.toml` package manifest + local-path dependency
graph, CLI overhaul (`run`, `test`, `--release`, `--emit-ast`/`--emit-hir`/
`--emit-mir`/`--emit-llvm`).

*Sequencing note:* atomic ARC arguably wants to land before or alongside
Stage 4's async runtime, since a Tokio bridge implies cross-thread
references. This is flagged here as an open sequencing question for
whoever scopes Stage 4 in detail — not resolved by this document.

---

## 3. Unstaged syntax cleanup

Small, low-risk syntax items the full spec calls for for that don't yet have
a stage assignment. Pick these up opportunistically, most naturally
alongside Stage 3's other grammar work:

- Newline-aware optional semicolons (spec §2.5) — needs real lexer/parser
  work (not a text preprocessing pass), not merely dropping a token
  requirement.
- Block comments (`/* ... */`).
- Distinguishing fixed-size inline-array literals (`{T, N}`) from growable
  vector literals (`[]`) — today `[]` means `Array<T>` unconditionally.
- Variadic parameters lowering to a fixed-size compile-time collection
  (`...T` as a hidden const-generic-sized array) instead of today's
  `Array<T>` desugaring.
- Mutable closure captures (`FnMut`-style capture-by-reference).

---

## 4. Verifying a stage is complete

Each stage should leave `cargo test --workspace` fully green, or every
exception explicitly `#[ignore]`d with a comment naming the specific stage
expected to fix it — never a silently broken test. Development and release
build modes must always produce identical *behavior* for any given program,
even where Stage 3 onward makes their *codegen strategy* diverge (witness
tables vs. monomorphization) — this should be covered by tests that compile
and run the same program both ways once dev/release modes actually diverge.
