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
  Safety входит в interface/override signature checks. Unsafe function values
  пока отклоняются E0501, чтобы не стирать safety в safe callable.

## Не завершено

- Полная NMM acceptance matrix, отдельный MIR verifier, явное представление
  O/R/W tokens вместо нынешних view/mutable flags и полный backend audit.
- Raw pointers/storage operations и FFI: parser знает часть синтаксиса,
  checker явно отклоняет неподдержанную семантику даже в unsafe.
- Полное исполнение inheritance, upcast/dyn, пользовательские Vec/String.
- Weak, capturing closures, channels/async. Их целевой контракт описан NMM;
  наличие документа не означает наличие source implementation.

## Проверки

Наборы: `compiler/core/tests/nmm.rs`, `unsafe.rs`, borrowed storage/results,
entries, interfaces/inheritance, lifecycle/native; frontend parser и CLI tests.
CI проверяет fmt, Clippy, workspace Debug/Release и diagnostic runtime.
Новая матрица — [memory-cases.md](memory-cases.md); план — [mvp_plan.md](mvp_plan.md).
Проверка 2026-10-25: полный workspace Debug и Release прошёл без failures;
один performance probe остаётся ignored. После дополнения NMM cases наборы
nmm/unsafe/modules отдельно перепроверены в обоих профилях — по 25 тестов.
Diagnostic runtime — 6 тестов, Clippy all-targets/all-features с `-D warnings`,
fmt, whitespace и локальные ссылки документации прошли. Это регрессионная
приёмка поддержанного subset, не закрытие полной NMM/unsafe матрицы.

Последний опубликованный performance run — 2026-10-23: четыре из шести
микробенчмарков близки к Rust; collatz — 1.75× и fib — 1.89× времени Rust.
Это результаты предыдущей сборки, не замер новой базовой линии.
[Данные и ограничения](../../performance/RESULTS.md).
