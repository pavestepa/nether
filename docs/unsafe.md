# Nether 0.1 — Rust-like unsafe

Редакция 2026-10-25. Модель памяти — [NMM](core-memory-managment.md),
идентификатор `nmm-0.1`. Unsafe — граница доказательства безопасности внутри
NMM, а не альтернативная форма владения, GC или отключение checker.

## Обязательства и явный допуск

`unsafe fn` объявляет дополнительные safety preconditions для caller.
Автор функции документирует их; caller обеспечивает их до вызова.
`unsafe { ... }` утверждает, что обязательства каждой unsafe операции выполнены.
Тело unsafe fn не получает автоматического допуска: требуется явный блок.
Это соответствует Rust-like разделению объявления обязательств и их выполнения.

```nether
// Safety: caller гарантирует value >= 0.
unsafe fn trusted_nonnegative(value:i32):i32 {return value}

fn checked_nonnegative(value:i32):i32 {
    if value < 0 {return 0}
    unsafe {
        // Safety: предыдущая проверка исключила отрицательное значение.
        return trusted_nonnegative(value)
    }
}

fn main():i32 {return checked_nonnegative(7)}
```

Unsafe permission лексическое, не переходит к callee, не выходит за блок и
не меняет семантику scope, return/break/continue, panic и cleanup.
`main` всегда safe. Direct/generic/method calls сохраняют safety контракта.
Interface implementation и override не могут менять safety signature.
Unsafe функции не приводятся к safe function pointers. Пока callable type
не содержит проверяемой safety, получение такого value запрещено E0501,
включая generic/method values и получение внутри unsafe block.

## Что checker продолжает проверять

Обычные safe пути сохраняют O/R/W, lifetime, initialization, uniqueness,
finalizing и exactly-once cleanup. Unsafe не разрешает `var` из readonly,
второй owner-token, consumption alias, escaping safe view или owning-cycle.
Ошибки типов и private access тоже не отключаются. Неизвестная операция
остаётся unsupported; обёртка unsafe не делает её реализованной.

## Raw operations: нормативный контракт

Эта часть задаёт обязательства будущих raw primitives; их source execution
ещё не реализовано. `*T` — raw read pointer, `*var T` — raw write pointer.
Pointer не владеет целью и не продлевает lifetime. Копирование pointer не
копирует payload и не создаёт owner. Получение writable адреса из обычного
safe пути требует W; cast не превращает R в W.

Следующие действия требуют unsafe boundary и отдельной приёмки:

- raw dereference/read/write и образование safe view из raw storage;
- pointer arithmetic/casts с условиями provenance, allocation bounds,
  alignment, initialization, lifetime и допустимого доступа;
- unchecked initialize/take/drop, allocation adoption/release;
- вызов foreign declaration и unsafe function;
- FFI callbacks, ABI/layout и external resource operations.

Само хранение или копирование raw pointer не является передачей ownership.
Создание safe view требует действительной цели и соблюдения всех NMM loans.
Принятие allocation во владение требует отдельного trusted adapter с ровно
одним O, сведениями extent/alignment/release и согласованным cleanup.
Нельзя создать O простым cast адреса. API с raw graph links не легализует
несколько владельцев одного payload; raw links остаются невладеющими.

Автор unsafe обеспечивает отсутствие use-after-free, double free/drop,
uninitialized/invalid reads, недопустимого aliasing, гонок и выхода за bounds.
Panic/unwind не пересекает обычную C ABI boundary. UB остаётся ошибкой программы
внутри unsafe. Safe wrapper обязан сохранять гарантии для любого допустимого
safe caller, включая panic/cancellation и вызовы в неожиданном порядке.

Подробности storage — [storage-contracts.md](0.1/storage-contracts.md),
ABI — [abi.md](0.1/abi.md). Эти документы не вводят иной memory model.

## Реализованный subset и приёмка

Реализованы unsafe function/method declarations, explicit statement blocks,
call checks и сохранение NMM проверок. Raw pointers/storage/FFI, unsafe
traits/impls, unsafe callable types и expression-form unsafe blocks пока
не реализованы. Их нельзя считать доступными по одному наличию спецификации.

Проверки — `compiler/core/tests/unsafe.rs`, NMM и native suites.
Критерии расширения — [план 0.1](0.1/mvp_plan.md).

Основания терминологии: [Rust Reference: unsafe keyword](https://doc.rust-lang.org/reference/unsafe-keyword.html)
и [undefined behavior](https://doc.rust-lang.org/reference/behavior-considered-undefined.html).
NMM задаёт собственные правила доступа; автоматический перенос всех деталей
Rust syntax или aliasing model этим документом не утверждается.
