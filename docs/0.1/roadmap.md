# Nether 0.1 — roadmap первой жизнеспособной версии

Статус: план реализации. Этапы ниже являются требованиями к результату, а не утверждением о готовности существующего компилятора.

Основа языка — [основная спецификация](../main.md), прежде всего §21–24, §34, §69, §81, §88–89, §91–95 и §99. Этот документ задаёт сокращённый объём версии 0.1 и порядок его реализации. Исключённые возможности остаются направлениями будущего развития; их присутствие в `main.md` не делает их обязательными для этого релиза.

## 1. Цель и основной приоритет

**Получить минимальную жизнеспособную версию Nether, на которой можно писать полноценные синхронные однопоточные программы, самостоятельно реализовывать контейнеры и начинать стандартную библиотеку.**

Критерий жизнеспособности — возможность написать на Nether обобщённые `Vec<T>`, `String`, связанные структуры данных и обёртки над системными ресурсами. Компилятор предоставляет примитивы работы с типами, памятью и платформой, а не встроенные реализации каждого высокоуровневого контейнера.

Главный приоритет — **практически подтвердить модель управления памятью до начала её оптимизации**. Нужны работающие owned/borrowed contracts, автоматический move, проверка views, уникальность мутации и детерминированное уничтожение. GC, ARC, скрытые clone и удержание всех объектов до конца программы не являются допустимыми упрощениями.

«Программы любой сложности» означает отсутствие искусственного ограничения на композицию функций, рекурсию, generic-типы и пользовательские структуры данных. Это не обещание принять любой граф ссылок или алгоритм, нарушающий правила владения. Некоторые структуры потребуют явно организованного хранилища, индексов, weak-связей или проверенного `unsafe` внутри библиотеки.

Идеология сохраняется: **бесплатная простота при максимальной скорости**. В 0.1 сначала проверяется корректность этой модели. Производительность оценивается, но не достигается ценой подмены семантики.

## 2. Обязательный объём 0.1

| Область | Требование |
| --- | --- |
| Объявления | `class`, `struct`, `enum`, `interface`, `fn`, `const` |
| Переменные | `let`, `var`; access mode отдельно от ownership |
| Управление выполнением | `if`, `else`, `while`, `match`, явный `return`; `break` и `continue` для циклов |
| Типы | Целые фиксированной разрядности, `usize`/`isize`, floats, `bool`, `char`, `()`, tuple, фиксированные массивы |
| Операции | Арифметика, сравнения, логические и битовые операции, сдвиги, присваивания, явные casts |
| Объектная модель | `new`, конструкторы, поля, static/readonly/mutable методы, детерминированные destructors |
| Память | Owned/view/Copy, автоматический move, уникальный mutable access, cleanup, логические группы владения, weak class fields |
| Generics | Типовые параметры функций/методов/типов, inference, interface/Copy constraints, мономорфизация |
| Массивы | `{T; N}`, доступ по индексу, проверка границ, ограниченные `const N: usize` параметры |
| Модули | `import`, `export`, видимость, контракты между модулями |
| Низкий уровень | Raw pointers `*T`, writable-вариант, адрес, разыменование, арифметика указателей в `unsafe` |
| Платформа | `extern "C"`, C-совместимый layout, связывание, прямые системные вызовы |
| Ошибки | Библиотечные `Option<T>`/`Result<T, E>`, `match`, panic и корректный cleanup при unwinding |
| Основа библиотеки | Allocation/deallocation, layout типов, typed initialization/move/drop, безопасные views на пользовательское хранилище |

`Option` и `Result` реализуются обычными enum. `Vec` и `String` не являются условием запуска первых этапов; их реализация — обязательная проверка достаточности ядра перед выпуском.

### Что не входит

- `async`, `await`, `spawn`, `task_scope`, scheduler и detached tasks.
- Потоки, workers, channels, межпоточный `transfer` и связанный синтаксис/API.
- Региональные оптимизации размещения: arenas, bump allocation, reset региона, объединение allocations, escape-based stack promotion.
- Variadic packs, `...T`, C varargs и variadic forwarding.
- Capturing closures, сложные type-level вычисления, associated types/GAT и пользовательские lifetime-параметры.
- Наследование class/struct, `dyn` и runtime vtables. В 0.1 interfaces обеспечивают статические generic constraints; композиция и enum заменяют эти дополнительные механизмы.
- `for` и общий iterator protocol, interpolation/форматирование, синтаксический сахар динамических массивов. Для первой библиотеки достаточно `while`, методов и явных вызовов.

Перечисленные возможности не должны молча приниматься с неполной семантикой. До реализации компилятор выдаёт понятную диагностику о неподдерживаемой возможности. Расширение scope требует отдельного решения, а не становится условием завершения текущего этапа.

## 3. Память: та же семантика, простое размещение

### 3.1. Базовая реализация

- Каждый обычный `new` получает самостоятельный allocation через простой allocator. Адрес class object стабилен до destruction.
- `struct`, tuple, enum и fixed array имеют inline layout; место хранения определяется содержащим значением. Inline не означает `Copy`.
- Move class передаёт ответственность за существующую identity, а не копирует payload. Move inline non-Copy переносит значение и инвалидирует источник.
- После cleanup allocation возвращается allocator. Для независимо удаляемого элемента это происходит при удалении/замене, а не при завершении приложения.
- Обычный view не увеличивает счётчик ссылок, не удерживает allocation в runtime и не проверяется при каждом доступе.
- Допустимы необходимые drop flags, сведения о частичной инициализации и metadata логических групп/weak. Это явные расходы базовой реализации, не GC и не ARC.
- Allocator может сохранять освобождённые страницы. Проверка утечек различает живые объекты, занятые allocations и зарезервированную allocator capacity.

Первый backend не выполняет собственные оптимизации размещения. Проверки корректности сначала проходят при отключённых оптимизациях; затем те же программы проверяются с оптимизациями backend для обнаружения неверных assumptions.

### 3.2. Логическая группа не равна физической арене

Отказ от оптимизаций по регионам **не отменяет** общие lifetime связанных объектов из §93 основной спецификации.

В 0.1 логическая группа может быть реализована как один owning root и список отдельных allocations с состоянием и порядком cleanup. Объекты по-прежнему выделяются и освобождаются по одному. Не нужны ни contiguous arena, ни bump pointer, ни `region.reset()`.

Внутренние views группы могут образовывать циклы. Они не становятся owning edges и не требуют поиска достижимости. Группа живёт по контракту root; потеря достижимости отдельного узла сама по себе его не освобождает. Для независимого удаления узел должен иметь самостоятельного владельца, а связи — подходящие lifetime либо weak.

Компилятор не объединяет независимо удаляемые элементы в группу до конца программы, чтобы обойти ошибки анализа. Нельзя также выбрать произвольный порядок destructors: взаимозависимые cleanup effects проверяются по §69. Неразрешимый цикл требований отвергается; явный `finish` может завершить ресурсные зависимости заранее.

### 3.3. Инварианты, которые нельзя отложить

1. У каждого owned ресурса ровно один логический владелец. После consuming move прежнее место не читается до повторной инициализации.
2. `Copy` создаёт независимое owned значение по правилам типа. Копирование pointer/descriptor не дублирует владение allocation.
3. View никогда не переживает цель. Lifetime учитывает поля, результаты функций, контейнеры, ветвления и повторные итерации.
4. Активный mutable access уникален для затронутой памяти. Несколько `var` сами по себе не конфликтуют; конфликтуют пересекающиеся доступы.
5. `let` запрещает запись через binding, но не замораживает owned значение навсегда. Допустимый move в `var` восстанавливает mutable ownership; readonly borrowed view так преобразовать нельзя.
6. Обычные параметры имеют применимые owned/borrowed entries. Consuming-only body не получает фиктивный borrowed entry.
7. `this` и `var this` всегда borrowed receivers. Получить owned экземпляр для consuming операции можно через обычный явный параметр static/free function.
8. Каждая успешно инициализированная owned часть уничтожается ровно один раз. Moved и неинициализированные части не уничтожаются.
9. Переприсваивание owned места завершает прежнее владение; перепривязка borrowed места не уничтожает его цель.
10. Рост/перемещение storage, удаление элемента и замена variant запрещены при сохраняющемся инвалидируемом view.
11. Общая группа и weak не являются способом обойти unique-access и lifetime checks.
12. `unsafe` не отключает обычный ownership checker. Непроверяемые операции имеют отдельные обязанности автора кода.

## 4. Минимальные низкоуровневые контракты

Этот раздел фиксирует необходимые решения для 0.1. Имена compiler intrinsics ниже обозначают требуемые операции; полный каталог сигнатур утверждается на этапе 0 до реализации зависимых библиотечных API.

### 4.1. Указатели и `unsafe`

Для профиля 0.1 принимается написание:

- `*T` — raw pointer для чтения;
- `*var T` — raw pointer для чтения и записи;
- `&place` / `&var place` — получение соответствующего raw address;
- `*pointer` — разыменование;
- `pointer + offset`, `pointer - offset` — смещение в элементах `T`;
- `left - right` — расстояние в элементах одного allocation.

Это явное уточнение surface syntax относительно `ConstPtr<T>`/`MutPtr<T>` из §99 `main.md`. Одновременно вводить две независимые системы указателей не требуется. Неявные safe views сохраняют прежнюю семантику и не записываются через `*T`.

Тип pointer допустим в поле, параметре и объявлении вне `unsafe`. Получение raw address, разыменование, pointer arithmetic, pointer casts и вызов небезопасного intrinsic разрешены только внутри `unsafe { ... }`. Само хранение или передача pointer не доказывает возможность его разыменовать.

```nether
fn first_byte(): u8 {
    var bytes: {u8; 4} = {10, 20, 30, 40}

    unsafe {
        var start: *var u8 = &var bytes[0]
        *(start + 1) = 99
        return *start
    }
}
```

Контракт операций требует живого подходящего allocation, правильного alignment, bounds, initialization и прав доступа. One-past pointer разрешён для адресной арифметики, но не для разыменования. Вычитание/упорядочивание указателей разных allocations не определяется как допустимая операция. Pointer arithmetic проверяется также на переполнение адреса и особый случай zero-sized типов.

`&var` требует mutable place. Cast не делает readonly view владельцем и не даёт права писать в readonly память. Pointer не продлевает lifetime; после освобождения или перемещения storage прежний pointer может стать недействителен.

Integer/pointer casts нужны для FFI и системных вызовов, но числовой адрес не является доказательством provenance или действительного объекта `T`. На этапе 0 фиксируются допустимые round trips и отдельные контракты для памяти, полученной от ОС. Backend не получает необоснованных `inbounds`, `noalias` или alignment assumptions.

### 4.2. Примитивы, без которых не получится библиотечный `Vec<T>`

Нужен небольшой доверенный слой compiler/runtime operations:

| Операция | Обязательный контракт |
| --- | --- |
| `size_of<T>`, `align_of<T>` | Layout после подстановки generic arguments; zero-sized и over-aligned типы |
| Allocation/deallocation | Size + alignment, проверяемая арифметика размера, определённая ошибка выделения, парность allocator/free |
| Typed initialization/write | Помещает owned `T` в неинициализированное storage; consuming effect виден checker |
| Typed move/read | Извлекает owned `T`, оставляя исходный slot неинициализированным; не скрытый Copy |
| Typed replacement | Передаёт старое значение либо уничтожает его по контракту, корректно устанавливает новое |
| `drop_in_place<T>` | Запускает compiler-generated cleanup полностью инициализированного `T` ровно один раз |
| Перенос диапазона | Учитывает overlap и число инициализированных элементов; перенос non-Copy не оставляет двух владельцев |
| Получение safe view из storage | Связывает результат с реальным владельцем/receiver, bounds и access mode, а не только с raw pointer |

Raw allocation не создаёт инициализированный `T`. Нулевые байты не являются допустимым значением произвольного типа. Raw pointer field сам по себе не вызывает destruction содержимого.

Для 0.1 достаточно allocation → move элементов → deallocation старого буфера; оптимизированный `realloc` не нужен. Обычный memcpy не подменяет семантический `Copy`, особенно для class с отдельной identity.

**Контракт библиотечного хранилища — обязательная часть модели.** Generic `push` должен статически требовать допустимый owned аргумент, `pop` возвращать owned значение, `get` возвращать view с зависимостью от receiver, а `get_mut` — уникальный mutable view. Lifetime вложенных borrowed компонентов `T` должен сохраняться при помещении в контейнер.

Checker не может восстановить эти факты из произвольных pointer casts. Нужны compiler-known intrinsics с проверяемыми параметрами ownership/provenance и экспортируемые эффекты wrapper. Точная связь задаётся в IR/metadata; пользовательские `owns` и lifetime-аннотации не добавляются. Авторы unsafe wrapper отвечают за соответствие raw storage объявленному контракту. Wrapper не должен иметь возможности незаметно объявить raw pointer источником бессрочного safe view.

Одинаковые примитивы должны позволять написать второй контейнер с другим именем и layout. Распознавание только имени `Vec` не считается достаточным решением.

### 4.3. FFI и прямые системные вызовы

Обязательны `extern "C"` declarations, вызовы foreign functions внутри `unsafe`, link symbols, C-compatible function pointers и `#[repr(C)]` POD structs. Экспортируемый C entry использует явно заданный C ABI; обычный модульный `export` сам по себе не меняет ABI функции.

Через FFI проходят только поддержанные ABI-safe scalars, raw pointers, opaque handles и POD aggregates. Class, enum языка, `Vec`, `String` и borrowed views не передаются как будто это C layouts. Строки/буферы передаются по явному pointer + length протоколу. C varargs исключены.

Panic не пересекает C boundary. Для первой версии допустим adapter с abort при panic; wrapper с явным error mapping может выполнять cleanup и вернуть согласованный C status. Foreign exception/longjmp через живые Nether frames запрещены контрактом.

**Прямые системные вызовы входят в обязательный release scope.** Вызов libc-функции `syscall` через FFI не закрывает это требование.

Первый эталонный target плана — `x86_64-unknown-linux-gnu`; запуск на другом host выполняется через Linux CI/VM. Это ограничение первого порта, а не языка. macOS и другие targets получают собственные platform adapters позднее. Если первый target меняется, меняется весь согласованный ABI/test profile до начала backend-этапа.

Backend предоставляет unsafe target intrinsic с номером вызова и фиксированными шестью machine-word аргументами, например контракт `syscall(number: usize, args: {usize; 6}): isize`. Это фиксированный API, не variadic pack. Неиспользуемые slots задаются явно. Lowering выполняет системную инструкцию target с корректными registers, clobbers и memory effects.

Номера вызовов, codes/errors и layout системных структур относятся к platform-модулю. Safe обёртки `read`, `write`, `close` и операции памяти проверяют buffers, lengths, partial results и ресурсные lifetime. Сам syscall не обещает автоматически вернуть `Result` или сохранить raw buffer после завершения его владельца.

## 5. Этапы реализации

Последовательность обязательна по критериям готовности. Ранние этапы дают инфраструктуру для проверки памяти; широкая стандартная библиотека и региональные оптимизации не начинаются до прохождения соответствующих проверок.

### Этап 0. Зафиксировать исполнимый профиль и проверяемые контракты

- [ ] Составить короткую grammar/type table всего включённого subset; исключённые формы получают диагностику.
- [ ] Сохранить `{T; N}` для fixed array, `{...}` для array literal в expression context и blocks в позициях control flow; явный `return`, опциональный `;` — по `main.md`.
- [ ] Закрепить таблицу operators, casts, overflow/division/shift rules. Не допускать host UB вместо ошибки Nether; отдельно описать `MIN / -1`, `MIN % -1`, float-to-int и invalid shift.
- [ ] Зафиксировать `*T`/`*var T`, pointer provenance, правила адреса и `unsafe`, точные сигнатуры storage intrinsics.
- [ ] Определить contracts safe views из unsafe storage, dynamic indexing и invalidation effects пользовательского контейнера.
- [ ] Зафиксировать layout/ABI начального target, entry point, link protocol и минимальный panic/runtime ABI.
- [ ] Описать формат ownership/lifetime metadata и правила вывода contracts для рекурсивных функций и взаимных вызовов.
- [ ] Подготовить positive/negative примеры для каждого memory invariant, включая логические группы и weak.

**Критерий готовности:** для каждого примера можно заранее указать owned/view режим, допустимые mutations, владельца результата и точный cleanup. Неясность в этих свойствах блокирует соответствующее lowering, а не закрывается runtime refcount.

### Этап 1. Frontend, простые значения и исполняемые программы

- [ ] Lexer/parser со spans; AST для обязательного синтаксиса.
- [ ] Resolver модулей, signatures, generic parameters и members; циклические ссылки declarations внутри пакета без runtime module initialization.
- [ ] HIR с определёнными типами и ссылками на declarations.
- [ ] Primitive arithmetic/comparison/bitwise operations, явные numeric casts, const evaluation с диагностикой циклов и overflow.
- [ ] `let`/`var`, scopes, `fn`, recursion, `if`/`else`, `while`, `break`/`continue`, `return`.
- [ ] Inline structs/tuples/fixed arrays и tagged enums; `match`, exhaustiveness, basic patterns.
- [ ] Базовый LLVM lowering, object emission, linking и запуск `main` без зависимости от готовых `String`/`Vec`.

**Критерий готовности:** исполняемые программы с вычислениями, ветвлениями, recursion и массивами; ошибки типов/инициализации не доходят до backend. Пока ownership для формы не реализован, форма отклоняется, а не исполняется небезопасно.

### Этап 2. MIR, владение и безопасные заимствования

- [ ] CFG-based MIR с places: local, field, variant payload, element и projection path.
- [ ] Раздельные признаки initialization, ownership, access mode, provenance и lifetime; type `T` не подменяет эти сведения.
- [ ] Явные IR operations для Copy, Move, Borrow, Reborrow, Initialize, Replace и Drop.
- [ ] Liveness/future-use analysis по всему CFG: branches, early exits, back edges, repeated iterations, reinitialization.
- [ ] Place-sensitive loans: непересекающиеся доказанные поля допускают независимые операции, неизвестные пересечения индексированных элементов проверяются консервативно.
- [ ] Автоматический выбор move/view/Copy и проверка перехода readonly owned → mutable owned.
- [ ] Вывод и проверка owned/borrowed entries, consuming-only effects, lifetime результатов, borrowed полей и static/instance receiver contracts.
- [ ] Fixed-point inference для recursive call groups без runtime выбора entry. Неоднозначный неподтверждённый контракт диагностируется.
- [ ] Class identity, construction, heap allocation; проверки полного и частичного initialization.
- [ ] Проверка `#[Copy]`, включая запрет ресурса с destructor и отдельную identity у Copy class.
- [ ] Diagnostics с местом создания loan, конфликтующей операцией и зависимым последующим use.

**Критерий готовности:** checker пропускает корректные программы и отвергает use-after-move, escaped view, readonly promotion, conflicting mutation, invalid return и использование неинициализированного payload. Владение не вычисляется динамически при запуске.

### Этап 3. Детерминированный cleanup и первая проверка модели памяти

- [ ] Drop elaboration после ownership analysis: cleanup blocks и необходимые drop flags на каждом выходе из CFG.
- [ ] Cleanup на normal exit, `return`, `break`/`continue`, замене значения и panic unwinding.
- [ ] Destructors и порядок §69: roots в обратном порядке initialization, пользовательский body до owned полей, array/tuple по индексам, enum по активному payload.
- [ ] Construction failure очищает только уже инициализированные части; moved части повторно не уничтожаются.
- [ ] При panic destructor продолжается предусмотренный cleanup оставшихся полей; повторная panic во время unwind ведёт к abort.
- [ ] Простой allocator и диагностический режим с allocation IDs, журналом создания/move/drop/free и проверками double-free/leak.
- [ ] Отдельная обработка OOM и bounds/arithmetic panic: определённый результат вместо случайного UB.
- [ ] Проверка MIR маленьких программ через интерпретатор/эталонную модель и сравнение наблюдаемых эффектов с backend.

**Критерий готовности — первая обязательная граница:** basic owned/view модель работает на исполняемых программах. Для каждого ресурса наблюдается один корректный cleanup, view не требует retain/release, замены не накапливают прежние allocations. До этого нельзя начинать широкую библиотеку или оптимизации памяти.

### Этап 4. Generics, interfaces и контракты библиотек

- [ ] Generic `class`, `struct`, `enum`, `interface`, функции и методы; явные type arguments и однозначный inference.
- [ ] Interface/Copy constraints, conjunction через `&`, проверка body по constraints, static dispatch.
- [ ] Ограниченные `const N: usize` для fixed arrays; проверка размера при specialization. Простые defaults — по §38.
- [ ] Monomorphization сохраняет ownership/lifetime/access modes и выбирает применимые entries, включая смешанные режимы нескольких параметров.
- [ ] Проверки рекурсивных типов: class indirection допустима, бесконечный inline layout отклоняется.
- [ ] Function pointers и их конкретные entry contracts; plain C function pointers не смешиваются с Nether ABI.
- [ ] Сериализация public signatures, ownership/lifetime/effect metadata и compiler-readable generic bodies.
- [ ] Импортированная библиотека проверяется по metadata; correctness не зависит от чтения исходного body обычной функции.

**Критерий готовности:** одна generic-реализация работает с Copy primitive, non-Copy struct, class с destructor и типом с borrowed компонентами. Аналогичные нарушения диагностируются внутри одного модуля и через импорт.

### Этап 5. Unsafe, FFI, syscalls и библиотечное storage

- [ ] Raw pointer types/address/deref/arithmetic/casts, проверка unsafe context и mutability исходного place.
- [ ] Layout intrinsics, allocation/deallocation и typed initialization/move/drop operations.
- [ ] Контракты safe view и ownership effects из §4.2 проходят через checker и metadata.
- [ ] Простой `RawBuffer<T>` с явным числом инициализированных элементов; рост через allocate/move/free.
- [ ] `extern "C"`, C ABI aggregates/functions/callbacks, link и panic boundaries.
- [ ] Прямой syscall lowering для начального target; минимальные read/write/close/memory wrappers.
- [ ] Проверки partial initialization, failing allocation, overlap, alignment, zero-sized `T` и переполнения размера.

**Критерий готовности:** программа пишет/читает данные через прямой syscall, вызывает C и отдаёт поддержанный callback; буфер non-Copy значений безопасно растёт и освобождается. Нарушение требования unsafe context даёт compile error, а корректность ручных storage operations проверяется инструментально.

### Этап 6. Связанные структуры, логические группы и weak

- [ ] Вывод owning groups и общего lifetime без arena allocation; отдельные allocations и явный cleanup list.
- [ ] Внутренние cycles из views не создают нескольких владельцев; внешние views ограничивают destruction root.
- [ ] Проверка destructor effects и недопустимых зависимостей cleanup order.
- [ ] Независимое владение удаляемыми элементами; borrowed links и weak не меняют момент удаления.
- [ ] `weak` только в class fields и только на class identity; readonly `get()` с `Option` результатом.
- [ ] Registry domain/slot/generation, invalidation до destructor, безопасное переиспользование slots, отсутствие ABA при исчерпании generation.
- [ ] Static provenance результата weak lookup: используемый view запрещает конфликтующее удаление/изменение возможной цели; при неизвестной цели применяется консервативный domain contract.
- [ ] Metadata групп/weak effects проходит через функции, generics и module boundaries.

**Критерий готовности:** граф с общим lifetime корректно освобождается; независимо удаляемые узлы не удерживаются до конца программы; stale weak никогда не становится ссылкой на новый объект. Для обычных объектов без weak registry entry не требуется.

### Этап 7. Минимальная стандартная библиотека как проверка достаточности

- [ ] `Option<T>`, `Result<T, E>`, базовые numeric/layout utilities.
- [ ] `Vec<T>`: create, length/capacity, push, pop, checked get/get_mut, replace, remove, clear, grow и destructor.
- [ ] `String` на owned UTF-8 storage; минимальные construction/append/length и readonly `str` view с lifetime владельца. Byte offsets не выдаются за Unicode character indices.
- [ ] Статические UTF-8 literals и минимальная поддержка `str` descriptor без зависимости parser от готового `String`.
- [ ] Проверенные wrappers allocator и файлового handle, явные close/finish и fallback destructor.
- [ ] Второй контейнер: например, slot storage или map с replace/remove. Реализация не использует специального распознавания её имени в compiler.
- [ ] Структура с рекурсивными связями, проверяющая class identity, группу либо independently owned storage с weak.
- [ ] Небольшое приложение из нескольких модулей: чтение файла, обработка через пользовательские коллекции, запись результата.

**Критерий готовности — полная практическая проверка:** библиотечные контейнеры безопасно работают с non-Copy ресурсами. Рост, удаление и cleanup не требуют GC/ARC, скрытого clone или удержания старых версий. Пользователь может написать собственный аналог на тех же доступных примитивах.

### Этап 8. Стабилизация и выпуск 0.1

- [ ] Все обязательные строки feature matrix имеют parser/typecheck/codegen/runtime tests либо явно неприменимый слой.
- [ ] Compile-pass, compile-fail, interpreter/backend differential и runtime resource tests выполняются автоматически.
- [ ] Инструментальные проверки backend-программ: AddressSanitizer/UndefinedBehaviorSanitizer, где поддержаны, и собственный allocation/drop журнал. Отсутствие sanitizer ошибки не заменяет static checks.
- [ ] Длительные replacement/remove/grow workloads не увеличивают число живых объектов пропорционально числу уже завершённых операций.
- [ ] Документированы supported target, link/runtime dependencies, ABI, casts, unsafe obligations и известные консервативные отказы checker.
- [ ] CLI компилирует и запускает примеры; diagnostics показывают исходные locations; build воспроизводима.
- [ ] Все memory acceptance tests пройдены при baseline lowering и повторно при включённых backend optimizations.

**Критерий выпуска:** 0.1 пригодна для начала реальной стандартной библиотеки и синхронных приложений, а её основная модель памяти подтверждена не только простыми примерами.

## 6. Обязательная матрица проверки памяти

Каждый сценарий имеет положительную программу и, где применимо, отрицательный пример с ожидаемой диагностикой. Runtime checks измеряют observable destructor effects, identity и живые allocations, а не только финальный числовой результат.

| Сценарий | Что должно подтверждаться |
| --- | --- |
| Возврат свежего class из функции | Объект жив у caller; move не копирует identity и не вызывает преждевременный drop |
| Последующее использование source | Выбирается допустимый borrowed entry либо consuming call отклоняется |
| Readonly owned → `var` | Move допустим без конфликтующих aliases; аналогичный readonly view отклоняется |
| Borrowed результат `identity`/метода | Lifetime и capability не теряются через несколько вызовов и imports |
| Две mutable переменные | Независимые объекты/доказанные disjoint places работают; конфликтующие aliases отклоняются |
| Ветвления и циклы | Move только на части путей и на повторной итерации не приводит к чтению инвалидного значения |
| Partial move и reinitialization | Оставшиеся части уничтожаются один раз; инварианты пользовательского destructor сохраняются |
| Частично созданный aggregate | При ошибке очищаются только успешно созданные части |
| Owned root против view binding | Замена root уничтожает старое значение; перепривязка view не уничтожает цель |
| Array/enum с non-Copy payload | Нет неявного Copy; drop следует initialization и активному variant |
| `Vec<T>` grow/remove/replace | Каждый элемент имеет одного владельца; старый буфер освобождён; drop count точен |
| View на элемент перед grow/remove | Последующее использование инвалидируемого view вызывает compile error |
| Контейнер с borrowed `T` | Контейнер не переживает внешнюю цель; владение storage не повышает права элемента |
| `str` view перед изменением `String` | Invalidation и lifetime проходят через библиотечный API |
| Copy class | Копия имеет самостоятельную identity; это не retain того же объекта |
| Общая группа с циклическими views | Cleanup без tracing/refcount; root lifetime определяет освобождение |
| Независимое удаление узла и weak | Drop происходит при удалении; lookup после удаления даёт None |
| Weak lookup и последующее удаление | Активный полученный view блокирует потенциально конфликтующее удаление |
| Panic и destructor effects | Cleanup order соблюдён; double panic имеет определённый abort |
| FFI и syscalls | Bounds/ABI/resource contracts соблюдены; unwind не пересекает C boundary |
| Длительная замена элементов | Число живых объектов ограничено текущим содержимым, а не историей замен |

Нужен небольшой набор автоматически генерируемых MIR/program cases с комбинациями branch, move, borrow, replace и cleanup. Эталонный интерпретатор отслеживает логические allocations, initialization и loans; backend проверяется по тем же ожидаемым эффектам.

Практическая проверка не является математическим доказательством soundness. Она должна сочетаться с описанными инвариантами, проверкой каждого lowering и аудитом небольшого trusted unsafe/runtime слоя. Консервативный compile error допустим; принятие unsafe поведения как safe Nether — дефект модели или реализации.

## 7. Что открывает путь к оптимизациям

Региональные оптимизации начинаются только после прохождения этапов 0–8. Базовая реализация с отдельными allocations сохраняется как эталон для сравнения.

Следующий roadmap может включать arenas/pools, объединение allocations, escape analysis, размещение в stack, сокращение metadata и объединение эквивалентных entry specializations. Каждая оптимизация обязана сохранять:

- допустимость/недопустимость исходной программы;
- identity и права доступа;
- момент и порядок наблюдаемых destructors;
- независимое удаление owned элементов;
- lifetime и invalidation views/weak;
- отсутствие скрытого clone, GC или ARC.

Для оптимизации нужны измерение конкретного overhead и прохождение всей memory suite в сравнении с baseline. Async и многопоточность получают отдельный план после стабилизации синхронной модели; они не являются условием выпуска 0.1.
