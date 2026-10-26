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
Unsafe callable type записывается `unsafe (T) => R`. Получение и хранение
такого value разрешены вне unsafe; indirect call требует явного unsafe block
(E0500), в том числе внутри unsafe fn. Safety сохраняется при generic
specialization, type inference, возврате, хранении в aggregate, импортах и
reexports. Unsafe pointer нельзя привести к safe `(T) => R` (type mismatch).
В текущей реализации safety типов должна совпадать точно: автоматического
приведения safe function к unsafe callable также пока нет.

```nether
unsafe fn trusted(n:i32):i32 {return n}
fn main():i32 {
    let callback:unsafe (i32)=>i32=trusted
    unsafe {return callback(7)}
}
```

Взятие адреса функции не вызывает её и не даёт разрешения на последующие
unsafe операции. Safe wrapper обязан сам выполнить preconditions вызываемой
unsafe функции; общий адаптер произвольного unsafe callback обычно тоже unsafe.

## Что checker продолжает проверять

Обычные safe пути сохраняют O/R/W, lifetime, initialization, uniqueness,
finalizing и exactly-once cleanup. Unsafe не разрешает `var` из readonly,
второй owner-token, consumption alias, escaping safe view или owning-cycle.
Ошибки типов и private access тоже не отключаются. Неизвестная операция
остаётся unsupported; обёртка unsafe не делает её реализованной.

## Raw operations: нормативный контракт

Реализованный скалярный subset описан ниже; allocation/storage primitives для scalar/Unit описаны в [storage contracts](0.1/storage-contracts.md#6-реализованный-owned-scalar-subset). `*T` — raw read pointer, `*var T` — raw write pointer.
Pointer не владеет целью и не продлевает lifetime. Копирование pointer не
копирует payload и не создаёт owner. Получение writable адреса из обычного
safe пути требует W; cast не превращает R в W.

Следующие действия требуют unsafe boundary:

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
call checks, unsafe callable types и сохранение NMM проверок.

Raw types `*T` и `*var T` — Copy, невладеющие значения; их можно хранить,
возвращать и передавать через generics. Сейчас получение адреса и доступ к
pointee поддержаны для integer, bool, char, f32 и f64. Адрес scalar field или
элемента fixed array также поддержан. `&place` и `&var place` не требуют unsafe,
но требуют существующего initialized place; `&var` проверяет W и активные loans.
Временное значение не становится адресуемым от добавления `&`.

Внутри явного unsafe блока доступны:

- `*pointer` — чтение scalar; `*pointer = value` — запись через `*var T`;
- `pointer + offset`, где offset имеет тип `isize`, — смещение в элементах T;
  отрицательное значение смещает назад. Результат остаётся внутри того же
  массива либо one-past; разыменование one-past запрещено. Для отдельного
  scalar область состоит из одного элемента;
- `pointer as *T` — снятие writable capability при том же pointee type.
  Readonly → writable, reinterpret и integer/address casts отклоняются.

```nether
fn main():i32 {
    var values={2,5,9}
    let first=&var values[0]
    unsafe {
        let second=first+1
        *second=7
        let readonly=second as *i32
        return *readonly
    }
}
```

Raw pointer не является safe view: compiler не выводит lifetime/loans для
скопированного адреса. Автор unsafe обязан обеспечить живой initialized target,
правильную provenance, bounds, alignment и отсутствие конфликтующего доступа
на момент каждой операции. Тип pointee и запрет R → W сохраняются статически.
Возвращать адрес локального значения можно, но разыменовывать его после конца
lifetime нельзя. Reference interpreters диагностируют dangling storage и выход
за границы; это дополнительная диагностика, а не полный UB detector. Native
backend использует настоящие Rust raw pointers без runtime bounds/lifetime checks.

Non-scalar dereference, raw compound assignment, создание safe views из raw,
non-scalar allocation/storage, FFI, unsafe traits/impls и expression-form
unsafe blocks ещё не реализованы. Через `core/intrinsic` доступны owned
scalar/Unit allocation, initialize/take/replace/drop_slot/release и safe layout
queries; они сохраняют NMM ownership и требуют unsafe для изменения slots.
Полный storage subset для произвольного Vec ещё не готов.
[Пример](../examples/raw_pointers/main.nr) включает safe метод, который удерживает
W на поле объекта и локально выполняет raw-запись; произвольный raw pointer такой
метод от caller не принимает.

Проверки — `compiler/core/tests/unsafe.rs`, `raw_pointers.rs`, MIR mutation, NMM и native suites.
Критерии расширения — [план 0.1](0.1/mvp_plan.md).

Основания терминологии: [Rust Reference: unsafe keyword](https://doc.rust-lang.org/reference/unsafe-keyword.html)
и [undefined behavior](https://doc.rust-lang.org/reference/behavior-considered-undefined.html).
NMM задаёт собственные правила доступа; автоматический перенос всех деталей
Rust syntax или aliasing model этим документом не утверждается.
