# Nether 0.1 — план согласования и приёмки

Редакция 2026-10-25. Единственная модель памяти — [NMM](../core-memory-managment.md).
[Unsafe](../unsafe.md) — явная граница обязательств внутри этой модели.
Старый трек C0–C9 заменён этим планом; его отметки не считаются приёмкой NMM.
Расширение языка не предшествует стабилизации памяти.

## P0 — единый контракт

- [x] NMM `nmm-0.1` принята как единственная модель 0.1.
- [x] Удалены из активной документации альтернативы move по будущему use,
  readonly promotion и owning-циклов через groups.
- [x] Разделены нормативный контракт и реализованный subset.
- [x] Определены Rust-like unsafe-обязательства и safe API boundary.

## P1 — синхронное ядро NMM

- [x] Non-Copy aliases, readonly запреты и sequential reborrow.
- [x] Borrowed параметры по умолчанию; вывод consuming-effect до fixed point.
- [x] Consuming API требует O; alias не выбирает обходной borrowed entry.
- [x] `var` non-Copy параметр не потребляет сам вход.
- [ ] Полная N01–N34 приёмка: branches/loops, results/storage, initialization,
  replacement/panic, generics/modules/indirect calls и observable drop traces.
- [ ] Отдельный MIR verifier O/R/W, provenance, loans и cleanup transitions.
- [ ] Аудит generated Rust и runtime по тем же инвариантам.

## P2 — unsafe boundary

- [x] `unsafe fn` и явные лексические `unsafe`-блоки.
- [x] Unsafe direct/generic/method call запрещён снаружи блока, в том числе
  внутри тела unsafe fn. Обычные NMM checks не отключаются.
- [x] Unsafe callable нельзя выдать за safe function pointer; до реализации
  safety в callable types такой pointer отклоняется.
- [ ] Raw pointers/address/deref/casts, allocation/storage intrinsics.
- [ ] Raw ↔ safe/owned adapters: provenance, initialization, exactly-once drop,
  invalidation, bounds/alignment, ZST, OOM и panic safety.
- [ ] FFI safety/ABI/resource contracts, callback и unwind boundaries.
- [ ] Негативная и native O0/O3 приёмка всех unsafe operations.

## P3 — приёмка базовой линии

- [x] Полный workspace Debug и Release, runtime diagnostic mode, Clippy и fmt (2026-10-25).
- [x] Native O0/O3 сравнение HIR/MIR результатов, drop order и allocations
  поддержанного subset (2026-10-25); полная матрица P1 остаётся открытой.
- [x] Все опубликованные примеры соответствуют профилю и проверяются автоматически.
- [ ] Совместимость docs/metadata/backend; unsupported не принимается молча.
- [ ] Измерены время, allocations и code size; регрессии опубликованы.

Отметки проверок заполняются по фактическому результату, а не по старым числам.
P0–P3 должны быть закрыты до заявления о завершении базовой NMM-приёмки.

## После стабилизации ядра

Сначала завершается нужный unsafe/storage subset и библиотечная основа.
Weak и acyclic ownership graphs, closures, channel transfer и async добавляются
по NMM с отдельными acceptance cases. Inheritance/upcast/dyn не заменяют
проверку памяти и не являются текущим приоритетом. Общие направления —
[roadmap](roadmap.md); фактическое состояние — [status](status.md).
