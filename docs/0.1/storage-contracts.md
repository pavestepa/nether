# Storage, provenance и эффекты 0.1

Статус: нормативный контракт этапа 0; реализации intrinsics ещё нет.
Основание — [утверждённые статические режимы](decisions/container-ownership.md).
Сигнатуры ниже доступны из compiler-owned модуля `core/intrinsic` по symbol ID,
а не по совпадению имени Vec/String/пользовательской функции.

## 1. Представление и режим

`RawStorage<T>` — compiler-known non-Copy allocation capability. Публичный
тип имеет только T; compiler отдельно выводит mode `Owned` / `ReadonlyView`,
representation и зависимости borrowed компонентов. В owned mode slot содержит
представление T; в borrowed mode — readonly descriptor живого T.
Descriptor borrowed class указывает на объект, borrowed inline T — на его place.
Ни один режим не меняется во время выполнения. Совместимые storage unification
constraints собираются до выбора default owned. Mutable-view storage в 0.1
не поддерживается; mutable target access доступен через owned specialization.

Capability содержит allocation base, slot capacity и allocator identity;
ни RawStorage, ни raw pointer не являются готовым Vec. RawStorage не хранит
length, не реализует insertion/removal и не распознаёт пользовательский
контейнер. Для safe API автор библиотеки задаёт length, операции и guards.

У allocation есть логический ID, generation, byte extent и alignment.
Для ZST/нулевой capacity физическая allocation не нужна: возвращается
ненулевой aligned sentinel. Логические slots, initialization и cleanup при
этом существуют. RawStorage — owning root allocation; его перемещение
передаёт allocation identity и не перемещает буфер.

## 2. Сигнатуры intrinsics

`M` ниже — запись выведенного режима в compiler metadata, не source generic.
`T@M` обозначает контракт результата/аргумента, не новый пользовательский тип.
Обычный исходник по-прежнему использует T. Каждый intrinsic кроме layout
queries требует `unsafe { ... }`; access/ownership проверки остаются включены.

| Source signature | Effects / обязательства |
| --- | --- |
| `size_of<T>(): usize` | Layout собственного значения T после specialization |
| `align_of<T>(): usize` | Ненулевой power-of-two alignment T |
| `slot_size<T>(storage: RawStorage<T>): usize` | Stride представления T@M; readonly borrowed storage receiver |
| `slot_align<T>(storage: RawStorage<T>): usize` | Alignment представления T@M; не всегда align_of<T> |
| `allocate<T>(capacity: usize): RawStorage<T>` | Fresh owned allocation, все slots uninitialized; mode из использования |
| `capacity<T>(storage: RawStorage<T>): usize` | Copy capacity, readonly borrow, не consuming |
| `slot_address<T>(var storage: RawStorage<T>, index: usize): *var u8` | Address raw slot representation, `index <= capacity`; не safe view |
| `initialize<T>(var storage: RawStorage<T>, index: usize, value: T): ()` | Index < capacity, slot uninitialized; Owned требует owned T, View сохраняет view |
| `take<T>(var storage: RawStorage<T>, index: usize): T` | Slot initialized; после операции uninitialized; результат T@M |
| `replace<T>(var storage: RawStorage<T>, index: usize, value: T): T` | Slot initialized; старое T@M передаётся caller, новое устанавливается; без implicit drop старого |
| `drop_slot<T>(var storage: RawStorage<T>, index: usize): ()` | Slot initialized → uninitialized; Owned cleanup, View удаление descriptor |
| `view<T>(storage: RawStorage<T>, index: usize): T` | Slot initialized; readonly view, не consuming результат |
| `view_mut<T>(var storage: RawStorage<T>, index: usize): T` | Только Owned; unique mutable view, не consuming результат |
| `move_slots<T>(var destination: RawStorage<T>, destination_start: usize, var source: RawStorage<T>, source_start: usize, count: usize): ()` | Перенос между **разными** allocations одного M, source initialized, destination uninitialized |
| `move_within<T>(var storage: RawStorage<T>, destination_start: usize, source_start: usize, count: usize): ()` | Overlap-safe перенос внутри одного storage; destination \ source uninitialized |
| `release<T>(storage: RawStorage<T>): ()` | Consuming-only allocation token; все slots uninitialized |
| `drop_in_place<T>(pointer: *var T): ()` | Полностью initialized owned T → uninitialized, полный generated cleanup |

Layout queries safe; slot queries (`slot_size`, `slot_align`, `capacity`) также
safe readonly operations. RawStorage поле нельзя создать из произвольного pointer.
Число bytes `capacity × slot_size` проверяется до allocation, также alignment
и target object limit. OOM запускает определённую panic; guard allocation
failure не получает несуществующий token. Panic payload/аварийная печать не
должны требовать успешной новой heap allocation.

`take`, `replace`, `initialize` не вызывают пользовательский код, не паникуют
после начала передачи при выполненных preconditions и не делают Copy. Copy,
если требуется source language assignment, выполняется **до** intrinsic.
Для Copy class это самостоятельная identity и отдельная потенциально failing
allocation; destination не меняется при её failure.

`drop_slot` инвалидирует initialization state до вызова destructor; если он
паникует, этот slot не уничтожается повторно. Cleanup оставшихся slots
организует библиотечный guard. Default cleanup RawStorage освобождает allocation,
но **не угадывает** initialized subset и не вызывает destructors slots.
Обязанность unsafe автора: до release/default cleanup все owned slots завершены
либо перенесены. Это тот же обязательный контракт и при unwinding. Диагностический
интерпретатор хранит initialization bitmap для проверки, baseline runtime не
должен добавлять bitmap ко всем буферам. Нарушение unsafe precondition — дефект
unsafe кода, не разрешённое поведение safe контейнера.

Raw `drop_in_place` не является способом снять compiler loan: safety зависит
от доказанного provenance либо unsafe обязанности автора; safe wrappers не
могут скрыть invalidation effect. Обычный owned local нельзя уничтожить через
pointer и затем оставить доступным как initialized местом.

## 3. Перенос диапазонов и ZST

Диапазоны — полуоткрытые logical slot indices. Start+count проверяется на
переполнение и на capacity **до** изменения state. Для разных allocations
post-state: source \ range, destination ∪ range. Для overlap внутри одного
allocation `I' = (I \ source_range) ∪ destination_range`.
Например, `[A,B,C,_,_]`, move 0..3 → 1..4 даёт `[_,A,B,C,_]`.
Это memmove физического representation и семантический Move, не Copy значений.
Нельзя сначала indiscriminately очистить всю старую область после memmove.

При совпадении диапазонов операция — no-op с сохранением initialization.
При count=0 допускаются one-past starts, dereference не происходит. Для ZST
переносятся только logical ownership/initialization; count не вычисляется
делением на stride. Cleanup ZST с destructor вызывается один раз на slot.

## 4. Safe views и инвалидирование

Сигнатура `view(storage,index)` всегда содержит реальный allocation capability.
Произвольная пара `(owner, raw_pointer)` не принимается вместо неё. Runtime
address вычисляется от capability и проверенного index. Unsafe автор доказывает
initialized slot; compiler сохраняет loan и provenance в return metadata.

В owned mode view lifetime ограничен живым slot allocation и borrowed
компонентами T; move/grow/remove/drop/replace, меняющие representation или
завершающие target lifetime, конфликтуют с последующим use view.
Class view (включая результат `get_mut`) ссылается на identity и не даёт права
заменить owning slot через `*view = new C()`. Замена требует явной операции
`replace` с проверкой invalidation ([решение](decisions/class-view-replacement.md)).
У class перенос ссылки между slots не перемещает identity, но baseline
консервативно сохраняет storage dependency до завершения view.

В readonly-view mode прочитанный descriptor зависит от **внешней цели**,
не от дальнейшего существования slot: remove/pop может удалить descriptor,
не уничтожая target. Зависимости внешних roots сохраняются через fields,
function results и metadata. `get_mut` на такой specialization недоступен;
заменить descriptor можно через replace, повысить права target — нельзя.

Два известных различных field/index projections могут быть disjoint.
Неизвестные dynamic indices конфликтуют консервативно; разные allocations
disjoint только при доказанном provenance. Разрешение unsafe не позволяет
объявить обычные safe loans несуществующими. Whole-storage invalidation
конфликтует со всеми производными loans, пока анализ не докажет более узкую цель.

Compiler-known effects включают `Read`, `Write`, `Initialize`, `Consume`,
`Invalidate`, `Free`, `ReturnView`, `ReturnOwned` с symbolic argument/field paths.
Wrapper выводит их из тела. Входящая unsafe функция не может экспортироваться
как safe без обычных проверок входных прав/lifetime; непроверяемые raw bounds,
initialization и resource protocol остаются обязанностью unsafe реализации.

## 5. Указатели

Raw pointer Copy копирует адрес и compiler provenance, не target ownership.
`&place`/`&var place` требуют live initialized place; mutable address требует
прав записи. Получение raw адреса не создаёт бессрочный safe view. `*p`,
pointer arithmetic/casts и foreign calls требуют unsafe.

Pointer +/- integer использует signed `isize` offset в элементах, checked
multiplication на stride и checked address arithmetic; результат внутри
того же allocation или one-past. Для ZST offset допускается только 0;
доступ к логическим элементам ZST выполняется storage intrinsics. Pointer
subtraction требует одинакового allocation, ненулевого stride и целого числа
элементов; результат должен помещаться в isize. Ordering разных allocations
не определён как допустимая операция; equality addresses допустимо, но не
доказывает общую identity (особенно ZST и reused allocation).

Pointer → usize/isize сохраняет связь exposed address с живым allocation в
семантической модели. Обратный cast разрешает доступ только к тому же живому
allocation, подходящим bounds/alignment/initialization и исходным правам;
integer arithmetic не продлевает lifetime и не повышает access. Нулевой cast
создаёт null, который нельзя разыменовать. Произвольное число без такого
происхождения не создаёт compiler-known owned resource или safe view.

Адреса от ОС/FFI допускаются для raw access при выполненном внешнем resource
protocol. Для превращения в owned storage нужен отдельный trusted adapter,
регистрирующий extent, alignment и release function; обычный cast не заменяет
такой adapter. В начальной библиотеке raw OS handles/buffers оборачиваются
unsafe кодом, а safe views возвращаются из RawStorage, без implicit adoption.

Backend baseline не навешивает noalias/inbounds/alignment assumptions из
одного наличия raw pointer. Сначала должны быть доказаны нужные premises;
unsafe source contract не разрешает вводить более сильные LLVM assumptions.
