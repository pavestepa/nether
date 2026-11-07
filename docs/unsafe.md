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

Реализованный raw subset описан ниже; owned allocation/storage primitives описаны в [storage contracts](0.1/storage-contracts.md#6-реализованный-owned-subset). `*T` — raw read pointer, `*var T` — raw write pointer.
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
pointee поддержаны для integer, bool, char, f32/f64, Unit, raw pointers и
их Copy tuple/array/struct/enum композиций без class payload. Адрес scalar field или
элемента fixed array также поддержан. `&place` и `&var place` не требуют unsafe,
но требуют существующего initialized place; `&var` проверяет W и активные loans.
Временное значение не становится адресуемым от добавления `&`.

Внутри явного unsafe блока доступны:

- `*pointer` — чтение поддержанного Copy значения; `*pointer = value` — запись через `*var T`;
- `pointer + offset` / `pointer - offset`, где offset имеет тип `isize`, —
  смещение в элементах текущего T. Результат остаётся внутри того же
  массива либо one-past; разыменование one-past запрещено. Для отдельного
  scalar область состоит из одного элемента;
- `pointer as *T` — снятие writable capability при том же pointee type.
  Readonly → writable запрещён;
- `end - begin` возвращает `isize` — расстояние в элементах текущего T.
  Требуются одна живая allocation, ненулевой size T и кратное size T расстояние;
- `pointer as *U` / `pointer as *var U` — reinterpret scalar pointee и
  padding-free tuple/array/Copy struct/enum. Writable результат требует W.
  Cast сохраняет адрес и provenance, но меняет stride последующей арифметики.
- `pointer as usize/isize` и обратный cast — для non-ZST owned pointees.
  Адрес не удерживает target живым и не даёт дополнительных прав. Null и
  произвольные числа допустимы как значения указателя, но не как доказательство
  provenance. Native casts не проверяют lifetime и права; диагностические
  интерпретаторы сохраняют exposed allocation, bounds и исходный доступ;
- `*pointer += rhs` и остальные числовые compound assignments — адрес и старое
  значение читаются один раз до RHS. При арифметической panic запись не происходит.

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
на момент каждой операции. Допустимые pointee types и запрет R → W проверяются статически.
Возвращать адрес локального значения можно, но разыменовывать его после конца
lifetime нельзя. Reference interpreters диагностируют dangling storage и выход
за границы; это дополнительная диагностика, а не полный UB detector. Native
backend использует настоящие Rust raw pointers без runtime bounds/lifetime checks.

Implicit owning dereference, borrowed RawStorage, FFI,
unsafe traits/impls и expression-form unsafe blocks ещё не реализованы.
Через `core/intrinsic` доступны owned scalar/Unit/struct/class/enum storage,
initialize/take/replace/drop_slot/drop_range/release, range moves, slot addresses,
drop_in_place, read_owned/write_owned, raw-parts transfer и safe layout queries. Они сохраняют NMM ownership.
Для non-Copy class и plain Copy значений реализованы storage views и safe
`OwnedBuffer.at`/`at_mut`. `let x=buffer.at(0)` копирует Copy значение;
`let ref x=buffer.at(0)` сохраняет readonly loan, а `let ref var x=buffer.at_mut(0)`
позволяет менять slot через `*x`. Прямое возвращение view сохраняет provenance
и не позволяет вернуть view локального буфера. Inline resource views остаются открытыми.
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


## Slot addresses и raw ownership adapters

`slot_address(storage,index):*T` и `slot_address_mut(var storage,index):*var T`
требуют unsafe и проверяют `index <= capacity`. Initialized slot не требуется
для получения адреса. One-past адрес допустим, но недоступен для read/write/drop.
Pointer не удерживает storage и не создаёт safe loan; после release он dangling.
Арифметика и снятие W доступны для всех поддержанных owned storage pointee.

`drop_in_place(pointer:*var T)` требует initialized owned target и исключительного
доступа. Вызывает Nether drop glue, включая destructor и cleanup при panic.
После вызова slot uninitialized; allocation остаётся allocated. Повторный drop/read
нарушает unsafe-контракт. Byte window не является отдельным owned slot: уничтожать
нужно исходный целый элемент, а не его побайтовое представление.

`into_raw_parts(storage):(*var T,usize)` потребляет allocation capability без
освобождения. `from_raw_parts(pointer,capacity)` принимает её обратно при совпадении
исходного T, base pointer/capacity и отсутствии другого owner. До adoption cleanup
— обязанность caller. Иностранные allocations этим adapter не поддерживаются.
Reinterpret не разрешает принять allocation во владение под другим T: сначала
нужен обратный cast. Интерпретаторы проверяют переход owned/detached и capacity.

## Reinterpret представлений без padding

Integer/f32/f64 допускают все битовые комбинации. Поддержаны также непустые
fixed arrays, tuple и Copy struct, рекурсивно составленные из этих типов, если
layout не содержит промежуточного или хвостового padding. Raw pointers, class,
non-Copy struct не допускаются внутри reinterpret payload. Bool и char
поддержаны при допустимых битовых значениях: 0/1 для bool, Unicode scalar
value для char. Copy enum поддержан, если каждый вариант заполняет весь
payload и между u32 tag и payload нет padding. Неверные bool/char/tag значения
диагностируются при декодировании; native unsafe caller обязан исключить их.
Это ограничение не запрещает обычный typed raw read/write поддержанных Copy типов.

Чтение/запись сохраняет биты, а не выполняет numeric conversion. Частичная запись
меняет только выбранные байты, сохраняя остальные поля и элементы. Адрес не меняется;
`pointer + n` смещается на `n * size_of<T>()` байт для текущего T. Bounds всегда
остаются границами исходной allocation и не расширяются после cast.

```nether
#[Copy] struct Pair {first:u32;second:u32}
fn main():i32 {
    var pair=Pair {first:2,second:3}
    unsafe {
        let fields=(&var pair) as *var u32
        *(fields+1)=5
    }
    return (pair.first+pair.second) as i32
}
```

Доступ требует initialized байтов и alignment целевого T. Интерпретаторы используют
гарантированное выравнивание исходного типа: случайно более сильное выравнивание
native адреса не считается гарантией. Обратный cast допустим, если исходный объект
гарантирует нужный alignment. Невыровненные доступы и выход за bounds диагностируются.

Generic-функция может передавать уже полученный reinterpret-pointer. Cast из
произвольного `T` в конкретный тип не проходит generic body audit: один `Copy`
не доказывает отсутствие padding или допустимость всех битовых комбинаций.
Unsafe не ослабляет generic audit. Полная поддержка object representations,
padding и ZST integer/address casts остаётся открытой.

Явные `read_owned`/`write_owned` переносят владение через `*var T` и требуют
whole-slot initialization protocol. Read делает источник uninitialized, write
принимает владельца только в пустой slot. Они не снимают автоматически владение
обычного local из compiler metadata: unsafe код обязан восстановить local до
его дальнейшего safe use/cleanup. Обычное `*p` по-прежнему читает plain Copy значение.
