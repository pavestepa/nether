# Nether Memory Model — основополагающая модель управления памятью

**NMM 0.1 · редакция от 2026-10-25 · нормативное ядро проекта**

Этот документ фиксирует выбранную модель памяти Nether: ссылочную семантику
non-Copy значений, раздельные права владения, чтения и записи, автоматически
выводимую передачу ответственности за уничтожение, статические заимствования,
детерминированный cleanup и невладеющие `weak` связи.

Модель является **архитектурной константой проекта**. Frontend, анализатор,
библиотека, runtime, Rust backend и последующие оптимизации обязаны опираться
на её инварианты. Удобство реализации или ограничения backend не разрешают
незаметно менять семантику. Изменение нормативного ядра требует отдельного
явного решения владельца языка, новой редакции документа и обновления
приёмочных примеров.

Это единственная модель памяти Nether 0.1 (`nmm-0.1`), редакция 2026-10-25.
Нормативный контракт и завершённая реализация различаются: поддержанный subset
указан в [status](0.1/status.md). Неподдержанный случай отклоняется явно.
[Unsafe](unsafe.md) задаёт обязательства автора, не вторую модель памяти.
Математическое доказательство soundness не заявлено.

## Содержание

1. [Основание, приоритет и границы](#1-основание-приоритет-и-границы)
2. [Краткая формулировка](#2-краткая-формулировка)
3. [Термины и сущности](#3-термины-и-сущности)
4. [Обязательные инварианты](#4-обязательные-инварианты)
5. [Исходный синтаксис](#5-исходный-синтаксис)
6. [Copy, non-Copy и представление](#6-copy-non-copy-и-представление)
7. [Переменные и права доступа](#7-переменные-и-права-доступа)
8. [Заимствования и последовательный reborrow](#8-заимствования-и-последовательный-reborrow)
9. [Ownership и время жизни](#9-ownership-и-время-жизни)
10. [Присваивание и замена](#10-присваивание-и-замена)
11. [Функции, методы и результаты](#11-функции-методы-и-результаты)
12. [Поля, агрегаты и контейнеры](#12-поля-агрегаты-и-контейнеры)
13. [Графы и циклы](#13-графы-и-циклы)
14. [Weak и временный read guard](#14-weak-и-временный-read-guard)
15. [Создание и уничтожение](#15-создание-и-уничтожение)
16. [Ветвления, циклы и исключительные пути](#16-ветвления-циклы-и-исключительные-пути)
17. [Функциональные значения и замыкания](#17-функциональные-значения-и-замыкания)
18. [Изоляция, каналы и async](#18-изоляция-каналы-и-async)
19. [Unsafe, FFI и raw storage](#19-unsafe-ffi-и-raw-storage)
20. [Контракты функций и модулей](#20-контракты-функций-и-модулей)
21. [Статический анализ компилятора](#21-статический-анализ-компилятора)
22. [Typed MIR и проверка корректности](#22-typed-mir-и-проверка-корректности)
23. [Rust backend и оптимизации](#23-rust-backend-и-оптимизации)
24. [Диагностика](#24-диагностика)
25. [Приёмочные сценарии](#25-приёмочные-сценарии)
26. [Состояние реализации 0.1](#26-состояние-реализации-01)
27. [Порядок реализации и открытые детали](#27-порядок-реализации-и-открытые-детали)

## 1. Основание, приоритет и границы

### 1.1. Единственный контракт

NMM принята как единственная модель управления памятью новой версии 0.1.
Все compiler/runtime/backend layers и safe библиотеки обязаны сохранять O/R/W,
origins, lifetime и cleanup. Успешная компиляция не делает противоречащее
контракту поведение допустимым; такой случай является дефектом реализации.

### 1.2. Состав спецификации

Этот документ определяет память. [Основная спецификация](main.md) определяет
остальные правила, [профиль](0.1/profile.md) — поддержанный source subset,
[unsafe](unsafe.md) — границу обязательств. Эти документы не вводят
альтернативных memory rules. Планы и статус описывают работу, не меняя NMM.

Совместимые частные contracts: [storage](0.1/storage-contracts.md),
[layout](0.1/layout.md), [metadata](0.1/metadata.md),
[Rust backend](0.1/rust-backend.md). Они конкретизируют NMM и проверяются
по её инвариантам. Псевдокод, предлагаемые названия MIR operations и layout
metadata могут заменяться эквивалентной реализацией после приёмки.

### 1.3. Обязательная граница

Non-Copy binding не превращается в move по последнему use. O не выдаёт W.
Consuming API требует owner-token; borrowed ABI не подменяет consuming-effect.
Owning-рёбра ацикличны; regions/groups не являются обходом этого ограничения.
Unsafe допускает только явно определённые операции с дополнительными
обязательствами автора; обычные safe loans и ownership checks сохраняются.

## 2. Краткая формулировка

У каждого живого non-Copy объекта есть одна идентичность и ровно один
логический owner-token. Этот token определяет, кто обязан завершить lifetime
объекта и выполнить его cleanup. Ссылок на объект может быть несколько;
добавление ссылки не создаёт нового token и не клонирует объект.

Компилятор независимо отслеживает:

- **O — ownership:** право и обязанность передать либо завершить lifetime;
- **R — read:** право читать живую цель;
- **W — write:** эксклюзивное право читать и изменять живую цель.

`O ≠ W`. Владение readonly объектом позволяет уничтожить его в установленный
момент, но не позволяет изменить его через обычный readonly путь.

Для пересекающейся памяти допустимы несколько совместимых R или один активный
W. Передача W создаёт последовательное перезаимствование: доступ родителя
приостанавливается на срок использования дочернего пути. Передача O — отдельное
действие, выводимое из owning-контекста.

Последнее использование ссылки завершает её borrow. Уничтожение происходит
в установленной точке завершения **owned lifetime**, с сохранением наблюдаемого
порядка деструкторов. Компилятор не ищет последнюю ссылку во время выполнения.

Если безопасность обычных ссылок нельзя доказать статически, программа
отклоняется. Для её принятия нельзя незаметно добавить GC, strong ARC,
скрытое клонирование, runtime borrow checker или отложить destructor.

## 3. Термины и сущности

| Сущность | Смысл |
| --- | --- |
| `ObjectId` | Логическая identity объекта/ресурса; не равна машинному адресу |
| `PlaceId` | Место хранения: root, поле, элемент, payload; включает projection path |
| `OwnerTokenId` | Единственный логический token ответственности за конкретный lifetime |
| `CapabilityId` | Путь доступа с R/W, origin и ограничениями использования |
| `LoanId` | Заимствование с областью действия, родителем reborrow и множеством целей |
| `WeakId` | Стабильная невладеющая ссылка на identity, переживающая её payload |
| `GuardId` | Временный R guard успешного weak lookup |
| `RegionId` | Аналитическая область lifetime или физический allocation region; эти понятия различаются |
| `EntryId` | Статически выбранная специализация функции с фиксированным контрактом |
| `Origin` | Символическое происхождение: параметр, root, поле, внешний target, allocation |
| `StorageMode` | Статический режим компонента: owned или readonly descriptor |
| `Finalizing` | Lifetime уже завершается; повторный drop, resurrection и whole replacement запрещены |

ObjectId в статическом анализе может обозначать абстрактное множество runtime
экземпляров, например объекты, создаваемые одной инструкцией внутри цикла.
Это не разрешает считать разные итерации одним физическим объектом.

Owner-token — понятие семантики. В простом случае он полностью исчезает после
компиляции; не требуется отдельный heap object или счётчик для каждого token.
Drop flags и cleanup state нужны там, где путь исполнения влияет на initialization.

Class view указывает на identity. Inline view указывает на место живого значения.
Место descriptor и его target различаются: уничтожение descriptor не обязано
уничтожать target, а перенос descriptor не переносит внешний target.

## 4. Обязательные инварианты

1. **Единственность O.** У каждого завершённо созданного живого объекта ровно
   один owner-token. Во время переноса token не дублируется и не теряется.
2. **Exactly-once cleanup.** Каждый принятый во владение ресурс завершается
   ровно один раз на каждом завершающемся пути, кроме немедленного abort процесса.
3. **Отсутствие неявного clone.** Alias, reborrow, ownership transfer и channel
   transfer не клонируют non-Copy payload.
4. **Lifetime.** Каждый safe access происходит к живой и инициализированной цели.
5. **Эксклюзивность.** Пересекающиеся активные W/W и R/W несовместимы.
6. **Отсутствие повышения прав.** R non-Copy не превращается в W ни присваиванием,
   ни передачей O, ни generic specialization, ни промежуточной функцией.
7. **Независимость O и доступа.** Ни alias, ни W borrow сами по себе не передают O.
8. **Ацикличность ownership.** Owning-граф не содержит циклов; weak не является
   owning-ребром.
9. **Прозрачность effects.** Потребление, escape, invalidation и результат
   описаны в проверяемом интерфейсе вызова, включая indirect calls.
10. **Детерминированность cleanup.** Оптимизация не меняет наблюдаемый момент
    и порядок destructor, закрытия внешнего ресурса или weak invalidation.
11. **Identity.** Перенос class ownership сохраняет identity; Copy class создаёт
    другую identity; адрес не служит заменой identity.
12. **Изоляция.** Передача между контекстами не оставляет отправителю safe
    доступ к отправленному изменяемому графу.
13. **Weak safety.** Неуспешный lookup не читает мёртвый payload; успешный
    результат нельзя использовать после окончания guard/lifetime.
14. **Backend preservation.** Generated Rust обязан сохранять эти свойства,
    в том числе внутри собственного `unsafe`.

Здесь safe-программа предполагает корректность trusted runtime, intrinsics
и используемых unsafe/FFI адаптеров. Наличие произвольного ошибочного unsafe
кода не покрывается гарантией для обычных ссылок.

## 5. Исходный синтаксис

Примеры используют формы актуального профиля:

```nether
class User {
    age: i32

    constructor(initial_age: i32) {
        this.age = initial_age
    }

    fn read_age(this): i32 {
        return this.age
    }

    fn birthday(var this): () {
        this.age += 1
    }
}

fn inspect(user: User): i32 {
    return user.age
}

fn modify(var user: User): () {
    user.age += 1
}
```

Этот `User` — non-Copy: у пользовательского class отсутствует `#[Copy]`.
Если далее не приведено полное объявление, фрагмент использует эти `User`,
`inspect` и `modify` и помещается в тело функции.

| Назначение | Написание |
| --- | --- |
| Readonly binding | `let user = new User(20)` |
| Mutable binding | `var user = new User(20)` |
| Readonly parameter | `fn inspect(user: User): i32` |
| Mutable parameter | `fn modify(var user: User): ()` |
| Class construction | `new User(20)` |
| Inline struct | `Item { value: 1 }` |
| Fixed array | `{1, 2}`, тип `{i32; 2}` |
| Function type | `(User) => i32`, `(var User) => ()` |
| Weak class field | `weak parent: Node` |
| Optional lookup | `parent.get()` → `Option<Node>` |
| Exhaustive branch | `match ... { Option.Some(x) => ..., Option.None => ... }` |

`User()` без `new`, `Node?`, `fn f() -> T` и `let mut` не используются как
синтаксис Nether в этом документе. Вызов `modify(user)` не требует `var`
перед аргументом: требование объявляется у параметра.

Обычному non-Copy коду NMM не нужны `move`, `&`, `&mut`, явные lifetime
параметры, `own`, `deown` или `CloneOwn`. Raw address operations в `unsafe`
и прежние явные `ref` patterns — отдельные формы профиля. Совместимость
`ref` не разрешает изменять правила NMM или повышать права. Основные примеры
новой модели выражены без `ref`.

Блоки `text` ниже — псевдокод/контракты, не исходники. Примеры с `Vec`,
`Option`, каналами и weak обозначены как библиотечные или целевые: наличие
примеров не означает, что соответствующий API уже реализован.

## 6. Copy, non-Copy и представление

### 6.1. Copy

Для Copy value assignment создаёт независимое значение:

```nether
let original: i32 = 10
var independent = original
independent += 1
// original == 10; independent == 11
```

Это не повышение доступа к `original`. Изменяется другая копия. Аналогично,
обычный Copy параметр может быть скопирован в mutable local.

`#[Copy]` пользовательского типа проверяется по его компонентам и lifecycle
правилам. Пользовательский destructor несовместим с Copy. Copy class создаёт
отдельную identity с копиями полей; возможную allocation нельзя скрывать
за утверждением «Copy всегда бесплатен».

Копирование readonly descriptor копирует descriptor и его зависимости,
а не выдаёт W к target. `str` остаётся readonly descriptor UTF-8 данных.

### 6.2. Non-Copy

```nether
var a = new User(20)
let b = a
let c = b
// a, b, c обозначают один объект. Создан только один User.
let total = inspect(b) + inspect(c)
modify(a) // допустимо после последних uses b и c
```

Инициализация non-Copy alias не копирует payload. Non-Copy struct, enum,
tuple или array также подчиняется этим правилам, если весь тип не Copy.
Не следует выводить Copy только из формы литерала.

Ссылочная семантика не требует heap allocation для каждого значения.
Struct/tuple/array может храниться inline в storage владельца; alias становится
descriptor этого storage. Class имеет стабильную identity; базовое представление
использует отдельный payload allocation. Escape analysis может выбирать
размещение при сохранении lifetime, identity и наблюдаемого поведения.

Физическое перемещение inline payload допустимо, когда нет используемого view
на старое место либо доказано корректное обновление всех зависимостей. Оно
не является семантическим клонированием и не запускает второй destructor.

Явная библиотечная операция копирования может создать новый объект. Её имя
и контракт задаёт библиотека; NMM не вводит автоматический `clone`.

## 7. Переменные и права доступа

### 7.1. Таблица

| Инициализация | Ownership | Доступ |
| --- | --- | --- |
| `var a = new User(20)` | Новый O у root `a` | W |
| `let a = new User(20)` | Новый O у root `a` | R |
| `let b = a`, существующий non-Copy | Дополнительный O не возникает | R alias |
| `var b = a`, источник имеет W | Дополнительный O не возникает | W reborrow, родитель приостановлен |
| `var b = a`, источник имеет только R | Не допускается | Ошибка повышения прав |
| `let/var b = copy_value` | Независимая Copy value | R/W к копии, с сохранением прав внешних targets |
| `let/var b = owned_result()` | O по контракту результата | Доступ не сильнее разрешённого result provenance |

`let` не переприсваивается. `var` позволяет перепривязать binding и использовать
W к допустимой цели. Возможность изменить binding и возможность изменить
память target проверяются отдельно.

### 7.2. Запрет повышения readonly

```nether
let a = new User(20)
var b = a // ERROR: non-Copy R не предоставляет W
```

Ошибка сохраняется, даже если `a` больше нигде не используется и является
единственным владельцем.

```nether
fn forbidden(user: User): () {
    var editable = user // ERROR
    modify(user)        // ERROR
    user.age += 1       // ERROR
}
```

Consuming-effect обычного параметра не снимает эти запреты. `unsafe` блок
также не выключает обычную проверку safe bindings.

Ограничение относится к **пути доступа**, а не навсегда замораживает объект,
который ранее имел W. После завершения R borrow первоначальный W-путь
восстанавливается:

```nether
var a = new User(20)
let b = a
let age = inspect(b)
modify(a) // OK: R borrow b завершён; a исходно имел W
```

Но `var c = b` остаётся ошибкой: завершение borrow не повышает права `b`.

### 7.3. Отсутствие обхода через ownership

Передача token не создаёт W. Если non-Copy значение проходит через readonly
параметр, возвращается из него или сохраняется как readonly компонент,
его access provenance обязан сохраниться.

```nether
fn relay(user: User): User {
    return user
}

fn invalid(): () {
    let original = new User(20)
    var editable = relay(original) // ERROR: результат не выдаёт W
}
```

Для свежего результата права задаются его происхождением:

```nether
fn make_user(): User {
    return new User(20)
}

fn valid(): i32 {
    var user = make_user() // fresh owned result допускает W
    modify(user)
    return inspect(user)
}
```

Смысл запрета — исключить превращение **той же** readonly non-Copy цели
в mutable посредством переименования, обёртки, consuming entry или возврата.
Создание независимого Copy/явного clone результата имеет другое происхождение.

## 8. Заимствования и последовательный reborrow

### 8.1. Совместимость

| Активный доступ к пересекающейся памяти | Новый R | Новый независимый W | Invalidate/drop |
| --- | --- | --- | --- |
| Нет loan | Допустим | Допустим при W capability | Допустим при O/slot authority |
| R | Допустим | Запрещён | Запрещён |
| W | Только как дочерний R с приостановкой W | Только как дочерний W с приостановкой родителя | Запрещён |

Reborrow не является вторым независимым доступом. Он образует дерево
capabilities; предки временно не используются в конфликтующем режиме.
При активном дочернем R родитель не пишет. При активном дочернем W родитель
не читает и не пишет пересекающуюся цель.

```nether
var a = new User(20)
var b = a
b.age += 1
var c = b
modify(c)
// c больше не используется: завершается его W loan.
b.age += 1
// b больше не используется: завершается его W loan.
a.age += 1
```

Один O остаётся у `a`; итоговый возраст — 24. Ни копий User, ни дополнительных
владельцев, ни runtime locking эта последовательность не требует.

```nether
var a = new User(20)
var b = a
modify(a)    // ERROR: конфликт с последующим использованием b
modify(b)
```

### 8.2. Lifetime loan

Область loan — множество точек CFG, в которых capability нужна прямо или
транзитивно. Это может быть меньше лексического scope, но не меньше области
всех зависимых uses.

Uses включают чтение, запись, последующий вызов, сохранённый descriptor,
возвращённый alias, implicit destructor read, cleanup guard и async state.
Последняя текстовая строка с именем переменной не обязательно является её
последним семантическим use.

Loan не снимается только потому, что descriptor скопирован под другим именем.
Нужно завершение всех зависимых capabilities.

### 8.3. Раздельные поля

Доказанно непересекающиеся поля допускают независимые loans. Для неизвестных
индексов `items[i]` и `items[j]` возможность `i == j` учитывается консервативно.
Whole-object replacement конфликтует со всеми производными views.

Анализ выполняется после разрешения aliases в origins. Разные имена или
разные descriptor slots не доказывают, что их targets различны.

## 9. Ownership и время жизни

### 9.1. Кто является последним владельцем

«Последний вычисленный захватчик времени жизни» означает **конечный owning
контекст на конкретном пути исполнения**, а не последнюю прочитавшую ссылку.
На разных ветвях таким контекстом могут стать разные runtime места.
CFG и drop flags позволяют это выразить без runtime подсчёта обычных ссылок.

O автоматически передаётся, когда это требует проверенный owning-контекст:

1. Возврат локально принадлежащего объекта как owned result.
2. Инициализация owning поля или owning элемента контейнера.
3. Передача параметра в consuming entry.
4. Захват O в окружение замыкания (§17.2).
5. Успешная отправка transferable графа по каналу.
6. Извлечение owned значения из поля/контейнера в новое owning место.

Никакого пользовательского `move` для этого не требуется. В IR существует
явный `TransferOwner`.

### 9.2. Условия передачи

Для перехода O из `S` в `D` компилятор доказывает:

- token действительно принадлежит `S` и ещё жив;
- `D` имеет определённый owned lifetime и принимает этот token ровно один раз;
- источник после commit не выполняет действий, требующих прежнего ownership;
- сохраняющиеся views допустимы относительно destination lifetime;
- перемещение storage не инвалидирует используемые views;
- не появляется owning-cycle;
- права результата не сильнее входных access прав;
- normal, failure и unwind paths сохраняют cleanup ответственность.

Базовый безопасный алгоритм может отклонять transfer при живых aliases.
Более точный алгоритм вправе сохранить R alias, если докажет lifetime нового
владельца, отсутствие invalidation и прежние права. Это не разрешение
использовать отправленный через канал объект: межконтекстная передача строже.

### 9.3. Alias не становится владельцем сам по себе

```nether
var owner = new User(20)
let alias = owner
let age = inspect(alias)
// Завершение alias не уничтожает User.
// Непереданный O owner будет очищен на выходе из owning scope.
```

Если локальный объект должен выйти из функции, компилятор может передать
его O в результат; это требует доказательства, а не продления stack pointer:

```nether
fn build_user(): User {
    var local = new User(20)
    modify(local)
    return local // O из локального контекста передаётся в owned result
}
```

Возврат alias на внешний аргумент имеет borrowed контракт, если не выбран
доказанно consuming/owned entry. Возврат alias на локальную цель допустим
только при передаче её единственного O в результат и допустимом представлении.
Если token принадлежит другому независимому владельцу, такого решения нет.

Компилятор не переносит автоматически все связанные владельцы или весь scope
в heap, чтобы принять escaping view.

## 10. Присваивание и замена

### 10.1. Три разных операции

| Операция | Результат |
| --- | --- |
| Изменение поля через W | Меняется payload существующей цели |
| Перепривязка borrowed binding | Меняется descriptor; старая цель не уничтожается |
| Замена owned root/slot | Завершается старый owned lifetime и устанавливается новый |

```nether
var first = new User(20)
var second = new User(30)
var current = first
current.age += 1
current = second // rebind, first не уничтожается
current.age += 1
```

`current` остаётся borrowed binding. Он не становится owner в результате
присваивания и не позволяет присвоить себе свежий temporary без определённого
владельца с достаточным lifetime.

```nether
var owner = new User(20)
owner = new User(30) // owned replacement; прежний User завершается
```

### 10.2. Порядок replacement

1. Вычислить destination place, включая index, ровно один раз.
2. Вычислить RHS и завершить его создание в защищённом temporary.
3. Проверить/обеспечить предпосылки commit.
4. Начать cleanup прежнего owned значения; исключить его повторный drop.
5. После успешного cleanup установить новое значение и ownership.
6. Обновить initialization flags и позицию root в порядке cleanup.

Статические конфликты выявляются до запуска программы. Эти шаги задают runtime
порядок побочных эффектов, а не runtime borrow checks.

Если RHS паникует, старое значение сохраняет O и свою позицию cleanup. Если
старый destructor паникует, replacement не считается успешным: новый RHS
очищается своим guard, старый target повторно не уничтожается, cleanup его
оставшихся частей продолжается. Частично разрушенное место недоступно как
полностью initialized значение.

### 10.3. Class view и inline target

Class view позволяет менять поля/вызывать методы по W, но не заменять owning
reference класса:

```nether
var owner = new User(20)
var alias = owner
alias.age = 21        // OK
*alias = new User(0)  // ERROR: object view не даёт owning-slot authority
```

Для inline target safe `*view = value` остаётся операцией записи в целевое
место, если источник действительно является mutable view. В отличие от неё
`view = other` перепривязывает descriptor. Для raw pointer dereference
дополнительно требуется `unsafe`.

Replacement не выводится только из наличия W: нужны capability целевого
места, initialization, отсутствие live loans и допустимый lifecycle.

## 11. Функции, методы и результаты

### 11.1. Обычный параметр

`fn inspect(user: User): i32` для non-Copy предоставляет R. Без consuming
effects вызов заимствует объект, даже если это последний текстовый use caller:

```nether
var user = new User(20)
let age = inspect(user)
modify(user)
```

Обычный параметр может иметь выведенный consuming-effect: например, тело
передаёт O в owning storage или в owned result. Тогда caller обязан передать
token. Параметр получает O + R, **не W**.

Если тело только читает параметр, компилятор не должен произвольно выбрать
consuming entry ради раннего destructor.

### 11.2. Mutable параметр

`fn modify(var user: User): ()` для non-Copy предоставляет W borrow.
Ответственность за уничтожение остаётся у caller. Функция может выполнить
дочерний reborrow, но не потребить O самого параметра.

```nether
fn twice(var user: User): () {
    modify(user)
    modify(user)
}

fn example(): i32 {
    var user = new User(20)
    twice(user)
    return inspect(user) // 22
}
```

Извлечение owned поля/элемента через mutable storage — отдельный effect
в отношении **поля/элемента**, а не потребление owner-token параметра.
Оно обязано оставить storage в допустимом состоянии, например заменить
элемент или обновить длину.

Для Copy аргумента сохраняется value semantics: `var n: i32` изменяет
локальную копию. `var` не превращает все Copy параметры в ссылки на caller slots.

### 11.3. Receiver

`this` предоставляет R borrow, `var this` — W borrow. Instance receiver
не становится consuming только из-за своего написания. Вызов mutable метода
на readonly объекте не исправляется скрытым Copy receiver.

Constructor имеет отдельное право инициализации нового payload; это не обычный
mutable borrow уже готового объекта. Destructor имеет отдельную finalizing
capability (§15).

### 11.4. Возвращаемые значения

Контракт результата содержит как минимум:

```text
ownership: FreshOwned | TransferredOwned(origins) | Borrowed(origins)
access: Read | Write
component_modes: ...
component_origins: ...
```

Это metadata, а не новые типы в исходнике.

```nether
fn readonly_identity(user: User): User {
    return user
}

fn mutable_identity(var user: User): User {
    return user
}
```

У первого entry возможен borrowed R результат. Consuming/owned специализация
допустима только при необходимом owning-контексте, переданном O и сохранении R.
У второго результат — W reborrow аргумента; token остаётся снаружи.

```nether
var owner = new User(20)
var alias = mutable_identity(owner)
modify(alias)
let age = inspect(owner) // OK после завершения alias
```

Все пути одного выбранного entry должны иметь совместимые ownership,
capability и physical result ABI. Owned и borrowed варианты не смешиваются
неявным runtime tag. При разных borrowed origins lifetime ограничивается
всеми возможными целями; W разрешён только если каждый путь обеспечивает W.

### 11.5. Temporary и результаты без binding

Свежий temporary, переданный в borrowing call, сохраняет своего временного
владельца как минимум до окончания вызова и всех разрешённых uses результата.
Нельзя вернуть view этого temporary за его lifetime.

Неиспользованный owned result очищается в точке окончания соответствующего
выражения. Compiler не оставляет его «без владельца» и не перекладывает
cleanup на callee после успешной передачи O caller.

## 12. Поля, агрегаты и контейнеры

### 12.1. Static storage modes

Для non-Copy компонента compiler выводит один режим места:

- `Owned`: содержит единственный O компонента;
- `ReadonlyView`: содержит R descriptor внешней цели;
- `Weak`: только явно объявленное class field, с отдельным runtime контрактом.

Логический тип поля остаётся `T`. Режим, access provenance и зависимости
входят во внутреннее представление и metadata. У `struct` разные поля могут
иметь разные режимы; у одного array/buffer все элементы согласованы.

```nether
struct Holder {
    value: User
}

fn example(): i32 {
    var owner = new User(20)
    let view = owner
    let holder = Holder { value: view }
    return holder.value.age
}
```

`holder` владеет собственной оболочкой; `holder.value` хранит R descriptor.
Cleanup оболочки не уничтожает `owner`. Lifetime descriptor не может
превысить допустимый lifetime target.

Для owning literal fresh значение создаёт отдельный O компонента:

```nether
let holder = Holder { value: new User(20) }
```

Readonly доступ через holder не даёт W. Mutable оболочка также не повышает
права компонентов, изначально полученных через R. Для выдачи W к owned полю
нужны mutable путь оболочки и разрешающее access provenance самого поля.

### 12.2. Вывод режима

Constraints собираются из initializers, последующих присваиваний, вызовов,
результатов и внешних зависимостей до fixed point. Нельзя выбирать mode по
первому runtime insertion и менять его позже.

Явный R alias требует borrowed storage. Свежая owning temporary требует
владельца; compiler не создаёт скрытого owner ради превращения её в
долгоживущий R descriptor. Неограниченное storage получает owned default
после сбора constraints; противоречивые требования диагностируются.

Для прямого non-Copy root в owning позиции разрешён transfer, если доказаны
все условия §9. Будущий use источника не должен молча менять уже выбранный
контракт библиотечного метода. Borrowed specialization допустима лишь при
согласовании **всего** storage и его интерфейса.

### 12.3. Контейнерный контракт

Следующая таблица — целевой библиотечный контракт, не утверждение о готовности Vec.

| Операция | Owned elements | Readonly-view elements |
| --- | --- | --- |
| `push` | Принимает O элемента | Сохраняет R descriptor |
| `pop`/`remove` с результатом | Передаёт O результата | Возвращает R view внешней цели |
| `get` | Выдаёт R borrow элемента | Выдаёт R view внешней цели |
| `get_mut` | Выдаёт W при допустимом access provenance | Не выдаёт W к внешнему target |
| `replace` | Передаёт старый O в результат или очищает по API | Перепривязывает descriptor |
| Cleanup | Завершает оставшиеся owned элементы | Удаляет descriptors, не targets |

```nether
// Целевой пример: библиотека Vec и её effects должны быть реализованы.
var users = Vec<User>.create()
users.push(new User(20))
let removed = users.pop()
// removed владеет извлечённым User в Option.Some.
```

```nether
// Целевой пример readonly-view specialization.
var owner = new User(20)
let view = owner
var users = Vec<User>.create()
users.push(view)
let removed = users.pop()
// removed содержит view owner; O User остаётся снаружи.
```

Разные режимы не смешиваются в одном buffer. Нет per-element ownership tag.
Смешанный aggregate с owned и borrowed **полями** допустим: его shape известен
статически и отличается от динамического смешивания режимов элементов.

### 12.4. Invalidation и borrowed writes

Grow, remove, replacement и смена enum variant проверяются относительно views.
Факт, что allocation в конкретном запуске не переместилась, не отменяет effect.
Class identity может сохраняться при перемещении handle, но compiler должен
доказать lifetime независимость от старого slot; baseline может быть консервативным.

Переназначение borrowed поля mutable оболочки меняет descriptor, а не target.
May-write contract обязан описывать новые origins на normal и panic edges.
Нельзя записать в caller storage view локального объекта callee, который будет
уничтожен при возврате или panic.

Destructor оболочки может читать borrowed поля. Тогда эти targets должны быть
живы до соответствующего implicit use, а не только до последнего явного чтения.

## 13. Графы и циклы

Owning-рёбра образуют лес: у объекта один входящий owner-token, либо он является
root. Объект может владеть несколькими детьми. Нельзя создать второе owning
ребро к тому же объекту или замкнуть ownership на предка.

```text
root O → parent O → child
                    weak → parent
```

Обратная weak связь не участвует в подсчёте O. Shared descendants представляются
одним владельцем и проверенными R views/weak связями, а не несколькими владельцами.

Обычные borrowed рёбра могут образовывать цикл, если единый внешний lifetime
и cleanup dependencies статически доказаны. Это **не owning-cycle** и не
механизм самостоятельного владения каждого узла каждым соседом.

Регионы служат размещению и пакетному освобождению storage. Они не дают
бессмертия views, не заменяют logical ownership и не откладывают наблюдаемые
destructors. Старый замысел «поместить произвольный owning-cycle в группу»
не входит в NMM.

Если destructors требуют чтения друг друга в обе стороны, заданный порядок
cleanup не может удовлетворить оба требования. Такая программа отклоняется.
Разрыв через weak решает lifetime хранения ссылки, но destructor должен
обрабатывать `None`; weak не обещает доступ к уже завершаемому соседу.

Для сложного закрытия допускается явная операция `finish` до cleanup, снимающая
межобъектные зависимости. Compiler не добавляет tracing collector.

## 14. Weak и временный read guard

### 14.1. Source contract

Сохраняется актуальное написание: `weak` — модификатор поля непосредственно
в class; target — class identity. Это не `Node?` и не обычное `Option<Node>`
поле. Optional является **результат lookup**.

```nether
// Целевой weak-пример; требует реализации weak runtime и Option.
class Observer {
    weak target: User

    constructor(user: User) {
        this.target = user
    }

    fn age_or_missing(this): i32 {
        return match this.target.get() {
            Option.Some(user) => user.age,
            Option.None => -1,
        }
    }
}
```

Сохранение `user` в weak поле не потребляет O, не сохраняет длительный R loan
на payload и не мешает последующему уничтожению User.

В базовом профиле нет weak locals, weak параметров, weak struct/enum fields
или weak элементов контейнера. Явный weak capture замыканий требует
отдельного синтаксического расширения (§17.6); semantic правила будут теми же.

`get()` возвращает readonly результат. Mutable Observer не выдаёт W к User.
Weak не превращается в owner, не позволяет вызывать consuming API и не
является способом обойти conflict checker.

### 14.2. Состояния

```text
Unregistered/Alive → AliveWithWeakMetadata → Finalizing → Dead
                                                ↘ повторный get: None
```

Weak invalidation происходит **до пользовательского destructor**. Из destructor
нельзя воскресить target через weak. Повторное использование адреса не делает
старый weak handle живым.

Требуемая абстракция — control block/registry record, доступная без чтения
уже освобождённого payload:

```text
WeakControl {
    identity_or_generation
    domain
    state: Alive | Finalizing | Dead
    payload_address_if_alive
    weak_handle_lifetime_state
    active_read_guards
}
```

Это референсное устройство. Возможны отдельный control block или registry
с `(domain, slot, generation)`. Память metadata освобождается только когда
ни handle, ни guard не могут к ней обратиться; registry может безопасно
сохранять slots до завершения domain. Переполнение generation не допускает
ABA: исчерпавший поколения slot выводится из повторного использования.

Обычный объект, для которого weak не создаётся, не обязан иметь control block.
Счётчик weak handles/guards, если он используется, не является strong ARC:
он не назначает последнюю обычную ссылку владельцем payload.

### 14.3. Lookup и guard

```text
weak_get(handle):
    record = resolve_metadata(handle)        // не dereference старого payload
    if identity/domain/generation mismatch:
        return None
    if record.state != Alive:
        return None
    guard = acquire_read_guard(record)
    return Some(ReadView(record.payload, guard))
```

Получение guard и проверка Alive должны быть неделимы относительно допустимого
начала destruction. В одном изолированном синхронном контексте это может
обеспечиваться самим протоколом исполнения; межпоточное использование такого
record без отдельного протокола запрещено.

Guard живёт до последнего зависимого использования результата, включая
aliases и borrowed компоненты. Guard нельзя освободить при выходе из
`match` arm, если разрешённый результат вышел из arm и ещё используется.
При panic/cancellation guard завершается через обычный cleanup.

### 14.4. Почему guard не заменяет статический анализ

Runtime lookup проверяет, что цель жива **сейчас**. Compiler обязан доказать,
что во время использования результата не произойдёт:

- уничтожение или перемещение payload с invalidation;
- конфликтующая запись через иной W;
- transfer в другой изолированный контекст;
- escape результата за guard или за доказанный lifetime.

Если origins известны, loan относится к ним. Если неизвестны — к
консервативному множеству weak-addressable целей domain. Неизвестный
destructive call конфликтует с этим множеством.

Базовый вариант NMM: static checks запрещают drop/конфликтующий W при live
guard. Runtime guard state проверяет соблюдение weak протокола; он **не**
превращает недоказанную программу в безопасную за счёт ожидания, отложенного
destructor или runtime ошибки обычного доступа. Нарушение проверенного
протокола внутри trusted unsafe/runtime — invariant failure, а не поведение
корректного safe исходника.

```text
begin_destroy(record):
    require no live conflicting loans or guards
    record.state = Finalizing
    make future weak_get return None
    run destructor and owned-field cleanup
    release payload
    record.state = Dead
    reclaim metadata only when handles/guards cannot access it
```

При невозможности доказать отсутствие мутации/уничтожения между suspension
и возобновлением weak guard нельзя переносить через `await`.

### 14.5. Стоимость

Weak требует metadata, проверки identity/state и сопровождения guard.
Регистрация может потребовать allocation и иметь OOM путь. Для metadata
OOM нельзя оставить опубликованный handle на несуществующий record.
Промежуточные handles/guards должны корректно очищаться на failure.

Weak не удерживает мёртвый payload ради будущих lookup. Capacity registry
может сохраняться отдельно от payload allocations. Нельзя рекламировать
weak как бесплатный raw pointer или обещать стоимость обычного borrow.

## 15. Создание и уничтожение

### 15.1. Lifecycle

```text
Uninitialized → Constructing → Live → Finalizing → Dead
                         failure ↘ partial cleanup
```

До завершения constructor можно обращаться только к инициализированным частям.
Нельзя публиковать неполный `this`, получить на него обычный escaping view
или weak handle, доступный внешнему коду.

При неуспешном создании очищаются только успешно инициализированные owned
части; destructor целого незавершённого объекта не вызывается.

### 15.2. Точки cleanup

- Выход из owning scope для непереданных roots.
- Replacement/removal, завершающий старый owned lifetime.
- Завершение неиспользованного owned temporary/result.
- Cleanup окружения, сообщения или task state, владеющего token.
- Panic/unwind и cancellation, проходящие через соответствующий scope.

Последнее чтение R и окончание W loan сами по себе не являются этими точками.

### 15.3. Порядок

Сохраняется принятый наблюдаемый порядок:

1. Roots scope — обратно порядку успешной инициализации **текущих** owned
   значений. Успешная замена root обновляет его позицию.
2. У объекта — пользовательский destructor, затем owned поля в порядке
   объявления. Borrowed и weak поля не уничтожают targets.
3. У class hierarchy — cleanup производной части, затем base части.
4. Tuple/array — по индексам; enum — payload только активного variant.
5. Vec — по текущему порядку элементов, согласно библиотечному контракту.
6. При partial construction — только реально созданные части в применимом
   порядке cleanup, без destructor незавершённого целого.

```nether
// Resource destructor записывает id в непаникующий test trace.
var a = new Resource(1)
let b = new Resource(2)
a = new Resource(3)
// Полный trace: 1 при replacement, затем 3, 2 при выходе.
```

Ownership transfer снимает cleanup ответственность с прежнего места и
устанавливает её в destination. У поля/элемента позиция cleanup задаётся
порядком содержащего aggregate, а у нового root — моментом принятия O.
Обычное создание alias не меняет порядок owner roots.

### 15.4. Finalizing capability

Destructor получает специальный exclusive `this` после завершения конфликтующих
loans. Это lifecycle полномочие, поэтому destructor разрешён и для readonly
owned объекта. Оно не является общим преобразованием `let` в `var`.

Можно читать/менять собственные ещё живые поля и вызывать совместимые методы.
Нельзя:

- целиком заменить или повторно инициализировать finalizing target;
- передать его O в другое место;
- опубликовать ссылку, переживающую cleanup;
- получить новый weak доступ к нему;
- повышать права на внешние readonly borrowed targets.

Запрет whole replacement проверяется транзитивно через aliases и call effects.
Если callee может заменить finalizing target и отсутствие такого действия
не доказано, вызов отклоняется.

### 15.5. Panic и внешние ресурсы

При panic destructor cleanup оставшихся initialized частей продолжается.
При повторной panic во время unwinding — abort. Флаги снимаются так, чтобы
повторный заход в cleanup не вызвал destructor заново.

Destructor синхронен: без `await`, возвращаемого `Result` и убегающей работы.
Закрытие с ожиданием/обработкой ошибок выражается явным `finish`/`close`,
а destructor остаётся корректным fallback.

Region deallocation и оптимизация размещения не переносят закрытие файла,
lock release или иной наблюдаемый effect на произвольный более поздний момент.

## 16. Ветвления, циклы и исключительные пути

### 16.1. Join

Для каждой входящей CFG edge хранятся initialization, token location, loans
и origins. Если объект потреблён только на одной ветви, после join его нельзя
безусловно читать. Drop допускается только на путях, где token ещё принадлежит
этому месту.

```text
if condition:
    TransferOwner(source, destination)
else:
    keep source

join:
    source is not definitely available
    cleanup uses branch-consistent ownership state
```

Нельзя превратить «возможно два разных owner места» в два одновременно живых O.
Runtime branch state допустим; runtime reference counting для поиска owner — нет.

### 16.2. Циклы

Анализ достигает fixed point по back edges. Borrow, сохраняемый на следующую
итерацию, не завершается автоматически на последней строке тела. Повторная
инициализация moved места проверяется на каждом пути.

Cleanup state ограничен числом активных roots/slots, а не числом исторических
replacements. Бесконечно растущий журнал инициализаций не является допустимой
базовой реализацией.

### 16.3. Partial moves и patterns

Извлечение owned компонента отмечает только его место как переданное.
Чтение неполного aggregate целиком запрещено. У типа с пользовательским
destructor нельзя оставить отсутствующее поле, которое destructor считает
инициализированным; нужны доказанный специальный протокол или отказ.

Обычный binding non-Copy payload в pattern получает доступ по источнику,
а не неявную копию. Owning extraction требует отдельного owning контекста.
Guard pattern только читает; consume/W начинается после принятия guard.
Прежнее `ref var` не повышает R и не разрешает пересекающиеся bindings.

### 16.4. Reachability

Доказанно недостижимая ветвь может быть исключена из ownership/effect анализа
только по общему проверенному CFG, используемому также backend. Типы source
ветвей всё равно проверяются по правилам языка.

Неизвестное условие, неизвестный enum discriminant и потенциальная panic
не считаются недостижимыми ради прохождения проверки.

## 17. Функциональные значения и замыкания

### 17.1. Виды callable значений

Функциональные значения бывают двух видов:

1. **Static function** — именованная функция, unbound method или lambda без
   captures. Runtime значение — один code pointer; окружения нет.
2. **Capturing closure** — lambda, использующая локальные значения внешней
   области. Значение состоит из кода и **окружения** (environment), которое
   хранит captures.

```nether
fn read_age(user: User): i32 {
    return user.age
}

fn example(): i32 {
    let reader: (User) => i32 = read_age
    let user = new User(20)
    return reader(user)
}
```

Захватывающее замыкание использует существующую форму lambda
`(x) => expr` / `(var x) => { ... }`; отдельного синтаксиса captures нет.
Capture выводится из тела автоматически, ключевое слово `move` не вводится.

```nether
fn example(): i32 {
    var counter = new Counter(0)
    let base = 10
    let add = (x: i32) => x + base          // base: Copy capture
    let bump = () => { counter.increment() } // counter: W capture
    bump()
    bump()
    return add(counter.read())               // 12
}
```

Индирект-вызов (function pointer или closure) не стирает consumes, R/W,
origins результата и зависимости borrowed компонентов (§20.2). Выбранный
callable contract фиксируется при компиляции вызова; runtime tag режима
capture не используется.

### 17.2. Окружение и виды захвата

Окружение — обычное owned значение с aggregate-правилами §12: у каждой captured
позиции фиксированный static mode. Режим каждой captured переменной выводится
по её использованию в теле (до fixed point §21.7) как **минимально достаточный**:

| Использование в теле | Capture | Требование |
| --- | --- | --- |
| Только чтение non-Copy | R | Target жив до последнего использования окружения; запрещён конфликтующий W |
| Запись / вызов `var` метода / передача в `var` параметр | W | Unique borrow, источник приостановлен; источник должен давать W |
| Передача в consuming sink (§9.1), возврат как owned | O | Token передаётся окружению; источник должен иметь ownership authority |
| Copy значение | Copy | Независимая копия в момент создания замыкания |

Правила:

- Capture **не повышает права**: W capture из R источника — `NMM_READONLY_TO_WRITE`;
  O capture через R/borrowed путь без token — `NMM_NO_OWNER_TOKEN`.
- Capture O передаётся в момент **создания** замыкания (`TransferOwner`);
  источник считается переданным по §16.1. Capture не происходит «лениво»
  при вызове.
- Capture R и W создают loans, живущие до последнего использования
  замыкания и всех его копий/alias (§8.2), включая implicit cleanup read.
- Captures упорядочены по первому упоминанию в теле. Этот порядок задаёт
  порядок cleanup окружения (§15.3): owned captures завершаются в порядке
  захвата; R/W captures и Copy ничего не уничтожают.
- Capture поля `a.f` захватывает путь `a.f`, а не весь `a`, если disjointness
  с остальными uses доказана (§8.3).

### 17.3. Вид замыкания и эффекты вызова

Из использования окружения в теле выводится вид замыкания:

| Вид | Тело делает с окружением | Вызов требует | Повторные вызовы |
| --- | --- | --- | --- |
| `Read` | Только читает captures | R к замыканию | Сколько угодно |
| `Write` | Изменяет W captures | W к замыканию (`var` binding/параметр) | Последовательно |
| `Once` | Потребляет O capture | Передачу O замыкания | Один раз |

Вид входит в callable contract и effects (§20.1) и не стирается через
function type, generics, interface dispatch и переопределения. Вызов
`Write`-замыкания через readonly путь — `NMM_READONLY_TO_WRITE`. Вызов `Once`
потребляет само замыкание; последующий use — `NMM_USE_AFTER_TRANSFER`.

Эффекты тела замыкания (`Read/Write/Consume` над captured origins,
`MayPanic`, `MaySuspend`) становятся эффектами каждого call site. Тело
замыкания анализируется как отдельный entry, но его captured origins
символически связаны с окружением, а не с произвольными параметрами.

### 17.4. Замыкание как значение

- Замыкание — non-Copy значение с одним owner-token, подчиняется §6–§7:
  `let g = f` создаёт R alias, `var g = f` требует W на `f`.
- Замыкание, все captures которого Copy, не имеет деструктора и является Copy:
  копирование создаёт независимое окружение.
- Замыкание без captures эквивалентно static function и свободно преобразуется
  к function type `(A) => B`.
- Замыкание с captures передаётся в параметр функционального типа или generic
  параметр по тем же правилам, что обычный non-Copy аргумент: borrowed R/W
  entry или consuming entry, если callee сохраняет его в owning месте.
  Преобразование к типу `(A) => B` не стирает вид замыкания и captures из
  контракта; ABI представления (одиночный code pointer или пара code+env)
  выбирается статически и должен совпадать у всех путей (§11.4).
- Замыкание не может быть `weak` полем или weak элементом (§14.1).

### 17.5. Escape и время жизни

**Non-escaping** замыкание (вызывается в области и передаётся только в
borrowing параметры) может держать R/W captures на локальные цели.

**Escaping** замыкание (возвращается, сохраняется в поле/контейнер, передаётся
в consuming sink, `spawn`, канал) обязано удовлетворять одному из условий для
каждого capture:

- capture — O, либо Copy;
- capture — R/W на внешнюю цель с доказанным достаточным lifetime
  (например, на `var`/R параметр, если escape не выходит за его область).

Скрытого перевода captures в heap, клона или авто-O для «спасения» R/W
capture компилятор не выполняет — это `NMM_CLOSURE_ESCAPE`.

Запрещён owning-cycle: объект, который владеет замыканием (поле/элемент),
не может быть O capture этого замыкания (§13). Для разрыва используется
невладеющая связь: callback получает объект параметром, либо объект
хранится в замыкании как weak, когда weak captures будут приняты (§17.6).

### 17.6. Weak capture

Weak capture не передаёт O и не удерживает R loan; вызов замыкания
выполняет `get()` с guard по §14.3 и обязан обработать `None`. Weak
остаётся модификатором class field (§14.1); синтаксис явного weak capture
не утверждён. До его принятия сохранить связь без владения можно через
class с `weak` полем, которое замыкание захватывает как обычный capture.

### 17.7. Изоляция и async

- Замыкание, отправляемое по каналу или в `spawn`, проверяется на
  transferability (§18.2): все captures — O или Copy, без внешних
  R/W/weak связей. Иначе — `NMM_TRANSFER_DOMAIN`.
- Loan-capture, пересекающий `await`, требует доказанного lifetime
  (§18.3); окружение, нужное после suspension, принадлежит task state.
- Panic внутри тела замыкания проходит обычные cleanup/unwind правила §16;
  `Once` замыкание, прерванное panic, очищает ещё не потреблённые captures
  ровно один раз.

### 17.8. Cleanup и backend

Окружение очищается как обычный owned aggregate: в точке завершения
owning lifetime замыкания. Недопустимо откладывать destructor O captures
в closure/region ради оптимизации. Backend вправе инлайнить вызов и
раскладывать окружение в регистры/стек при доказанном lifetime и
идентичности; Rc/Arc/RefCell окружений и неявное клонирование запрещены (§23).

## 18. Изоляция, каналы и async

### 18.1. Контекст владения

Каждый изменяемый owned граф относится к одному изолированному контексту.
Многопоточность не превращает обычные references в shared mutable memory.
Для перехода используется ownership transfer через канал.

```text
sender owns message graph
    → successful channel commit
channel owns queued message graph
    → receive commit
receiver owns message graph
```

После успешного send отправитель не использует прежний object path или
его aliases. Закрытие канала уничтожает оставшиеся сообщения в установленном
каналом порядке; это ответственность владельца очереди.

Операция send до commit может не состояться. Контракт API обязан определить
судьбу O при закрытом канале, timeout, panic или cancellation: вернуть payload
либо очистить его, но не потерять/дублировать. Сам вид `send`/`recv` не задаётся
этой спецификацией вместо ещё не реализованной библиотеки.

### 18.2. Transferability

Проверяется весь достижимый owning граф и его borrowed/weak зависимости.
Нельзя передать внешний borrowed target как будто он является owned частью.
Нельзя оставить sender weak handle рабочим указателем в чужой контекст.

Базовый вариант отклоняет межконтекстную передачу с живыми внешними
borrowed/weak связями. Поддержка внутренних weak связей переносимого графа
требует отдельного доказанного протокола migration domain/handles. До этого
она диагностируется, а не реализуется копированием адресов.

Readonly capability не меняется на W на другой стороне канала. Канал передаёт
O и разрешённые capabilities, а не отменяет access provenance.

### 18.3. Async

Owned значения, нужные после suspension, принадлежат task state. Loan,
пересекающий `await`, требует доказанного lifetime и отсутствия конфликтов
во всём интервале приостановки. Сам факт одного OS thread этого не доказывает:
между poll могут выполняться другие задачи и callbacks.

Ссылки внутрь перемещаемого task state требуют стабильности адреса или
доказанного lowering без dangling references. Простое помещение raw pointer
в future не является решением.

Cancellation очищает initialized state и guards ровно один раз. Borrowed
task не переживает владельца; detached task получает необходимые O.

Модель асинхронности (stackless-задачи, вывод эффекта `MaySuspend`, отмена
как drop состояния) зафиксирована в [async-await-runtime.md](async-await-runtime.md);
этот раздел задаёт её memory-инварианты.

Выбор Tokio или другого runtime — backend/library решение. NMM не обещает
совпадение производительности с Rust и не делегирует ownership runtime.
До реализации source async эти требования проверяются на symbolic MIR.

## 19. Unsafe, FFI и raw storage

Полный Rust-like контракт — [unsafe.md](unsafe.md). `unsafe fn` объявляет
обязательства caller; явный `unsafe { ... }` подтверждает их выполнение.
Unsafe операции требуют такого блока даже в теле unsafe fn. Типы, обычные
ownership и safe loans продолжают проверяться. Safety сохраняется в public
contracts; её нельзя стереть преобразованием к safe callable. Raw operations
ниже — нормативный контракт до завершения source implementation.

`*T`/`*var T` — raw pointer, не owner-token. Copy pointer копирует адрес
и provenance, не объект. Получение mutable адреса требует допустимого W пути.
Разыменование, arithmetic, casts и foreign access должны соблюдать alignment,
extent, initialization, lifetime и aliasing.

Корректный unsafe контейнер не может экспортировать safe API, скрывающий
invalidation от caller. Его контракты описывают `Initialize`, `Take`,
`Replace`, `Drop`, `Invalidate`, `ReturnView` и `ReturnOwned`.

Сохраняется основа [storage-contracts.md](0.1/storage-contracts.md):

- `RawStorage<T>` содержит capability allocation и статический storage mode;
- allocation не означает, что все slots initialized;
- `take` передаёт token и оставляет slot uninitialized;
- `replace` передаёт старое значение по своему API;
- `move_slots`/`move_within` переносят representation без семантического clone;
- освобождение allocation допускается после завершения всех owned slots;
- ZST имеют логические slots/cleanup даже без физических bytes;
- borrowed descriptors сохраняют origins внешних targets.

Сигнатуры intrinsics разделяют storage mode до выбора consuming-effect:
owned insertion требует O, borrowed insertion сохраняет descriptor без Consume.
Non-Copy `var` параметр не потребляет самого себя. Mutable receiver может
потреблять принадлежащий ему initialized slot только через проверенный intrinsic
с явным Take/Invalidate/cleanup effect, не через promotion W в O.

FFI адаптер отдельно задаёт, кто выделяет/освобождает память, может ли callee
сохранять pointer, вызывать callback или инвалидировать объект. Неизвестный
foreign call не считается pure/readonly. Через обычную C ABI boundary
не пропускается Nether panic без специально определённого протокола.

## 20. Контракты функций и модулей

### 20.1. Минимальная metadata

```text
EntryContract {
    safety: Safe | Unsafe
    parameters: [
        logical_type,
        physical_storage_shape,
        access: R | W | CopyValue,
        ownership: Borrowed | Consuming,
        component_origins,
        access_provenance
    ]
    result: ownership + access + symbolic_origins + component_shapes
    effects: set<effect(symbolic_path)>
    weak_domains_and_guard_dependencies
    unwind_effects
    model_version
}
```

Для non-Copy `var` сочетание `W + Consuming` самого параметра недопустимо.
Внутренние initialization/finalizing capabilities выделяются отдельно.

Полезный набор effects:

| Effect | Значение |
| --- | --- |
| `Read(path)` | Чтение target |
| `Write(path)` | Изменение target |
| `BorrowR/W(path)` | Создание loan |
| `Consume(path)` | Передача O из места |
| `StoreOwned(dst, src)` | Owning edge |
| `StoreView(dst, origin)` | Сохранение descriptor и его lifetime |
| `Capture(R/W/O, path)` | Захват пути в окружение замыкания |
| `ReturnOwned/ReturnView` | Контракт результата |
| `Invalidate(path)` | Завершение/перемещение target storage |
| `WholeReplace(path)` | Замена целого target |
| `WeakCreate/WeakRead/WeakInvalidate` | Weak protocol |
| `MayPanic`, `MaySuspend`, `Send` | Управление исключительными/межконтекстными путями |

Readonly не означает pure: функция может создавать свои объекты, работать
с другими `var` параметрами и выполнять разрешённый I/O. Ограничение относится
к конкретным input origins.

### 20.2. Вывод и отдельная компиляция

Effects выводятся из тела и транзитивных вызовов. Для рекурсивной SCC анализ
повторяется до fixed point. Нельзя сначала принять рекурсивный вызов как
безэффектный, а затем сохранить выданные на этом основании W/owned результаты.

Публичный typed interface фиксирует effects, capabilities, storage shapes,
result origins и NMM version. Caller проверяется по этому интерфейсу без
необходимости видеть тело callee. Изменение consuming-effect или происхождения
результата — изменение контракта, требующее перепроверки зависимых модулей.

Function pointers, generics, imports/reexports, interface dispatch и overrides
не стирают контракт. Override не может неожиданно потреблять readonly borrowed
вход, сохранять его дольше или выдавать более слабый lifetime результата.
Вызов по набору возможных функций использует совместимый контракт/консервативное
объединение effects; несовместимые physical ABIs требуют отказа.

### 20.3. Выбор entry

Для non-Copy ordinary parameter допустим borrowed R entry или consuming R
entry, когда это требуется телом/owning результатом. Наличие будущего use
caller — ограничение корректности, а не разрешение произвольно менять API.

Если R borrowing достаточно и нет owning требования, выбирается borrowed
поведение. Если owning sink требует Consume, caller должен предоставить O.
Неоднозначность, которую нельзя разрешить без изменения наблюдаемого cleanup,
диагностируется; compiler не угадывает поведение по уровню оптимизации.

## 21. Статический анализ компилятора

Это референсный алгоритм. Любая реализация обязана сохранять инварианты §4.

### 21.1. Pipeline

```text
source
  → parser / AST
  → name and type resolution
  → typed HIR with origins and candidate storage modes
  → CFG MIR with explicit places and exceptional edges
  → joint inference of entries, effects, ownership and storage shapes
  → loans, escape, initialization and cycle checks
  → cleanup elaboration
  → verified MIR
  → Rust source
  → rustc
```

Отдельные этапы могут выполняться совместно до fixed point. Нельзя проверять
loans окончательно до стабилизации effects и result/component origins.

### 21.2. Alias/provenance analysis

Нормализовать каждый доступ в множество origins:

```text
root.field
parameter[k].field
storage[index or AnyIndex]
descriptor.target
class_identity
weak_domain_possible_targets
```

`MayAlias(p, q)` обязан учитывать projections и возможные targets, а не только
равенство local IDs. Разные class handles могут обозначать одну identity.
Dynamic indices расширяются консервативно. Сужение допустимо только при
доказанных фактах, сохраняемых через соответствующие writes/calls.

### 21.3. Состояние точки CFG

```text
State {
    initialization[Place]: Uninit | Init | MaybeInit | PartiallyInit
    token_location[Object]: symbolic owning place(s), path conditions
    capabilities[Path]: access, origin set, parent, suspended state
    loans: LoanId → {origins, access, parent, region}
    storage_shape[Place]
    component_dependencies[Place]
    weak_origins[WeakId]
    finalizing_targets
    cleanup_order_constraints
}
```

Путевая неопределённость в token_location не означает несколько O в одном
runtime состоянии. Если analysis не может исключить дублирование, это ошибка.

### 21.4. Основные transfer functions

```text
bind_existing_noncopy(destination, source, access):
    origins = resolve(source)
    require initialized and live(origins)
    if access == W:
        require source grants W
        create child W loan; suspend conflicting parent access
    else:
        create R loan
    destination = descriptor(origins)
    // no token duplication, no implicit clone

transfer_owner(source, destination):
    require unique live token reachable with ownership authority
    require destination accepts this ownership/access/storage contract
    require no invalidated live loans
    require no ownership cycle
    remove source cleanup responsibility
    install destination cleanup responsibility
    preserve access provenance

access(path, requested):
    require definitely initialized and live(path)
    require capability permits requested
    reject conflicting live loans except valid parent-child reborrow chain
```

Borrowed параметр, у которого нет ownership authority, не становится consuming
из-за знания адреса владельца. Межпроцедурная специализация должна явно выбрать
соответствующий entry и проверить caller.

### 21.5. Liveness и regions

1. Собрать прямые и неявные uses, включая destructor/guard.
2. Распространить dependencies через aliases, поля, результаты и calls.
3. Вычислить backward liveness на normal и unwind edges.
4. Удерживать parent capability приостановленной на время descendant loans.
5. Проверить каждую запись, invalidation и ownership transition.
6. Завершить loans на всех exits их области, включая exceptional exits.

Для доступов с множеством origins region должен быть корректен для каждой
возможной цели. Для неизвестного external effect требуется консервативный
контракт или отказ.

### 21.6. Ownership graph и escape

При добавлении owning edge `parent → child` проверить:

```text
child does not already have another owner
parent is not reachable from child through owning edges
borrowed components of child outlive their future uses
```

При динамическом изменении graph одного обхода синтаксических declarations
недостаточно: анализ обязан доказать отсутствие цикла для всех возможных
runtime targets. Без доказательства операция отвергается.

Escape проверяет return, stores, callbacks, task states, channels и FFI.
Перенос owning wrapper не продлевает lifetime его внешних borrowed targets.

### 21.7. Fixed point

```text
worklist = all affected entries
while worklist not empty:
    entry = pop(worklist)
    new_contract = analyze(entry, current_callee_contracts)
    if contract changed:
        invalidate provisional HIR/MIR depending on old shape
        enqueue callers and recursive SCC members

verify stable contracts
elaborate cleanup
verify final MIR
```

Domain анализа должен обеспечивать завершение: конечные projection summaries,
консервативное объединение dynamic paths, ограничение бесконечной generic
specialization. При widening теряется точность, а не безопасность.

## 22. Typed MIR и проверка корректности

Возможный набор операций, не пользовательский синтаксис:

```text
CreateObject / InitializePlace
BorrowRead / BorrowWrite / Reborrow
MakeClosure(captures) / CallClosure / DropEnvironment
EndLoan
TransferOwner
Read / Write / RebindDescriptor
StoreOwned / StoreView
WeakCreate / WeakLookup / EndReadGuard / WeakInvalidate
BeginFinalize / DropField / DropObject / ReleaseStorage
SendCommit / ReceiveCommit
```

Для базового примера:

```nether
var a = new User(20)
var b = a
modify(b)
let age = inspect(a)
```

Смысл checked MIR:

```text
a = CreateObject(User, 20), token O1, access W
b = Reborrow(a, W), loan L1; suspend a
Call modify(b) with child W loan L2
EndLoan L2
EndLoan L1; restore a
age = Call inspect(a) with R loan L3
EndLoan L3
scope exit: BeginFinalize O1; DropObject a; ReleaseStorage a
```

Verifier после всех преобразований проверяет:

- у каждого use есть корректное initialization/lifetime доказательство;
- token сохранён ровно в одном месте на каждом пути;
- borrowing не создаёт token и не повышает access;
- transfer исключает source из cleanup;
- drop соответствует owning месту и выполняется не более одного раза;
- guards завершаются на всех путях после последнего dependent use;
- finalizing target не заменяется и не воскресает;
- physical ABI и metadata совпадают у всех calls;
- cleanup order соответствует нормативному, включая replacement;
- aliases не переживают invalidation и исключительные переходы.

Verifier должен выполняться после проходов, меняющих ownership, CFG, loans,
storage или cleanup. Backend не получает «почти проверенный» MIR.

## 23. Rust backend и оптимизации

Актуальный backend генерирует Rust source. Упоминание «до LLVM IR» во
вставленном тексте означает требование завершить memory analysis **до
генерации исполняемого представления**, а не возврат удалённого LLVM backend.

Представление может использовать Box для class payload, inline storage,
raw descriptors, MaybeUninit/ManuallyDrop и явный cleanup state. Это допустимо
только при доказанных initialization, lifetime, alignment и aliasing.

Rust borrow checker не заменяет NMM checker. Успешный rustc не доказывает
корректность generated unsafe, а использование raw pointers не освобождает
backend от правил Rust.

`var` в source **сам по себе недостаточен** для `&mut`, `noalias`, `inbounds`
или сильных optimizer assumptions. Нужны доказательства границ доступа,
отсутствия конфликтующих aliases и допустимого provenance на весь интервал.
Нельзя создать долгоживущий `&mut` на весь aggregate ради доступа к одному
полю при живом view другого поля.

Разрешённые оптимизации при сохранении поведения:

- удаление чисто статических owner/loan bookkeeping;
- устранение лишних descriptor copies;
- размещение возвращаемого результата непосредственно у получателя;
- stack/region allocation при доказанном lifetime и identity;
- удаление пустого cleanup без наблюдаемых effects;
- специализация storage/entries и inlining;
- сужение origins и shortening loans по доказанным uses.

Запрещены скрытые clone, обязательный Rc/Arc/RefCell для обычных объектов,
неявное продление resource lifetime, незаметное подавление destructor и
runtime выбор режима owned/borrowed вместо фиксированного контракта.

Производительность уровня эквивалентного Rust — измеряемая цель. Приёмка
сравнивает одинаковые allocations, dispatch, checks, weak операции и cleanup,
включая Debug/Release, размер кода и горячие циклы. Статическая уникальность
даёт возможность оптимизации, но не гарантирует конкретный машинный код.

## 24. Диагностика

Таблица задаёт семантические категории NMM. Большинство пока отображается
в E-коды; `NMM_NO_OWNER_TOKEN` уже используется непосредственно. Unsafe
call вне блока — E0500; неподдержанный unsafe callable — E0501.

| Категория | Причина |
| --- | --- |
| `NMM_READONLY_TO_WRITE` | Попытка получить W из R non-Copy |
| `NMM_BORROW_CONFLICT` | Пересекающиеся R/W или W/W вне допустимого reborrow |
| `NMM_SUSPENDED_ACCESS` | Доступ через приостановленного предка |
| `NMM_NO_OWNER_TOKEN` | Borrowed путь передан в consuming позицию |
| `NMM_USE_AFTER_TRANSFER` | Use после передачи O/изоляционного send |
| `NMM_ESCAPE` | View/borrowed компонент переживает target |
| `NMM_OWNERSHIP_CYCLE` | Невозможно доказать ацикличность owning edge |
| `NMM_STORAGE_MODE` | Несовместимые static storage modes |
| `NMM_RESULT_CONTRACT` | Несовместимые owned/borrowed/access результаты |
| `NMM_INVALIDATION` | Drop/replacement/grow при live зависимом view |
| `NMM_PARTIAL_INIT` | Чтение неинициализированной/переданной части |
| `NMM_FINALIZING_REPLACE` | Whole replacement/resurrection завершаемого target |
| `NMM_WEAK_GUARD_ESCAPE` | Weak result переживает guard или доказанный lifetime |
| `NMM_CLOSURE_ESCAPE` | Escaping замыкание держит R/W capture без доказанного lifetime |
| `NMM_TRANSFER_DOMAIN` | В сообщении остаются запрещённые внешние зависимости |
| `NMM_EFFECT_MISMATCH` | Call/interface/metadata не соответствует effects |
| `NMM_UNSUPPORTED` | Семантика определена, но ещё не реализована |

Хорошая диагностика указывает конфликтующую операцию, origin/owner declaration,
место создания loan и последующее use, удерживающее его живым. Для consuming
call нужно показать effect callee. Для weak unknown provenance — объяснить
консервативное множество целей. Unsupported нельзя выдавать за принципиальный
запрет языка, а soundness error нельзя маскировать fallback codegen.

## 25. Приёмочные сценарии

Эти cases задают **новый oracle**. Это не журнал уже прошедших тестов.
Фрагменты с обычным синтаксисом пригодны для parser проверки; memory pass/fail
проверяется checker после его перехода на NMM.

### 25.1. Самодостаточная положительная программа

```nether
class Counter {
    value: i32
    constructor(initial: i32) { this.value = initial }
    fn read(this): i32 { return this.value }
    fn increment(var this): () { this.value += 1 }
}

fn advance(var counter: Counter): () {
    counter.increment()
}

fn main(): i32 {
    var owner = new Counter(10)
    let initial = owner.read()
    var first = owner
    first.increment()
    var second = first
    advance(second)
    first.increment()
    owner.increment()
    let result = owner
    return result.read() - initial
}
```

Ожидается `4`, один Counter, один его owner-token, отсутствие clone и
эквивалентный результат HIR/MIR/native. Все W loans последовательны.

### 25.2. Ошибки доступа

Каждая функция — отдельный отрицательный case с prelude User из §5.

```nether
fn readonly_owner(): () {
    let owner = new User(20)
    var alias = owner // NMM_READONLY_TO_WRITE
}

fn readonly_parameter(user: User): () {
    modify(user) // NMM_READONLY_TO_WRITE
}

fn live_read(): i32 {
    var owner = new User(20)
    let reader = owner
    modify(owner) // NMM_BORROW_CONFLICT: reader используется ниже
    return inspect(reader)
}

fn live_write(): () {
    var owner = new User(20)
    var writer = owner
    modify(owner) // NMM_SUSPENDED_ACCESS
    modify(writer)
}
```

Отсутствие последующих uses readonly `owner` не превращает первый case в pass.

### 25.3. Матрица обязательных проверок

| ID | Сценарий | Ожидание |
| --- | --- | --- |
| N01 | Несколько `let` aliases одного non-Copy | Один payload/O, несколько R, один cleanup |
| N02 | Последовательные `var` reborrows | Pass, восстановление родителей после окончания children |
| N03 | R → W через local/parameter/return/generic/owned wrapper | Compile-fail, включая consuming специализацию |
| N04 | Copy readonly → независимая mutable value | Pass, оригинал не меняется |
| N05 | Return локального owner | O у caller; callee не уничтожает результат |
| N06 | Return view внешнего входа | Borrowed provenance; target должен пережить uses |
| N07 | Escaping view без передаваемого O | Compile-fail; без скрытой allocation/clone |
| N08 | Consuming sink и дальнейший use старого owning пути | Compile-fail либо отдельно доказанный допустимый R alias; не второй O |
| N09 | `var` parameter пытается потребить сам вход | Compile-fail |
| N10 | Owned/read-view storage, pop, cleanup | Результат и destructor соответствуют выбранному mode |
| N11 | Смешанные modes элементов одного buffer | Compile-fail |
| N12 | Replacement root 1; root 2; replacement на 3 | Trace `[1, 3, 2]` |
| N13 | Rebind borrowed alias | Нет destructor старого target в точке rebind |
| N14 | Class whole replacement через view | Compile-fail |
| N15 | Disjoint fields / неизвестные dynamic indices | Pass при доказанной disjointness / консервативный conflict |
| N16 | Partial construction с panic | Cleanup только initialized частей; нет destructor целого |
| N17 | Partial transfer и cleanup aggregate | Каждая оставшаяся owned часть уничтожена один раз |
| N18 | Destructor читает borrowed target | Target жив до implicit use; обратный порядок даёт ошибку |
| N19 | Whole replacement finalizing target через helper | Compile-fail по transitive effects |
| N20 | Panic destructor | Остальные части очищаются; повторная panic → abort |
| N21 | Owning-cycle / обратное weak ребро | Fail / pass при остальных доказанных контрактах |
| N22 | Weak lookup до/после начала destruction | Some R guard / None |
| N23 | Повторное использование адреса/registry slot | Старый weak не оживает |
| N24 | Weak R guard и конфликтующий W/drop | Compile-fail даже при неизвестном точном target |
| N25 | Weak result alias и panic | Guard жив до последнего use; освобождён при unwind |
| N26 | Send commit и use отправителем | Compile-fail use; единственный O у очереди/получателя |
| N27 | Send failure/cancellation | Payload возвращён либо очищен по API, без утечки O |
| N28 | Borrow через await | Pass только при доказанном полном lifetime/conflict контракте |
| N29 | Function pointer/import/recursion | Те же effects/provenance, что при direct call |
| N30 | Loop/branch transfer и reinitialize | Корректные drop flags и ограниченный cleanup state |
| N31 | Grow/remove при живом element view | Compile-fail по invalidation effect |
| N32 | Readonly target через mutable оболочку | Не получает W |
| N33 | Weak metadata OOM/slot exhaustion | Нет dangling handle, ABA и повторного cleanup |
| N34 | Class Copy против ownership transfer | Разная identity при Copy; та же при transfer |
| N35 | Замыкание с Copy / R / W captures, non-escaping | Pass; W capture приостанавливает источник до последнего использования замыкания |
| N36 | W capture из readonly источника; вызов `Write`-замыкания через `let` | Compile-fail (`NMM_READONLY_TO_WRITE`) |
| N37 | Возврат замыкания с R/W capture локальной цели | Compile-fail (`NMM_CLOSURE_ESCAPE`); с O capture — pass |
| N38 | `Once` замыкание: повторный вызов; panic до потребления captures | Compile-fail на повторный вызов; captures очищены ровно один раз |
| N39 | Объект хранит замыкание, захватывающее этот объект по O | Compile-fail (`NMM_OWNERSHIP_CYCLE`) |
| N40 | Замыкание в канале/spawn с внешним R/W capture | Compile-fail (`NMM_TRANSFER_DOMAIN`); с O/Copy captures — pass |
| N41 | Замыкание через function type, generic и indirect call | Тот же вид, effects и origins, что при прямом вызове |
| N42 | Порядок cleanup O captures | Порядок первого упоминания; один destructor на capture |

### 25.4. Уровни тестирования

1. **Parser:** актуальные `new`, `: T`, receiver, fields, patterns; новые
   семантические примеры не должны случайно использовать чужой синтаксис.
2. **Compile-pass/fail:** сравнение с NMM oracle, проверка notes и origins.
3. **HIR/MIR:** результаты, identity, initialization и точные drop traces.
4. **Native Rust:** те же программы при O0/O3, panic/failure/cancellation paths.
5. **Runtime:** allocation/free balance, weak state, guards, generation reuse,
   partial construction и repeated replacement.
6. **Verifier:** намеренно некорректный MIR с дублированием O, забытым guard,
   неверным result ABI и неподтверждённым W должен отвергаться.
7. **Performance:** сравнение с эквивалентной Rust реализацией, включая weak
   overhead отдельно от обычных accesses.

Нельзя засчитывать parser success или старые тесты readonly-to-owned-mutable
как доказательство выполнения NMM.

## 26. Состояние реализации 0.1

Редакция 2026-10-25. Новая базовая линия заменяет прежнюю модель;
параллельного legacy memory mode нет. [Эксперимент](../proof-of-work/provenance-inference.md)
остаётся ограниченной проверкой алгоритма, не вторым источником семантики.

### 26.1. Синхронное ядро и unsafe boundary

Реализованы aliases, sequential reborrow, R → W запреты, borrowed ordinary
параметры и consuming fixed point. Все consuming entries требуют O;
alias отклоняется `NMM_NO_OWNER_TOKEN`. Для borrowed storage непотребляющее
тело сохраняет локальный alias параметра и экспортирует его provenance.

Добавлены unsafe fn и явные statement unsafe blocks; calling unsafe требует
лексического блока даже внутри unsafe fn. Safe NMM checks сохраняются.
Safety проверяется в interface/override signatures. Unsafe function pointers
пока отклоняются, чтобы не стирать safety в callable type.

Полный [статус](0.1/status.md) содержит реализованный subset и ограничения.
Raw pointers/storage/FFI, weak, closures, channels/async и отдельный MIR verifier
не завершены. Названия большинства диагностик остаются E-кодами;
семантическим NMM именам соответствует матрица приёмки, а не обещание всех кодов.

## 27. Порядок реализации и открытые детали

### 27.1. Приоритет новой 0.1

Основной исполняемый план — [mvp_plan.md](0.1/mvp_plan.md).
Сначала согласуются и проверяются синхронное ядро NMM, unsafe boundary,
MIR verifier и Rust backend/runtime. Расширения языка не заменяют эту приёмку.

После неё: raw storage и safe adapters; затем weak/ацикличные структуры,
closures и channel/async state. Для каждого расширения нужны compile-pass,
compile-fail, HIR/MIR/native O0/O3 и observable cleanup cases по §25.

### 27.2. Детали, которые нельзя объявлять уже утверждёнными

- Конкретный layout control block, registry, width generation и способ
  сопровождения weak handles/guards.
- Конкретные числовые коды diagnostics и wire format новой metadata.
- Синтаксис явного weak capture и имена/поверхность callable типов (как вид замыкания `Read/Write/Once` виден в сигнатурах и function type).
- Полнота source реализации closures и её приёмки; синтаксис unsupported
  остаётся явно диагностируемым до готовности lowering.
- Библиотечные сигнатуры каналов, send failure result и порядок cleanup очереди.
- Поддержка transfer графа с внутренними weak связями между domains.
- Полнота alias/escape solver и допустимая степень консерватизма.
- Формальное доказательство soundness, полный trusted computing base и
  performance результаты нового варианта.

До реализации расширений допускается явный unsupported diagnostic. Для
открытой детали выбирается консервативный вариант, сохраняющий нормативные
инварианты; нельзя молча расширить capabilities или добавить runtime ARC.

### 27.3. Правило для разработчика и агента

Перед изменением compiler/runtime нужно определить затрагиваемые O, R, W,
origins, lifetime и cleanup transitions; сверить их с этим документом и
выбрать приёмочные cases. Если реализация требует нарушения инварианта,
ошибка находится в плане реализации либо требуется отдельное решение
об изменении языка. Основополагающая модель не выводится заново из случайного
поведения текущего backend.
