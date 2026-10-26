# Nether 0.1 — изменения базовой линии

2026-10-25. Эта редакция заменяет прежний draft 0.1.

- NMM `nmm-0.1` — единственная модель памяти.
- Existing non-Copy binding создаёт alias, а не move по последнему use.
- O, R, W независимы; readonly не превращается в writable через ownership.
- Non-Copy `var` parameter — borrow; consuming ordinary parameter требует O.
- Убран обход consuming-effect через borrowed entry для alias.
- Добавлены unsafe fn и явные unsafe blocks; безопасные NMM проверки сохраняются.
- Unsafe callable пока нельзя превратить в safe function value.
- Старые альтернативные memory rules, планы и устаревшие acceptance counts
  заменены единым [планом](0.1/mvp_plan.md) и [статусом](0.1/status.md).

Добавлены MIR verifier синхронного subset и негативные mutation tests;
проверяются ownership, access, provenance/effects и cleanup exits.
Raw operations/FFI и weak/closures/channels/async не объявляются готовыми. Контракт unsafe — [unsafe.md](unsafe.md).
История прежней редакции доступна в Git; она не является действующей спецификацией.

Backend optimizations: structured infallible scalar entries и frame allocation
для ограниченного nonescaping class subset в wrapping mode. Dynamic panic,
identity, destructors и escaping storage сохраняют fallback. Приёмка:
workspace Debug/Release по 202 tests, native O0/O3 и counting allocator.
[Повторные замеры](../performance/RESULTS.md): collatz/fib близки к Rust,
локальные классы без black_box больше не требуют heap allocations.

Расширение языка: unsafe callable type `unsafe (T) => R` сохраняет safety через
specialization/storage/imports; pointer можно получать вне unsafe, вызывать —
только в явном блоке. Реализован statement `loop` с break/continue/return и
cleanup; value-result loop ещё открыт. Весь объём дальнейшей реализации —
[план](0.1/implementation-plan.md).


## Raw scalar pointers (2026-10-25)

Добавлены невладеющие `*T`/`*var T`, адреса scalar places, unsafe read/write,
element offset и writable → readonly cast. Safety/capability checks действуют
в checker и на MIR boundary; оба reference interpreters моделируют pointer
provenance без удержания storage. Native O0/O3 использует Rust raw pointers.
Новые проверки охватывают array one-past/backtracking, aliases, generic storage,
borrowed fields, dangling/bounds diagnostics и подмену MIR capabilities.
Allocation/initialize/take/drop и non-scalar raw доступ остаются открытыми.


## Owned scalar storage (2026-10-25)

Compiler-owned `core/intrinsic` предоставляет opaque non-Copy RawStorage для
scalar/Unit, allocation/init/take/replace/drop_slot/release и safe layout queries.
Module linking сохраняет закрытые intrinsic IDs через reexports; имена user
functions/types не получают special semantics. Release consuming-only; mutation
требует W. Native allocator проверяет layout, использует ZST sentinel и возвращает
Nether panic при OOM без initialization bitmap. Reference interpreters проверяют
slot protocol. Добавлены safe fixed-capacity buffer example и native tests для
normal/panic/OOM deallocation. Non-scalar/borrowed mode и range/view APIs открыты.
