# Nether 0.1 — состояние реализации

Редакция 2026-10-25. Новая базовая линия — **NMM `nmm-0.1` + Rust-like unsafe**.
Это proof of work; стабильный выпуск и soundness не заявлены.

## Реализовано

- Rust workspace без внешних crates, frontend, imports/privacy, constants,
  typed HIR и CFG MIR, два интерпретатора, Rust backend и CLI.
- Struct/enum/tuple/fixed array, class/new/constructors/destructors, generics,
  статические interfaces, function values, borrowed results/storage.
- Deterministic cleanup: initialization, replacement, partial construction,
  normal/exceptional exits, первая panic и abort при повторной panic.
- NMM aliases вместо move по отсутствию будущего use; запрет R → W,
  sequential reborrow и ordinary R / `var` W параметры.
- Consuming-effect выводится до fixed point; все consuming entries требуют O.
  Alias в consuming API отклоняется `NMM_NO_OWNER_TOKEN`, включая indirect calls.
  Borrowed storage использует непотребляющий API и сохраняет provenance.
- `unsafe fn` и statement `unsafe` blocks. Direct/generic/method unsafe calls
  требуют явного блока, в том числе внутри unsafe fn. `main` всегда safe.
  Safety входит в interface/override signature checks и callable types.
  `unsafe (T) => R` сохраняется через generics/storage/imports; indirect call
  требует явного unsafe block. Safe/unsafe callable types не смешиваются.
- Scalar raw pointers: `*T`/`*var T`, `&`/`&var`, read/write, element offset
  и writable → readonly cast. Адреса невладеющие; raw доступ требует unsafe.
  [Контракт и ограничения](../unsafe.md).
- Owned scalar/Unit RawStorage через `core/intrinsic`: allocation, initialize,
  take, replace, drop_slot, consuming release и safe layout/capacity queries.
  Closed fields, NMM cleanup, zero capacity/ZST sentinel, layout/OOM panic;
  [границы storage](storage-contracts.md#6-реализованный-owned-scalar-subset).
- Statement `loop` с break/continue, cleanup и распознаванием diverging loop
  без выхода; `break value` пока не реализован.

- MIR verifier после checker и перед Rust backend: ownership duplication,
  capability/ABI, provenance/effects, initialization и cleanup exits.
  [Область проверки](nmm-verification.md).

## Не завершено

- Полная нормативная NMM acceptance matrix для будущих primitives;
  исчерпывающее доказательство soundness backend/runtime.
- Non-scalar raw/storage operations, borrowed storage mode и FFI:
  неподдержанная семантика отклоняется даже в unsafe.
- Полное исполнение inheritance, upcast/dyn, пользовательские Vec/String.
- Weak, capturing closures, channels/async. Их целевой контракт описан NMM;
  наличие документа не означает наличие source implementation.

## Проверки

Наборы: `compiler/core/tests/nmm.rs`, `unsafe.rs`, borrowed storage/results,
entries, interfaces/inheritance, lifecycle/native; frontend parser и CLI tests.
CI проверяет fmt, Clippy, workspace Debug/Release и diagnostic runtime.
Новая матрица — [memory-cases.md](memory-cases.md); план — [mvp_plan.md](mvp_plan.md).
Проверка 2026-10-25 после добавления scalar raw pointers: полный workspace Debug и Release
— по **210 passed, 0 failed, 1 ignored** (performance probe). Включены 11
негативных MIR mutation tests и 8 NMM tests с несколькими сценариями в каждом;
native cases исполняются при O0/O3. Diagnostic runtime — 6 passed.
Clippy all-targets/all-features с `-D warnings`, fmt, whitespace и ссылки
30 Markdown документов прошли. Проверки относятся к синхронному subset;
приёмка будущих weak/closures/channels/async, non-scalar raw и storage operations
остаётся открытой. Три raw-pointer теста содержат positive/negative/bounds/dangling
сценарии; последние изменения проверок аннотаций повторно проверены отдельно.

Backend получил conservative structured scalar entries и frame allocation
для простых неубегающих классов в wrapping mode. Checked/potentially-fallible
и сложные ownership cases сохраняют fallback; [условия](rust-backend.md#консервативные-оптимизации-2026-10-25).
Пять optimization tests проверяют eligibility, normal/panic semantics и ноль
heap allocations для frame-классов при O0/O3.

Performance run после оптимизаций 2026-10-25: collatz 1.003× Rust, fib 0.998×;
class без black_box 1.117× вместо 44.79×. Array/LCG/struct близки к Rust.
Класс с Rust black_box теперь асимметричен: Nether устраняет heap allocation,
Rust ограничен barrier, поэтому это не сравнение скорости allocator.
[Данные до/после и ограничения](../../performance/RESULTS.md).
