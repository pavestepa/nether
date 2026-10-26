# Nether 0.1 — roadmap

Редакция 2026-10-25. NMM — единственная модель памяти. Исполняемый план —
[mvp_plan.md](mvp_plan.md), семантика — [NMM](../core-memory-managment.md),
unsafe boundary — [unsafe.md](../unsafe.md).

1. Зафиксировать единый контракт O/R/W и устранить прежние противоречащие правила.
2. Закрепить синхронное ядро NMM compile-pass/fail и HIR/MIR/native проверками:
   aliases, loans, consuming effects, results/storage, branches/loops и cleanup.
3. MIR verifier синхронного subset реализован; поддерживать проверки trusted
   Rust backend/runtime при каждом новом primitive — [границы](nmm-verification.md).
4. Завершить Rust-like unsafe: raw operations, storage и safe adapters;
   каждый unchecked primitive имеет явные обязательства и приёмочные случаи.
5. Пройти Debug/Release/diagnostic/native O0/O3 и публиковать ограничения.
6. После стабилизации — библиотечные Vec/String, weak и структуры с
   ацикличным владением; затем closures, channels и stackless async по NMM.

Развитие class inheritance/upcast/dyn и platform APIs ведётся после памяти.
Региональные оптимизации сохраняют identity, O/R/W и время/порядок destructor.
Никакой этап не вводит GC/strong ARC, runtime borrow checker, hidden clone или
выбор ownership во время исполнения. Raw pointers в unsafe не являются второй
моделью владения и сами по себе не владеют payload.

Выпуск 0.1 не объявляется завершённым до приёмки выбранного subset и честной
фиксации unsupported features. Производительность сравнивается с эквивалентным
Rust, а не выводится из использования rustc.

Подробный статус всего запрошенного расширения — [implementation-plan.md](implementation-plan.md).
