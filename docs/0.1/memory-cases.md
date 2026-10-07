# Приёмочные контракты памяти этапа 0

Это normative positive/negative cases, **не уже пройденные compiler tests**.
Они задают oracle для этапов 2–8. Source checker, MIR interpreter и backend
должны проверяться отдельно. Общая prelude для фрагментов:

```nether
class Resource {
    id: i32
    constructor(value: i32) { this.id = value }
    fn read(this): i32 { return this.id }
    fn touch(var this): () { this.id += 1 }
    destructor { trace_drop(this.id) }
}
fn consume(value: Resource): () {
    var local = value
    local.touch()
}
fn identity(value: Resource): Resource { return value }
```

`trace_drop` — test harness sink, который записывает Copy i32 и не паникует.
Каждый fragment — отдельный function body, если не приведено declaration.
`panic_test()` — test panic origin. `finish_test(x)` только читает borrowed x;
поэтому его вызов не продлевает жизнь owner и не выполняет destruction.

## M01 — один владелец и consuming move (инвариант 1)

Pass:
```nether
let first = new Resource(1)
var second = first
second.touch()
```
first moved, second owned mutable, та же identity; cleanup `[2]` в конце scope,
один free. Fail:
```nether
let first = new Resource(1)
consume(first)
finish_test(first)
```
Consume имеет только owned entry; future use требует borrowed, которого нет.
Ошибка на вызове с указанием последующего use; программу не запускают.

## M02 — Copy независим от владения pointer (инвариант 2)

Pass:
```nether
#[Copy]
class Number {
    value: i32
    constructor(value: i32) { this.value = value }
    fn set(var this, value: i32): () { this.value = value }
}
fn copy_case(): i32 {
    let first = new Number(1)
    var second = first
    second.set(2)
    return first.value
}
```
Результат 1, две identities, каждый allocation освобождён один раз; destructor
отсутствует. Fail: добавить `#[Copy]` к Resource из prelude — запрещён
пользовательский destructor, не implicit deep clone/retain.

## M03 — view не переживает target (инвариант 3)

Pass:
```nether
var owner = new Resource(1)
let view = identity(owner)
finish_test(owner)
finish_test(view)
owner = new Resource(2)
```
identity borrowed readonly; view не owned. После последнего view use replacement
уничтожает 1; при выходе уничтожается 2. Fail: перенести replacement перед
`finish_test(view)`. Used loan блокирует уничтожение 1, даже если address позже
переиспользуется.

## M04 — unique mutable access (инвариант 4)

Pass:
```nether
var left = new Resource(1)
var right = new Resource(2)
left.touch()
right.touch()
```
Два независимых owner, cleanup `[3,2]`. Fail:
```nether
var owner = new Resource(1)
let view = owner
owner.touch()
finish_test(view)
```
Будущее owner.touch не позволяет move при alias creation; view readonly.
Его последующее использование конфликтует с mutable receiver owner.touch.

## M05 — readonly owned и readonly borrowed (инвариант 5)

Pass: M01. Fail:
```nether
var owner = new Resource(1)
let view = owner
finish_test(owner)
var promoted = view
promoted.touch()
```
View не получает ownership или writable capability; не подставляется clone.
Owning root сам по себе не разрешает мутацию через его readonly view.

## M06 — применимые entries (инвариант 6)

Pass:
```nether
let source = new Resource(1)
let borrowed = identity(source)
finish_test(source)
finish_test(borrowed)
var owned = identity(new Resource(2))
owned.touch()
```
Первый result borrowed(source), второй owned. Cleanup `[3,1]`. Fail: M01
consuming-only call с future source use. Function pointer, связанный с consuming
entry, также не может принять borrowed argument; это проверяется через metadata.

## M07 — receivers всегда borrowed (инвариант 7)

Pass: `owner.touch()` при var owner сохраняет caller ownership; owner cleanup
в caller scope. Fail: добавить Resource instance method
`fn steal(this): Resource { var local = this; return local }`.
Readonly non-Copy receiver нельзя превратить в owner. `var this` также не
получает consuming entry: возврат через него может быть только borrowed с
receiver lifetime, не independent owned результат.

## M08 — initialization, partial move и exactly-once drop (инвариант 8)

```nether
struct Pair { first: Resource; second: Resource }
fn partial(): () {
    let pair = Pair { first: new Resource(1), second: new Resource(2) }
    consume(pair.first)
}
```
Pass: Pair не имеет destructor, first moved в consume и drop `[2]` (id увеличен);
на выходе Pair drop только second `[2]`, два разных identities/free, не двойной
drop первой. Fail: прочитать pair целиком после partial move либо добавить Pair
destructor, которому требуется полностью initialized pair.
Дополнительный runtime case: panic при construction second очищает first,
но не вызывает destructor незавершённого Pair/Resource second.

## M09 — owner replacement против view rebinding (инвариант 9)

Pass owner replacement — M03. Pass borrowed rebinding:
```nether
var first = new Resource(1)
var second = new Resource(2)
var alias = first
alias.read()
finish_test(first)
alias = second
alias.touch()
finish_test(second)
```
Future uses сохраняют owners first/second; alias mutable borrowed. Rebinding
не уничтожает first; touch изменяет second до 3. Exit cleanup `[3,1]`.
Fail: передать `alias` в consume после rebinding — borrowed non-Copy не owner.
Ownership kind binding не меняется неявно с borrowed на owned при assignment.

## M10 — invalidation библиотечного storage (инвариант 10)

С библиотечным Vec owned specialization:
```nether
var values = Vec<Resource>.create()
values.push(new Resource(1))
let selected = values.get(0)
inspect_option(selected)
values.push(new Resource(2))
```
Pass: selected readonly view завершается до возможного grow. Cleanup Vec
по текущим индексам `[1,2]`. Fail: перенести inspect_option после push,
если grow может переместить storage. Аналогично remove/replace variant,
даже если конкретная allocation случайно не поменяла адрес.

Readonly-view specialization: push внешнего owner, используемого позже,
pop view, drop пустого Vec не уничтожает target. Использование pop result
после уничтожения внешнего owner — fail. Добавление owned и borrowed T
в один storage — mode conflict по утверждённому решению, не runtime tag.

## M11 — группы и weak не обходят checks (инвариант 11)

Group graph acceptance (после реализации group inference): root владеет
Resource identities A=1, B=2, принятыми в таком порядке; их внутренние поля
содержат только views друг на друга. Pass: destructors читают лишь собственные
id, exit root очищает B, A (`[2,1]`), не trace/refcount. Fail: destructor B
требует чтение A, destructor A требует чтение B — цикл требований cleanup.
Также fail: внешний used view на A после root destruction. Group не создаётся
для независимых удаляемых owners ради обхода такого отказа.

Weak acceptance: class Observer имеет weak поле target: Resource. Pass:
lookup A возвращает readonly Option.Some view; его использование заканчивается,
owner A уничтожается, следующий lookup возвращает None. Slot reuse с новой
generation не восстанавливает старую ссылку. Fail: destroy/mutate возможной
цели между lookup и последующим use результата. При unknown provenance
конфликт проверяется для всего weak-addressable domain. Mutable Observer не
даёт mutable Resource. Invalidation weak происходит до target destructor.

Group/weak cases описываются в symbolic MIR до source fixtures: конкретный
синтаксис library factories не должен подменять обязательный effect contract.

## M12 — unsafe сохраняет ordinary ownership (инвариант 12)

Pass:
```nether
var values: {i32; 2} = {1, 2}
unsafe {
    let pointer = &var values[0]
    *pointer = 3
}
```
Результат первого элемента 3, Copy inline owner, нет allocations/destructors.
Fail: заменить `var values` на `let values`. Unsafe не выдаёт writable capability.
Также fail: обычный M04 conflicting loan, помещённый внутрь unsafe block;
обычный checker не выключается. Stale/raw OOB pointer — нарушение unsafe
обязанности; диагностический interpreter обязан его обнаружить, это не
обещание полной static проверки произвольной pointer arithmetic.

## Дополнительные release acceptance traces

Panic: roots R1,R2, body panic → drops R2,R1; normal exit с panic R2 destructor
→ remaining fields R2, потом R1; повторная panic в любом cleanup → abort.
Construction failure → только успешно initialized части, никакого double free.
Частичная failure grow → guard нового буфера очищает перенесённые initialized
slots, старые moved slots не drop второй раз; commit не начинается до allocation.
Собственно move_slots не паникует при выполненных preconditions.

Long-running replacement/remove: после каждой операции live allocations равны
текущему содержимому плюс явные owners результата, а не числу исторических
insertions. Reserved allocator capacity учитывается отдельно. FFI callback
panic завершается в adapter по ABI, не пересекает C; прямой syscall тестируется
на Linux и не заменяется libc call.
