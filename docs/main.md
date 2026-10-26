# Nether Language Specification v0.1

**Редакция 2026-10-25.** Единственная модель памяти — [NMM](core-memory-managment.md)
(`nmm-0.1`); явная unsafe boundary — [unsafe.md](unsafe.md).
Реализованный subset указан в [профиле](0.1/profile.md) и [статусе](0.1/status.md).
Спецификация целевых возможностей не означает их готовность в компиляторе.

Nether компилируется в Rust source, затем обычным rustc в native code. Прямой LLVM backend не используется. Собственная модель памяти сохраняется; внутренний проверенный unsafe разрешён. Цель производительности — эквивалентный вручную написанный Rust при тех же storage/dispatch/cleanup contracts; проверка выполняется benchmarks. [Контракт backend](0.1/rust-backend.md).

## 0. Назначение

Этот документ фиксирует синтаксис и семантику Nether 0.1; memory rules определены NMM.

Он является исходной спецификацией для:

- разработчика компилятора;
- AI-агента;
- стандартной библиотеки;
- LSP;
- parser/type checker;
- MIR/backend;
- пользователей языка.

Если пример противоречит явно указанному правилу, правило имеет приоритет.

---

# 1. Основные свойства языка

Язык:

- статически типизирован;
- AOT-компилируем;
- не имеет GC;
- не имеет ARC;
- использует NMM с единственным owner-token и независимыми O/R/W;
- поддерживает самостоятельное владение owned элементами контейнеров и регионы связанных групп;
- выводит loans и lifetime; передаёт O только в owning-контексте;
- поддерживает `Copy` для создания самостоятельного owned значения;
- генерирует owned и borrowed варианты функций;
- использует `let` для readonly и `var` для mutable bindings/параметров;
- проверяет unique mutable access независимо от ownership;
- объявляет receiver методов явно: без `this`, `this` или `var this`;
- поддерживает `class`;
- поддерживает `struct`;
- поддерживает interfaces;
- поддерживает single class inheritance;
- использует Rust-подобные `enum` и `match`;
- определяет closures через NMM; их реализация пока не завершена;
- имеет ленивые async operations и structured `async/await`;
- привязывает async tasks к потоку без автоматической миграции;
- использует channel-only communication между потоками;
- использует bounded SPSC channels с backpressure и transactional transfer;
- выполняет deterministic синхронный cleanup ресурсов;
- использует статический polymorphism по умолчанию;
- использует `dyn` для runtime polymorphism;
- мономорфизирует generics при AOT;
- поддерживает отдельную компиляцию с ownership metadata и явную unsafe C ABI boundary.

---

# 2. Naming convention

Типы пользователя используют PascalCase:

```nether
User
PaymentService
HttpClient
Point
```

Функции и методы используют snake_case:

```nether
load_user()
calculate_total()
handle_request()
```

Переменные:

```nether
current_user
request_id
item_count
```

Примитивные встроенные типы являются исключением и имеют Rust-style lowercase naming:

```text
i8
i16
i32
i64
i128

u8
u16
u32
u64
u128

f32
f64

bool
char
str
usize
isize
()
```

`()` является unit type без отдельного имени (§85); `usize`/`isize` имеют размер указателя (§87).

---

# 3. Primitive numeric types

Signed integers:

```text
i8
i16
i32
i64
i128
```

Unsigned integers:

```text
u8
u16
u32
u64
u128
```

Floating point:

```text
f32
f64
```

Пример:

```nether
let age: i32 = 24
let distance: i64 = 100000
let ratio: f32 = 0.5
let pi: f64 = 3.1415926535
```

Compiler может поддерживать inference:

```nether
let count = 10
let pi = 3.14
```

Default types рекомендуется определить как:

```text
integer literal → i32
floating literal → f64
```

если контекст не требует другого типа.

---

# 4. `bool`

Boolean type:

```text
bool
```

Values:

```nether
true
false
```

Пример:

```nether
let enabled: bool = true
```

---

# 5. `char`

`char` представляет Unicode scalar value.

Синтаксис использует single quotes:

```nether
let letter: char = 'a'
let symbol: char = 'λ'
```

`char` не является строкой.

Его физическое представление определяется ABI, но рекомендуется использовать 32-bit Unicode scalar representation.

---

# 6. `u8`

Отдельный raw byte представлен типом:

```text
u8
```

Пример:

```nether
let byte: u8 = 65
```

Если нужен ASCII byte из символа, используется явное преобразование.

---

# 7. String literals

Double-quoted literal:

```nether
"some"
```

является строковым литералом.

Например:

```nether
let name = "Alex"
```

Compiler определяет, нужен ли здесь:

```text
str
```

или owned:

```text
String
```

в зависимости от контекста.

---

# 8. `str`

`str` представляет UTF-8 последовательность символов без самостоятельной изменяемой heap identity.

Концептуально это строковый view / immutable UTF-8 data.

Например:

```nether
let message: str = "hello"
```

`str` может быть представлен как:

```text
pointer + length
```

или compiler-known static data для literals.

Сам `str` не требует отдельного GC/ARC объекта.

---

# 9. `String`

`String` является отдельным ссылочным типом стандартной библиотеки с owned динамическим UTF-8 буфером. Концептуальный descriptor содержит pointer, byte length и capacity; порядок полей и размещение descriptor не фиксируют ABI. Такой подход к буферу аналогичен Rust `String`, но ссылочная семантика типа определяется правилами Nether.

`String` по умолчанию non-Copy: привязка от существующего места создаёт alias; transfer O требует owning-контекста NMM (§94). View не дублирует буфер и не создаёт нового владельца. Отдельное содержимое строки требует явной операции копирования; compiler не подставляет её для обхода конфликта.

Buffer representation: [Rust String documentation](https://doc.rust-lang.org/std/string/struct.String.html).

Пример:

```nether
let name: String = "Alex"
```

Compiler может автоматически создать `String`, если контекст требует owned string.

Явное создание также допустимо через API стандартной библиотеки:

```nether
let name = String.from("Alex")
```

Строковый literal:

```nether
"some"
```

не означает обязательную heap allocation.

Compiler может хранить literal непосредственно в read-only program data.

---

# 10. Tuple

Tuple использует Rust-style syntax:

```text
(i32, i64)
```

Пример:

```nether
let value: (i32, i64) = (10, 100)
```

Type inference:

```nether
let point = (10, 20)
```

Destructuring:

```nether
let (x, y) = point
```

Tuple является inline compound value, как struct и enum. Владение данными внутри tuple определяется его компонентами. Присваивание tuple использует общие правила move/view/Copy (§94); inline layout сам по себе не означает копирование.

---

# 11. Inline arrays

Fixed-size inline array использует синтаксис:

```text
{i32; 4}
```

Пример:

```nether
let values: {i32; 4} = {2, 4, 5, 1}
```

Memory layout:

```text
[i32][i32][i32][i32]
```

Все элементы хранятся inline.

Размер массива является частью типа.

Поэтому:

```text
{i32; 4}
```

и:

```text
{i32; 8}
```

являются разными типами.

---

# 12. Inline array literals

```nether
let numbers = {2, 4, 5, 1}
```

Compiler может вывести:

```text
{i32; 4}
```

Доступ:

```nether
numbers[0]
```

Fixed array не выполняет динамическую heap allocation.

---

# 13. Dynamic arrays — `Vec<T>`

Динамическая последовательность:

```text
Vec<T>
```

Пример:

```nether
let numbers: Vec<i32> = [2]
```

или:

```nether
let numbers = [2, 4, 5, 1]
```

Compiler выводит:

```text
Vec<i32>
```

Синтаксическое различие намеренное:

```nether
{1, 2, 3}   // inline fixed array
[1, 2, 3]   // Vec<i32>
```

---

# 14. `Vec<T>`

`Vec<T>` является отдельным ссылочным типом стандартной библиотеки для динамического contiguous container. Концептуальный descriptor содержит pointer, length и capacity. Length измеряется в элементах, capacity — вместимость буфера в элементах. Размещение descriptor и обязательная heap allocation для него не задаются этим описанием.

Буфер организован аналогично Rust `Vec`, а владение и aliases следуют Nether (§94). `Vec<T>` не является `Copy`, даже если `T` является `Copy`: копия descriptor с тем же owned буфером создала бы двух владельцев. Move передаёт владение без deep copy; view обращается к существующему контейнеру.

Buffer representation: [Rust Vec documentation](https://doc.rust-lang.org/std/vec/struct.Vec.html).

Пример:

```nether
var values = [1, 2, 3]

values.push(4)
```

Тип:

```text
Vec<i32>
```

Для struct элементы располагаются inline:

```text
[Point][Point][Point]
```

Для class хранятся object references:

```text
[ref][ref][ref]
```

Представление через references не означает совместное владение. Owned элемент имеет самостоятельный логический owning context внутри контейнера; borrowed элемент остаётся view на внешнего владельца. Удаление или замена owned элемента завершает его lifetime, если владение не передаётся получателю (§93).

Тип `Vec<T>` не требует пользовательского `owns`/`view` параметра. Compiler выводит соответствующий контракт контейнера и проверяет его операции. Удаление borrowed элемента убирает только view и не уничтожает внешнюю цель.

Уточнение для профиля 0.1 (утверждено 2026-10-06): один storage имеет статически выведенный режим owned либо readonly-view элементов; смешивание режимов запрещено. Пустой storage получает режим из ограничений всех использований, при отсутствии ограничений — owned. Owned inline элементы хранятся inline, borrowed inline элементы — через view descriptors. `pop` передаёт owned значение только в owned specialization; в borrowed specialization он возвращает view с lifetime внешней цели. Mutable-доступ к buffer не повышает права на borrowed цели. Подробности — [решение профиля 0.1](0.1/decisions/container-ownership.md).

`let values` предоставляет readonly-доступ к container и его элементам. Изменение состава, capacity или owned element storage требует `var values` и mutable receiver соответствующей операции. Это не повышает права ранее сохранённых readonly views: capability каждого элемента сохраняется в контракте. Ссылочный layout `Vec` не отменяет правила §23–24.

---

# 15. Empty Vec

Если тип невозможно вывести:

```nether
let values = []
```

это compile error.

Следует написать:

```nether
let values: Vec<i32> = []
```

---

# 16. `struct`

`struct` является inline value type.

```nether
struct Point {
    x: f64
    y: f64
}
```

Создание:

```nether
let point = Point {
    x: 10.0,
    y: 20.0,
}
```

Физически значение может находиться:

- на stack;
- внутри другого struct;
- inline внутри class;
- внутри `{T; N}`;
- внутри `Vec<T>`;
- в registers.

Struct не имеет обязательной отдельной heap allocation. По умолчанию он не получает `Copy` только из-за inline layout; пользователь может объявить проверяемый атрибут `#[Copy]` (§94).

---

# 17. Struct inheritance

Struct не может наследовать другой struct или class.

Запрещено:

```nether
struct Point3D : Point {
}
```

Struct может реализовывать interfaces:

```nether
struct Point : Printable {
    ...
}
```

---

# 18. `class`

`class` является reference/identity type.

```nether
class User {
    name: String
    age: i32
}
```

Class создаётся только через `new`:

```nether
let user = new User("Alex", 24)
```

`new` означает:

```text
создание новой identity
+
создание ownership root
+
логическая принадлежность memory context, определённому ownership inference
```

---

# 19. `new`

Для class:

```nether
let user = new User()
```

обязательно.

Запрещено:

```nether
let user = User()
```

Для struct наоборот `new` запрещён.

```nether
let point = Point {
    x: 1,
    y: 2,
}
```

---

# 20. `new` не означает `malloc`

`new` является семантическим указанием на создание новой class identity.

Compiler может реализовать:

```nether
let user = new User()
```

через:

```text
stack allocation
region allocation
arena allocation
scalar replacement
registers
```

Heap semantics языка не требуют отдельного системного allocator call для каждого объекта.

---

# 21. Ownership и передача O

O передаётся только из источника с ownership authority в owning-контексте.
`let b = a` для существующего non-Copy места создаёт R alias независимо от
будущего использования `a`. Полные правила — [NMM §9](core-memory-managment.md#9-ownership-и-время-жизни).

---

# 22. Aliases и lifetime

Aliases сохраняют origin и права цели. Последний use завершает borrow,
а не lifetime owner. Return/storage и call contracts сохраняют provenance.
См. NMM §8–§12.

---

# 23. `let` / `var`: O, R и W

`let` предоставляет R, `var` требует W; O не выдаёт W.
Non-Copy readonly root/alias/parameter не повышается в mutable даже при
последнем use или consuming entry. Copy может создать независимую mutable копию.
См. NMM §6–§8.

---

# 24. Привязки и замена

Fresh owned value создаёт owner; привязка от existing non-Copy места — alias.
`var b = a` требует W и создаёт sequential reborrow. Присваивание alias
перепривязывает его; `*inline_view = value` меняет цель по W. Class view
обозначает identity и не заменяет owning slot. См. NMM §7–§10.

---

# 25. Functions

Объявление обычной функции:

```nether
fn add(a: i32, b: i32): i32 {
    return a + b
}
```

Обычная block-bodied функция возвращает полезное значение только через явный `return`, как в TypeScript. Последнее выражение тела не является неявным результатом. `return` без значения и достижение конца тела допустимы для результата `()`; для другого return type все продолжающие выполнение пути должны явно возвращать совместимое значение.

```nether
fn invalid_add(a: i32, b: i32): i32 {
    a + b  // compile error: выражение не заменяет return
}
```

Expression-bodied non-capturing lambda `(x) => x * 2` сохраняет краткую форму (§29); lambda с block body требует явного `return` по тем же правилам, что fn. Добавление или удаление необязательного `;` не меняет результат функции.

---

# 26. Function types

Тип функции записывается в TypeScript-style форме:

```text
(A) => R
```

Например:

```text
(i32) => bool
```

означает функцию:

```text
i32 → bool
```

Несколько аргументов:

```text
(i32, i32) => i32
```

Без аргументов:

```text
() => i32
```

Без результата:

```text
(String) => ()
```

Mutable capability является частью типа параметра:

```text
(var User) => ()
(var User) => User
```

Функцию с параметром `var user: User` нельзя присвоить переменной типа `(User) => ()`, скрыв mutable parameter mode. Обычный `user: User` предоставляет R даже при consuming-effect; перенос O не разрешает mutable local из readonly пути. Поэтому `(User) => ()` не обещает отсутствие consuming effects или наличие borrowed entry. Выбранный entry, ownership/result и lifetime-контракты сохраняются вместе с callable metadata и проверяются при обычном `operation(user)`, без обязательных пользовательских generic annotations. Для unbound method pointer receiver всегда borrowed (§80).

Async qualification также является частью callable-контракта:

```text
async (i64) => User
```

Такой вызов создаёт ленивую async operation, а `User` является типом результата `await` (§57).

---

# 27. Function values

Язык поддерживает ссылки на функции.

```nether
fn double(value: i32): i32 {
    return value * 2
}

let operation: (i32) => i32 = double
```

Вызов:

```nether
let result = operation(10)
```

---

# 28. Function values и явное состояние

Обычный function value `(A) => R` содержит только указатель на функцию и не содержит captured environment.

Он может представлять free function, static function или non-capturing lambda. Указатель на функцию не требует обязательной heap allocation.

Stateful callable object остаётся отдельным механизмом (§32): его состояние передаётся через явный receiver и interface/generic constraint. Автоматическое превращение такого объекта или bound method в простой function pointer с неявным сохранением состояния не допускается.

Method reference с receiver рассматривается в §80.

---

# 29. Функциональные значения

Named function values поддерживаются. Closure/capture contracts определены
[NMM §17](core-memory-managment.md#17-функциональные-значения-и-замыкания);
source closures пока не реализованы и диагностируются как unsupported.

---

# 30. Захваты и замыкания

Captures подчиняются единственным O/R/W правилам NMM §17: R/W borrow либо
O capture из owner. Они не вводят refcount или скрытое клонирование.
До source implementation синтаксис отклоняется; это ограничение реализации.

---

# 31. Передача функций

```nether
fn map_value(
    value: i32,
    mapper: (i32) => i32
): i32 {
    return mapper(value)
}
```

Использование:

```nether
let result = map_value(
    10,
    (value) => value * 2
)
```

Поскольку function expression ничего не захватывает, hidden allocation отсутствует.

---

# 32. Stateful callable

Если callback требует состояние, используется обычный тип.

```nether
interface Mapper<T, R> {
    fn invoke(this, value: T): R
}
```

```nether
struct AddValue : Mapper<i32, i32> {
    amount: i32

    fn invoke(this, value: i32): i32 {
        return value + amount
    }
}
```

Использование:

```nether
let mapper = AddValue {
    amount: 10,
}

items.map(mapper)
```

---

# 33. Class syntax

```nether
class User {
    name: String
    age: i32

    constructor(initial_name: String, initial_age: i32) {
        name = initial_name
        age = initial_age
    }

    fn get_name(this): str {
        return this.name
    }

    fn set_age(var this, new_age: i32): () {
        this.age = new_age
    }
}
```

Instance методы class и struct явно объявляют `this` или `var this` первым параметром (§34). Собственные поля доступны через `this.field` либо по короткому имени с теми же правами. Отсутствие receiver в `fn` означает static method.

Локальные имена и параметры затеняют короткие имена полей; `this.name` явно обращается к полю. `name = name` при параметре `name` не считается инициализацией одноимённого поля и не разрешает переприсваивание readonly параметра.

Все объявления и члены по умолчанию public. `public` можно писать явно; закрытый доступ объявляется через `private`.

```nether
class Token {
    private value: String

    constructor(initial_value: String) {
        value = initial_value
    }
}
```

Конструктор должен инициализировать все обязательные поля до использования полностью созданного объекта. Он имеет специальный implicit mutable `this` для инициализации; писать receiver в `constructor(...)` не требуется. Это не делает результат `new` автоматически mutable: окончательные права задаёт destination `let`/`var`. Производный class вызывает базовый конструктор через `super(...)` (или `super()` без аргументов).

---

# 34. Методы и явный receiver `this`

В class, struct и interface форма первого параметра определяет receiver:

| Объявление | Вид | Вызов |
| --- | --- | --- |
| `fn method(): R` | Static, без экземпляра | `Type.method()` |
| `fn method(this): R` | Readonly borrowed receiver | `value.method()` |
| `fn method(var this): R` | Unique mutable borrowed receiver | `value.method()` при mutable-доступе |

```nether
class Counter {
    value: i32

    constructor(initial: i32) {
        this.value = initial
    }

    fn create(): Counter {
        return new Counter(0)
    }

    fn get(this): i32 {
        return this.value
    }

    fn increment(var this): () {
        this.value = this.value + 1
    }
}

let fixed = Counter.create()
fixed.get()
fixed.increment()       // compile error: readonly receiver

var changing = Counter.create()
changing.increment()
changing.get()
```

`this` или `var this` допускается только первым параметром метода и не требует type annotation: тип задаётся содержащим class/struct/interface. Внешний вызов не передаёт `this` отдельным аргументом. Receiver binding нельзя переприсвоить через `this = ...`; `var this` разрешает изменение данных экземпляра в пределах доказанного доступа. Остальные параметры используют обычные формы `name: T` / `var name: T`.

В instance method допустимы `this.field` и короткое имя поля, а также вызовы собственных методов. Короткая форма эквивалентна доступу через текущий `this` и сохраняет его права. Readonly `this` не может вызвать метод с `var this`, получить mutable alias своего non-Copy поля или передать ту же identity с повышенными правами в `var` параметр. Независимое Copy значение проверяется отдельно по §94. Mutable receiver может вызывать readonly методы с временным понижением доступа.

Instance call не подставляет автоматический Copy или consuming move receiver ради доступа к mutable методу. Через существующий readonly binding нельзя вызвать `var this` метод даже при owned или Copy значении. Для non-Copy mutable binding требует исходного W или fresh результата с допустимым access provenance; readonly root не повышается. Для Copy можно явно создать независимую mutable копию. Метод на mutable Copy receiver изменяет этот receiver, а не неявную временную копию. Свежий receiver expression может получить требуемое методом право с обычной проверкой lifetime/aliases.

Метод без receiver является static независимо от обращения в его теле к именам. В нём нет `this` и неявного доступа к instance fields/methods. Static вызов записывается через имя типа, не через экземпляр. При передаче явного `var` аргумента static метод может изменять этот аргумент.

Те же правила действуют для inline struct. Например, изменение поля struct в контейнере требует mutable-доступа к соответствующему месту хранения и проверки aliases. Наличие mutable метода не делает все экземпляры типа mutable.

```nether
struct Point {
    x: i32
    y: i32

    fn length_squared(this): i32 {
        return this.x * this.x + this.y * this.y
    }

    fn translate(var this, dx: i32, dy: i32): () {
        this.x = this.x + dx
        this.y = this.y + dy
    }
}

var point = Point { x: 1, y: 2 }
point.translate(3, 4)
let distance_squared = point.length_squared()
```

Receiver mode является частью публичного контракта, проверки interface implementation и override, а также vtable `dyn`. Implementation/override сохраняет объявленный receiver mode и права остальных параметров. Static методы не входят в instance vtable и не являются instance `virtual`/`override` methods.

`this` всегда является readonly view, `var this` — mutable view. Instance receiver не имеет owned entry и не потребляется даже при последнем использовании caller binding. `var local = this` не присваивает ownership экземпляра: для non-Copy readonly `this` это ошибка, для `var this` — допустимое mutable перезаимствование, если unique-access проверка проходит. Copy самого значения может создать независимую копию, но не перенести receiver ownership.

Consuming операция над экземпляром целиком записывается как free function или static method с явным value parameter, например `fn store(value: User): Holder<User>` с сохранением O в owning поле. Non-Copy `var` параметр сам вход не потребляет. Остальные параметры instance метода могут иметь owned/borrowed entries по общим правилам. Mutable receiver может извлечь принадлежащий контейнеру owned элемент или поле при сохранении корректного состояния владельца (§93); это не consuming move самого receiver.

Borrowed результат метода сохраняет lifetime receiver/других входов. Owned результат может быть fresh или извлечённым owned значением, но возврат non-Copy `this` даёт только view. Нельзя вернуть view на уничтожаемый temporary receiver; compiler проверяет время жизни соответствующего владельца (§22).

---

# 35. Inheritance

Только class поддерживает concrete inheritance. В Rust backend class A представлен payload struct, trait A и owned Box; B содержит base payload inline и реализует trait A. Upcast сохраняет identity без дополнительной allocation. Rust traits не содержат поля: доступ к base state реализуется accessors.

```nether
class Animal {
    virtual fn speak(this): str {
        return ""
    }
}

class Dog : Animal {
    override fn speak(this): str {
        return "woof"
    }
}
```

Синтаксис наследования использует `:`.

`virtual` разрешает переопределение, `override` объявляет соответствующую реализацию производного class. Эти модификаторы сами по себе не требуют runtime dispatch. Если конкретный тип известен compiler, выбирается его реализация и генерируется статическая специализация (§39).

Для выбора реализации по типу, известному только во время выполнения, требуется `dyn Animal` (§40). Нельзя незаметно превратить обычный `Animal` в динамический вызов.

---

# 36. Interfaces

```nether
interface Printable {
    fn print(this): ()
}
```

Class:

```nether
class User : Printable {
    fn print(this): () {
        ...
    }
}
```

Struct:

```nether
struct Point : Printable {
    fn print(this): () {
        ...
    }
}
```

---

# 37. Class + interfaces

```nether
class Dog : Animal, Printable, Serializable {
}
```

Правило:

```text
максимум один class parent
+
любое поддерживаемое количество interfaces
```

---

# 38. Generics

Generics используют небольшой набор возможностей с привычным синтаксисом `Type<T>` и `fn_name<T>(...)`. Они доступны для class, struct, enum, interface, функций и методов.

```nether
class Box<T> {
    value: T
}

fn identity<T>(value: T): T {
    return value
}
```

Типы выводятся из аргументов и ожидаемого результата, когда решение однозначно. Можно указать type arguments явно. Compiler проверяет тело по объявленным constraints; случайные методы конкретной подстановки не становятся доступными без соответствующего контракта.

```nether
let value = identity<i32>(10)

fn print_value<T: Printable>(value: T): () {
    value.print()
}

fn save<T: Printable & Serializable>(value: T): () {
    value.print()
    write_serialized(value)
}
```

После `:` задаётся один base class и/или несколько interfaces, соединённых `&`. Если base class есть, он первый и единственный. Все constraints должны выполняться одновременно. `&` в constraint не вводит произвольные intersection types в остальных type expressions. `Copy` может использоваться как compiler-checked constraint, соответствующий §94.

```nether
fn duplicate<T: Copy>(value: T): (T, T) {
    return (value, value)
}
```

Параметры типов могут иметь defaults:

```nether
class Counter<T = i32> {
    value: T
}
```

Обязательные generic параметры предшествуют параметрам с defaults. Default может ссылаться на ранее объявленные параметры и должен выполнять constraint. Сначала выполняется inference; default используется, когда аргумент не указан и inference не дал решения. Неоднозначность не маскируется произвольной подстановкой default.

Mutable capability проверяется в параметрах и операциях, а `owns`/`view` не являются пользовательскими generic arguments. Compiler формирует owned и borrowed варианты для каждой применимой специализации (§95). Generic container types инвариантны: `Vec<Dog>` не преобразуется автоматически в `Vec<Animal>`, независимо от наследования class.

Generic `value: T` предоставляет readonly binding при любой подстановке; его owned entry может передать значение в mutable binding по §24. `var value: T` допускает mutable borrowed доступ или получение mutable owned значения через проверенный move/Copy. Constraint или известный concrete type сами по себе не повышают права borrowed view. Для изменения generic receiver вызываемый interface method должен объявлять `var this`, а receiver должен иметь mutable capability.

Const-параметры сохраняются как ограниченная возможность для размеров fixed arrays:

```nether
fn length<T, const N: usize>(values: {T; N}): usize {
    return N
}
```

В первой версии const generic имеет тип `usize`. Аргумент — compile-time integer constant или выражение над const-параметрами, вычислимое при специализации. Размер проверяется на допустимость и является частью типа `{T; N}`. Это не общая система compile-time вычислений над произвольными типами и значениями.

Рекурсивные структуры данных используют class references; бесконечное inline-вложение struct или enum не допускается. Weak разрешён только в полях class (§89).

В первоначальный набор не входят пользовательские lifetime-параметры, associated types/GAT, higher-ranked bounds, specialization по перегрузке constraints, conditional/mapped types или произвольные type-level вычисления. Generic methods используют статический вызов; generic method без выбранной конкретной специализации не включается в runtime vtable `dyn`.

Variadic type packs ограничены параметрами функций и описаны отдельно (§96). Они не превращают generics в общий язык вычислений над списками типов.

Знакомые формы inference, constraints и defaults: [TypeScript generics](https://www.typescriptlang.org/docs/handbook/2/generics.html). Символы `:`/`&`, инвариантность и ограничение набора возможностей являются правилами Nether.

---

# 39. Static polymorphism

Generics и interface constraints по умолчанию специализируются AOT.

```nether
print_value(user)
print_value(point)
```

может дать:

```text
print_value_User
print_value_Point
```

Специализация также учитывает доказанный конкретный тип class:

```nether
fn speak(animal: Animal): str {
    return animal.speak()
}

let sound = speak(new Dog())
```

При известном типе `Dog` compiler выбирает `Dog.speak` через статический вариант `speak`. Если конкретный тип невозможно статически установить, динамическое использование должно быть выражено через `dyn`; compiler сообщает об отсутствующем контракте вместо скрытого runtime dispatch.

Число вариантов влияет на время компиляции и размер машинного кода. Compiler может объединять эквивалентные варианты, сохраняя семантику и отсутствие обязательного динамического вызова.

---

# 40. `dyn`

Runtime polymorphism выражается через `dyn`; class B допустим там, где ожидается базовый A. При известном concrete type используется static specialization, при runtime выборе — trait object с сохранением ownership:

```nether
fn execute(handler: dyn Handler): () {
    handler.handle()
}
```

Рекомендуемое представление:

```text
object pointer
+
vtable pointer
```

`dyn Handler` и `dyn Animal` допускают выбор реализации во время выполнения. Если конкретная реализация доказана, compiler может убрать динамический вызов как оптимизацию.

`dyn` не отменяет lifetime и mutable constraints. Ownership/view выбираются по общим правилам (§94–95), а не по одному написанию `dyn`.

Readonly `dyn` binding предоставляет только instance методы с receiver `this`. Для метода с `var this` требуется mutable `dyn` и доказанный unique access. Owned `dyn` можно перенести из `let` в `var` по §24; readonly borrowed `dyn` нельзя повысить до mutable. Упаковка, upcast и devirtualization сохраняют ownership, lifetime и ограничения borrowed компонентов. Vtable instance methods всегда получают borrowed receiver соответствующего mode.

Borrowed `dyn` содержит view на существующие данные и vtable. Он сохраняет lifetime и capability исходного значения. Borrowed представление struct не перемещает его из inline storage и не требует owned boxing.

Owned `dyn` содержит одного владельца скрытого concrete value. Его контракт и vtable предоставляют необходимые size/alignment, destruction и storage operations. Уничтожение вызывает destructor конкретного типа, затем освобождает принадлежащее ему storage.

```nether
let handler: dyn Handler = new HttpHandler()
handler.handle()
```

Здесь ownership class object передаётся в `handler`; отдельная allocation для wrapper не обязательна. Для owned inline struct compiler выбирает storage, соответствующий размеру, alignment и lifetime скрытого значения. Если подходящего inline/region storage нет, нужна allocation. Пользовательский `Box` для этого не требуется, но `dyn` не обещает отсутствие allocation для любого concrete type.

Owned оболочка не делает borrowed поля скрытого значения самостоятельными владельцами: зависимости от внешних lifetime сохраняются. `dyn` не получает автоматический `Copy` или виртуальный clone, даже если конкретная реализация является Copy.

Vtable содержит только методы с фиксированной erased сигнатурой и необходимыми capability/lifetime-контрактами. Неспециализированные generic methods не входят в динамический API (§38). Цена явно выбранного `dyn` — metadata, возможное отдельное storage и runtime dispatch, который compiler может устранить при доказанной реализации.

Аналог представления data pointer + vtable: [Rust Reference: trait objects](https://doc.rust-lang.org/reference/types/trait-object.html). Ownership и выбор storage выше являются правилами Nether.

---

# 41. Enum

Enum использует Rust-style algebraic data model и является inline value type, как struct и tuple. Его представление включает необходимое различение variants и inline payload; отдельная allocation для самого enum не обязательна. Layout может использовать оптимизации, сохраняющие семантику. Присваивание подчиняется §94; наличие class reference в payload не превращает inline enum в class.

```nether
enum Color {
    Red,
    Green,
    Blue,
}
```

---

# 42. Enum payload

```nether
enum Result<T, E> {
    Ok(T),
    Err(E),
}
```

---

# 43. Named enum payload

```nether
enum Shape {
    Circle {
        radius: f64,
    },

    Rectangle {
        width: f64,
        height: f64,
    },
}
```

---

# 44. `match`

```nether
match color {
    Color.Red => print("red"),
    Color.Green => print("green"),
    Color.Blue => print("blue"),
}
```

---

# 45. Match destructuring

```nether
match shape {
    Shape.Circle { radius } => {
        print(radius)
    },

    Shape.Rectangle { width, height } => {
        print(width * height)
    },
}
```

---

# 46. Match expression

```nether
let area = match shape {
    Shape.Circle { radius } => 3.14159 * radius * radius,
    Shape.Rectangle { width, height } => width * height,
}
```

Match обязан быть exhaustive. Короткая ветвь `=> expression` задаёт значение match. В expression context branch block может задавать локальный результат своим последним expression statement; добавление опционального `;` не меняет этот результат. Block без такого выражения даёт `()`. Это результат выражения match, а не неявный return содержащей функции.

Для нескольких инструкций перед возвратом полезного результата можно использовать явный `return` в ветвях функции:

```nether
fn area(shape: Shape): f64 {
    match shape {
        Shape.Circle { radius } => {
            trace_circle(radius)
            return 3.14159 * radius * radius
        },
        Shape.Rectangle { width, height } => {
            return width * height
        },
    }
}
```

Этот `return` завершает содержащую функцию, а не только ветвь match. Ветви с return/panic не обязаны давать локальное значение match; типы продолжающих выполнение expression branches должны согласовываться. В statement context результаты обычных ветвей отбрасываются.

---

# 47. Option

```nether
enum Option<T> {
    Some(T),
    None,
}
```

Обычные class references не nullable.

Используется:

```text
Option<User>
```

вместо implicit null reference.

---

# 48. Result

```nether
enum Result<T, E> {
    Ok(T),
    Err(E),
}
```

---

# 49. Inline array + match example

```nether
let values: {i32; 4} = {2, 4, 5, 1}

for value in values {
    print(value)
}
```

Никакого heap allocation для массива не требуется.

---

# 50. Vec example

```nether
var values: Vec<i32> = [2, 4, 5, 1]

values.push(10)
```

`Vec<i32>` может динамически изменять capacity.

---

# 51. Struct inside Vec

```nether
struct Point {
    x: f32
    y: f32
}

let points: Vec<Point> = [
    Point { x: 1, y: 2 },
    Point { x: 3, y: 4 },
]
```

Memory может быть:

```text
[x,y][x,y][x,y]
```

без references между элементами и storage.

---

# 52. Class inside Vec

```nether
let users: Vec<User> = [
    new User("Alice"),
    new User("Bob"),
]
```

Концептуально:

```text
Vec
├── ref → User
└── ref → User
```

Новые `User` передаются через автоматически выведенный move в самостоятельные owning contexts элементов контейнера. Существующие views сохраняют зависимость от lifetime их целей. Удаление owned элемента завершает его lifetime независимо от lifetime самого `Vec`, если операция не передаёт владение результату (§93).

---

# 53. Регионы

Регион — размещение/анализ lifetime внутри NMM, не отдельная модель памяти.
Owning-рёбра образуют ацикличный граф. См. NMM §9 и §13.

---

# 54. Region allocation

Оптимизация allocation обязана сохранять identity, O/R/W и наблюдаемый
cleanup. Физический регион не удерживает логически удалённый объект живым.

---

# 55. Region inference

Вывод регионов сохраняет нормативные lifetime и cleanup NMM; недостаточное
доказательство приводит к отказу компиляции, без fallback GC/ARC.

---

# 56. Циклы и регионы

Каждая итерация сохраняет initialization, loans и O transitions.
История замен не накапливает живые объекты. Owning-cycle не допускается.

---

# 57. Асинхронные операции

Целевой контракт — [async runtime](async-await-runtime.md), NMM §18.
Stackless state и MaySuspend не меняют NMM. Source async пока не реализован.

---

# 58. Async state

Состояние задачи — owned aggregate; loans через suspension проверяются
статически по NMM §18.3. Raw pointer в state сам по себе не доказывает lifetime.

---

# 59. Await boundary

Suspension является явным CFG effect. Borrowed state не переживает target;
недоказанная безопасность отклоняется. См. NMM §18 и async runtime.

---

# 60. Mutable state между await

W остаётся эксклюзивным. Права при suspension и cancellation определяет
NMM; owned state не отмывает readonly provenance.

---

# 61. Structured async

Task scope и cancellation подчиняются NMM §18; cancellation очищает
initialized state ровно один раз. Реализация source API не завершена.

---

# 62. Detached tasks

Detached state получает необходимые O и не сохраняет escaping borrowed
local targets. См. NMM §18.

---

# 63. Потоки

Изолированные контексты обмениваются изменяемыми данными через передачу O
по каналам. Thread runtime не заменяет проверку NMM.

---

# 64. Каналы

Отправка передаёт O транзакционно; failure возвращает ownership по контракту.
R/W aliases не становятся отправляемыми владельцами. См. NMM §18.1.

---

# 65. Сообщения

Transfer проверяет весь достижимый owning граф и borrowed/weak dependencies.
Объект получает одного владельца в принимающем контексте.

---

# 66. Межконтекстные ссылки

Ссылки не становятся shared mutable доступом между контекстами.
Каналы, weak и guards сохраняют изоляцию по NMM §14/§18.

---

# 67. Transferable memory

Transferability выводится из O, origins и lifetime; физический region
и сырой адрес не создают разрешение на передачу.

---

# 68. Уничтожение

Owned lifetime завершается детерминированно по NMM §15. Последний use
alias не уничтожает owner и не переносит его cleanup.

---

# 69. Деструкторы

Destructor body предшествует cleanup initialized fields в declaration order.
Roots scope очищаются в обратном порядке успешной инициализации текущих
lifetimes; replacement обновляет позицию root после commit. Finalizing
target нельзя целиком заменить или воскресить. Panic/partial construction
следуют NMM §15–§16; repeated panic приводит к abort.

---

# 70. Arrays summary

В языке существуют два разных синтаксиса:

```nether
let fixed: {i32; 4} = {1, 2, 3, 4}
```

Это:

```text
inline fixed-size array
```

А:

```nether
let dynamic: Vec<i32> = [1, 2, 3, 4]
```

это:

```text
dynamic vector
```

---

# 71. Strings summary

```nether
let letter: char = 'a'

let view: str = "hello"

let owned: String = "hello"
```

Конкретное размещение строкового literal выбирает compiler.

---

# 72. Function syntax summary

Функция объявляется как `fn name(args): Result { ... }`. Для функции с
дополнительными обязательствами используется `unsafe fn`; её вызов требует
явного unsafe block. Named function values сохраняют parameter/result contracts.
Source closures определены NMM §17, но пока отклоняются как unsupported;
они не являются альтернативой правилам памяти. См. [профиль](0.1/profile.md).

---

# 73. Compiler type representation

Compiler должен иметь distinct primitive types:

```text
I8
I16
I32
I64
I128

U8
U16
U32
U64
U128

F32
F64

Bool
Char
Str
String
Unit
Usize
Isize

Tuple<T...>
InlineArray<T, N>
Vec<T>

Struct<Symbol>
Class<Symbol>
Enum<Symbol>

Function<ParamsWithCapabilities..., Return, LifetimeContract>
AsyncOperation<State, Output, LifetimeContract>
WeakField<ClassTarget, DomainSlotGeneration>

Dyn<Interface, OwnershipCapability, AccessMode, LifetimeContract>
RawPointer<Pointee, ConstOrMut, ForeignABI>
```

Для каждого binding/place и производного view compiler отдельно хранит ownership mode, readonly/mutable access, происхождение capability и lifetime. Access не выводится только из имени `T`, указательного layout или наличия owning root. Method symbol хранит receiver kind `static | readonly_this | mutable_this`; эти сведения участвуют в проверке до codegen и не требуют runtime tag на каждом обычном объекте.

---

# 74. Function type AST

Для:

```text
(i32, str) => bool
```

AST должен концептуально содержать:

```text
FunctionType {
    parameters: [
        i32,
        str
    ],
    return_type: bool
}
```

Нельзя моделировать function type как closure type. Mutable capability входит в описание параметра, а выведенные ownership/lifetime и async effects сохраняются в callable-контракте.

Параметры без access modifier имеют R; consuming entry может передавать O дальше, но не повышает доступ в mutable. Receiver method AST хранит отдельный первый параметр с типом содержащего типа, mode `readonly`/`mutable` и обязательным borrowed ownership; отсутствие receiver обозначает static method. Return contract различает owned результат и borrowed capability/lifetime, сохраняя ограничения компонентов.

```text
(var User) => ()
```

```text
FunctionType {
    parameters: [{ type: User, access: mutable, borrowed_capability: unique }],
    return_type: Unit
}
```

---

# 75. Inline array AST

Для:

```text
{i32; 4}
```

AST:

```text
InlineArrayType {
    element: i32
    length: 4
}
```

Для:

```nether
{1, 2, 3, 4}
```

AST:

```text
InlineArrayExpression
```

---

# 76. Vec AST

Для:

```nether
[1, 2, 3]
```

AST:

```text
VecLiteralExpression
```

а не generic array expression.

Это различие должно существовать уже на parser/AST уровне:

```text
{ ... } в позиции выражения → fixed inline array
{ ... } в позиции тела конструкции → block
[ ... ] → Vec
```

---

# 77. String literal AST

```nether
"hello"
```

должен первоначально быть отдельным:

```text
StringLiteral
```

а не немедленно `String`.

Type checker решает, coerces ли literal в:

```text
str
String
```

или иной строковый тип.

---

# 78. Char literal AST

```nether
'a'
```

имеет тип:

```text
char
```

и должен содержать ровно один Unicode scalar value.

---

# 79. Static function representation

Обычный function pointer:

```text
(A) => R
```

представляется обычным native code pointer без receiver и captured environment. Stateful callable и `dyn` являются отдельными механизмами; неявной dynamic adaptation этого типа нет.

Например:

```text
(i32) => i32
```

может занимать один machine pointer.

---

# 80. Method references

Если метод требует receiver:

```nether
service.handle
```

это уже не простой function pointer, потому что требуется `service`.

Так как hidden closures запрещены, compiler не должен автоматически создавать:

```text
function pointer
+
captured receiver
```

как скрытый object.

Пользователь должен либо:

- передать object отдельно;
- использовать callable object;
- использовать API, явно понимающий method reference lifetime.

Для передачи receiver отдельно допускается unbound method reference через имя типа:

```nether
let read: (Counter) => i32 = Counter.get
let increment: (var Counter) => () = Counter.increment

var counter = Counter.create()
increment(counter)
let value = read(counter)
```

Первый аргумент такого pointer соответствует `this` / `var this` и всегда имеет borrowed contract, даже при последнем использовании аргумента caller. Pointer не приобретает consuming receiver entry; остальные аргументы следуют выбранным contracts. `Counter.create` не имеет receiver и имеет тип `() => Counter`. Получение `counter.increment` не создаёт неявного captured environment и не может скрыть mutable receiver в readonly function type.

---

# 81. Memory invariants

Единственный нормативный перечень — [NMM §4](core-memory-managment.md#4-обязательные-инварианты).
O/R/W, one owner, static loans, acyclic ownership и deterministic cleanup
обязательны во всех layers, включая trusted unsafe/runtime. Альтернативных
правил памяти в этой спецификации нет.

---

# 82. Базовая программа

```nether
interface Printable {
    fn print(this): ()
}

struct Point : Printable {
    x: f64
    y: f64

    fn print(this): () {
        console.write("Point")
    }
}

class User : Printable {
    name: String
    age: i32

    constructor(initial_name: String, initial_age: i32) {
        name = initial_name
        age = initial_age
    }

    fn print(this): () {
        console.write(name)
    }
}

fn print_value<T: Printable>(value: T): () {
    value.print()
}

fn main(): () {
    let point = Point {
        x: 10.0,
        y: 20.0,
    }

    let numbers: {i32; 4} = {1, 2, 3, 4}

    let dynamic_numbers: Vec<i32> = [1, 2, 3, 4]

    let user = new User("Alex", 24)

    let operation: (i32) => i32 = (x) => x * 2

    print_value(point)
    print_value(user)

    print(operation(numbers[0]))
}
```

---

# 83. Итоговая синтаксическая модель

Основные типы:

```text
i32
i64
u32
u64
f32
f64
bool
char
str
String

()
usize
isize

(i32, i64)

{i32; 4}

Vec<i32>

(A) => R

Point
User

Option<T>
Result<T, E>

dyn Interface
```

`weak` является модификатором поля class, а не самостоятельным типом для произвольного использования (§89).

Основные literals:

```nether
10
10.5
true
'a'
"hello"

(10, 20)

{1, 2, 3, 4}

[1, 2, 3, 4]

Point {
    x: 1,
    y: 2,
}

new User("Alex")
```

---

# 84. Основные принципы

NMM — единственная модель памяти. Синтаксическая простота не разрешает
скрытый clone, GC/strong ARC, runtime borrowing или изменение времени drop.
Unsafe явно обозначает дополнительные обязательства автора; см. [unsafe.md](unsafe.md).

---

# 85. Unit type — `()`

`()` является полноценным unit type с единственным значением `()`, как в Rust. Функция без полезного результата возвращает `()`; прежний отдельный тип отсутствия результата больше не используется.

```nether
fn log(message: str): () {
    println("{}", message)
}

let result: () = log("ready")
let unit: () = ()
```

Пустой block имеет результат `()`, но пустой inline array `{}` имеет тип `{T; 0}` при известном `T`. Это разные значения и типы.

`()` не обозначает невозвращающийся код. `panic` имеет never-result, совместимый с ветвями, которые не продолжают выполнение; отдельное пользовательское написание never type требует уточнения.

Семантическая основа: [Rust Reference: tuple types](https://doc.rust-lang.org/reference/types/tuple.html).

---

# 86. Грамматика выражений, blocks и приоритеты

Parser различает позиции объявления, инструкции, выражения, type и тела конструкции. `{}` определяется синтаксическим контекстом, без type inference:

```nether
let empty: {i32; 0} = {}  // expression: empty inline array

if enabled {}            // body: empty block
while enabled {}         // body: empty block
fn noop(): () {}          // body: empty block
```

После условия `if`/`while`, заголовка `for`/`fn`, `task_scope`, `unsafe`, `extern "C"`, в теле `match` и block-ветви match parser ожидает тело соответствующей конструкции. `unsafe {...}` содержит инструкции с явным допуском unsafe операций; `extern "C" {...}` содержит foreign declarations (§99). В позиции initializer, аргумента или результата `{...}` является inline array. `Point {...}` распознаётся как struct literal. Пустой массив без контекста типа не получает произвольный element type.

Самостоятельный вложенный bare block не является отдельной формой выражения. Blocks, связанные с конструкциями управления, функциями и match branches, разрешены, включая вложенные конструкции. Это явное отличие от общего block-expression синтаксиса Rust.

Expression statements, включая calls и assignments, разрешены. Инструкции разделяются явным `;`, завершающим переносом строки или закрывающей `}` тела. `;` опционален в конце строки и перед `}`; две простые инструкции на одной строке требуют `;`. Добавление `;` не меняет value/return semantics.

LF, CRLF и CR являются переносами строк исходника. Перенос внутри line/block comment также учитывается как граница строк; внутри string/char/template literal он относится к literal grammar. Escape `\n` или `\r` внутри литерала не разделяет инструкции.

В выбранной модели завершённая простая инструкция заканчивается на newline. Перенос игнорируется, если syntax ещё требует продолжения: после `=`, infix operator, `=>`, запятой в списке, внутри `(...)`, `[...]` или data literal `{...}`. Для тел конструкций `{...}` действует grammar инструкций, а не grammar data literal.

```nether
let total = left +
    right

let result = calculate(
    first,
    second,
)

let a = 1; let b = 2
```

Заголовок `if`/`while`/`for`/`fn`/`match`, объявление class/interface и другие конструкции, требующие body, могут отделяться от `{` переносом. Newline между `}` и связанным `else` не завершает if-chain. Атрибут `#[Copy]` остаётся связанным со следующим объявлением. Перенос перед member `.` продолжает цепочку; postfix/infix символы на новом завершённом statement не продолжают предыдущую строку автоматически.

```nether
let result = service
    .load()
    .validate()

run()
(next)  // отдельная инструкция, не аргумент предыдущего run()
```

В частности, начальные `(` и `[` на новой строке после завершённой инструкции не превращаются в продолжение предыдущего call/index. Для multiline infix выражения operator ставится в конце предыдущей строки либо всё выражение заключается в parentheses. Эти правила заимствуют удобство TS, но не воспроизводят все неоднозначности JavaScript automatic semicolon insertion.

После `return`, `break` или `continue` перенос завершает инструкцию. Значение `return`/`break`, если оно есть, начинается на той же строке. Для multiline return используется `return (` с закрывающей скобкой ниже:

```nether
return (
    first + second
)
```

`return` с newline сразу после keyword возвращает `()` и не забирает выражение следующей строки. Для функции с другим return type это compile error. Ограничение line terminator после return соответствует [ECMAScript return statement](https://tc39.es/ecma262/multipage/ecmascript-language-statements-and-declarations.html#sec-return-statement).

Списки function arguments, generic parameters, data literal elements и match branches разделяются запятыми, с разрешённой trailing comma; newline не заменяет comma. В type context закрывающий `>>` может разделять два вложенных generic arguments. Явные generic calls имеют форму `name<T>(args)` без Rust turbofish.

Приоритет поддерживаемых операторов принимается по таблице Rust. От сильного к слабому:

| Уровень | Операторы / выражения | Ассоциативность |
| --- | --- | --- |
| 1 | Paths / имена | — |
| 2 | Method calls | — |
| 3 | Field access | Слева направо |
| 4 | Calls, indexing | — |
| 5 | `?` | Postfix |
| 6 | Unary `-`, `!`; прочие unary при их введении | Unary |
| 7 | `as` | Слева направо |
| 8 | `*`, `/`, `%` | Слева направо |
| 9 | `+`, `-` | Слева направо |
| 10 | `<<`, `>>` | Слева направо |
| 11 | `&` | Слева направо |
| 12 | `^` | Слева направо |
| 13 | `\|` | Слева направо |
| 14 | `==`, `!=`, `<`, `>`, `<=`, `>=` | Цепочки требуют скобок |
| 15 | `&&` | Слева направо |
| 16 | `\|\|` | Слева направо |
| 17 | `..`, `..=` | Цепочки требуют скобок |
| 18 | `=`, compound assignments | Справа налево |
| 19 | `return`, `break`, lambda | — |

Принятие таблицы не вводит автоматически ещё не реализованные операции raw storage или Rust closures. Prefix `await` разбирается как unary form Nether; его операнд включает call/index/member access. Например, `await load_user(id)` ожидает результат вызова. `var` является модификатором declaration/parameter/pattern/receiver, а не unary expression и не prefix call argument. `this` является receiver expression внутри instance method; формы `this` / `var this` в parameter list разбираются по §34.

Основа таблицы и порядка вычисления: [Rust Reference: expressions](https://doc.rust-lang.org/reference/expressions.html#expression-precedence). Операнды обычных операций вычисляются слева направо; `&&`/`||` используют short-circuit evaluation.

---

# 87. Арифметика и control flow

На этой стадии принимается Rust-подобная семантика арифметики: числовые типы не получают неявных преобразований между уже типизированными значениями; явный numeric cast записывается через `as`. Тип литерала определяется контекстом, иначе integer/floating defaults — `i32`/`f64`.

`usize`/`isize` являются pointer-sized integer types. Индексы контейнеров и размеры используют `usize`. Деление целых на ноль вызывает panic. Проверки integer overflow по умолчанию включены в debug и отключены в release, с возможностью явно выбрать режим проверки. При отключённых проверках переполнение `+`, `-`, `*` выполняет wrapping по разрядности типа; при включённых — вызывает panic. Конкретные compiler flags и полный набор casts Nether ещё должны быть перечислены. Signed `MIN / -1` и `MIN % -1` требуют panic даже при отключённых обычных overflow checks.

Семантическая основа: [Rust Reference: operator expressions](https://doc.rust-lang.org/reference/expressions/operator-expr.html).

`if`, `while`, `for`, `loop`, `break`, `continue` и `return` используют Rust-подобный control flow с синтаксисом типов и массивов Nether. Conditions имеют тип `bool`. `for value in values` выполняет iteration, а `break`/`continue` относятся к соответствующему циклу. `loop` может возвращать значение через `break value`.

`if` управляет выполнением block bodies и сохраняет возможность использования как value expression. В expression context последнее expression statement ветви задаёт её локальный результат независимо от опционального `;`; для non-unit результата требуется `else`. В statement context результаты ветвей отбрасываются. Это не является неявным возвратом функции.

`match` может быть value expression с короткими `=> expression` branches или локальными результатами branch blocks (§46). `loop` может иметь явный результат `break value`. Типы продолжающих выполнение ветвей/result breaks согласовываются. Полезный результат функции возвращается только явным `return`; `return` внутри if/loop/match block завершает содержащую функцию. Обычный function body не имеет tail result.

```nether
fn choose(enabled: bool): i32 {
    if enabled {
        return 1
    }
    return 0
}

fn choose_expression(enabled: bool): i32 {
    return if enabled { 1 } else { 0 }
}
```

Отсутствие bare block expressions, обязательный явный return функции и независимость локального результата от опционального semicolon являются установленными отличиями от общего Rust block-expression синтаксиса. Точное разделение инструкций определяется §86.

---

# 88. Patterns и права bindings

Patterns сохраняют access/origins и initialization по NMM §16.
`ref`/`ref var` создают R/W loans; owning extraction требует O.
Наличие/отсутствие будущего use само по себе не выдаёт ownership и не повышает R.
Partial extraction из типа с пользовательским destructor ограничивается NMM.
Синтаксис и exhaustiveness проверяются frontend/type checker.

---

# 89. Weak и guard

Единственный контракт — [NMM §14](core-memory-managment.md#14-weak-и-временный-read-guard).
Weak не владеет payload; lookup даёт Option и временный R guard, защищающий
цель на всё использование. Generation предотвращает reuse/ABA, но сама не
доказывает безопасный доступ. Source implementation пока отсутствует.

---

# 90. Форматирование и interpolation

Поддерживаются Rust-подобные format placeholders и JS-подобные interpolated literals:

```nether
println("{}", value)
println(`${value}`)
let message: String = fmt("value = {}", value)
```

Formatted functions используют statically typed variadic pack (§96), а не обязательное преобразование каждого аргумента в отдельный `String`:

```nether
fn println<...Args: Format>(template: str, ...values: Args): ()
fn fmt<...Args: Format>(template: str, ...values: Args): String
```

`Format` является interface записи значения в writer; constraint применяется к каждому элементу pack. Built-in primitives и строковые типы поддерживают стандартное форматирование. Пользовательский тип реализует соответствующий interface явно.

```nether
interface Format {
    fn write_to(this, var writer: Writer): ()
}
```

Форматирующий код читает значения pack через views. Владение входными аргументами выбирается по контрактам §94–95; форматирование само по себе не добавляет consuming transfer или отдельный String для каждого значения.

Для постоянного шаблона compiler разбирает placeholders, проверяет число/типы аргументов и генерирует запись непосредственно в writer. `println` не требует промежуточной строки для каждого значения. `fmt` создаёт один итоговый owned `String`; allocation для результата является стоимостью выбранной операции.

Interpolated literal lowering учитывает назначение: при непосредственном formatted output compiler может писать части напрямую, а в контексте owned `String` формируется строка. Вызов произвольной функции должен соблюдать её реальный контракт и не получает автоматически writer semantics.

Выражения interpolation вычисляются однократно в порядке появления. `\n`, `\r` и остальные утверждённые escape sequences применяются внутри соответствующих литералов; полная grammar escaping и динамических шаблонов пока требует описания.

---

# 91. Модули, exports и отдельная компиляция

Imports используют следующий стиль:

```nether
import { User, load_user } from "app/users"
```

Объявления и члены по умолчанию public; `public` является опциональным явным модификатором, `private` ограничивает доступ. `private` на верхнем уровне закрывает объявление за границей модуля, а private member доступен внутри объявляющего class/struct. Публичный entry module пакета может собирать API через re-export:

```nether
export { User, load_user } from "app/users"
```

Compiler сохраняет вместе с библиотекой:

- публичные сигнатуры и необходимые type/layout metadata;
- контракты owned и borrowed entry variants, consuming effects и lifetime-связи;
- Copy properties и provenance/эффекты weak lookup, включая возможные destruction domains;
- контракты самостоятельного владения элементами, передачи и уничтожения owned результатов;
- явный static/readonly/mutable receiver, access modes параметров и provenance прав результата/полей;
- async effects и ограничения thread transfer;
- представление generic bodies, необходимое для специализации.

Вызывающий код проверяется по этим контрактам без чтения исходного тела обычной функции. Для generic specialization поставляется compiler-readable представление. Пользователь обычно не пишет выведенные контракты; IDE может их показывать. Изменение такого контракта может менять совместимость публичного API.

Imports связываются при сборке, не загружают исполняемый код во время выполнения и не запускают скрытую module initialization. Модули содержат declarations и compile-time constants; runtime initialization вызывается явно из `main` или другой функции. Compiler сначала собирает signatures, поэтому циклические ссылки между declarations разных модулей одного пакета разрешены. Циклическое вычисление constants и circular package dependencies запрещены (§98).

Private member базового class остаётся доступным только в объявляющем class; inheritance не открывает его производному class. Правила package resolution и совместимости metadata закреплены в §98, foreign boundary — в §99. Конкретное binary encoding metadata является деталью toolchain, а не переносимым ABI языка.

---

# 92. Ошибки, propagation и unwinding

`Option<T>` выражает отсутствие значения, `Result<T, E>` — ожидаемую ошибку. Postfix `?` выполняет Rust-подобную propagation в совместимом возвращаемом `Result` или `Option` без обязательной heap allocation и без скрытой panic.

```nether
fn read_value(): Result<i32, Error> {
    let value = read_number()?
    return Result.Ok(value)
}
```

`panic(...)` прерывает обычное выполнение и запускает unwinding с cleanup живых ресурсов. Обычные ошибки остаются значениями `Result`, а panic не заменяет их автоматически.

При unwinding выполняются необходимые destructors и cleanup regions в порядке §69. Async scope сначала проходит child cancellation/join barrier (§61) и не освобождает память, пока дети продолжают обращаться к ней.

Если destructor или другой cleanup вызывает panic во время уже активного unwinding/cleanup из-за panic, процесс выполняет abort: второй unwind поверх первого не запускается. Одна panic из destructor при обычном выходе начинает обычный unwind и cleanup оставшихся частей. Независимые child panics собираются barrier scope (§61) и не считаются сами по себе повторной panic внутри одного cleanup.

Panic не пересекает plain C ABI. Export boundary должен перехватить panic, завершить безопасный cleanup и преобразовать её в явно выбранный внешний результат; foreign exceptions также должны быть обработаны до входа в Nether (§99).

Ожидаемые ошибки закрытия/flush возвращаются из явного `finish` через `Result`, а не из destructor (§69). Unwinding требует cleanup metadata и кода на пути ошибки; отсутствие GC/ARC не означает нулевую стоимость всех failure paths.

Основа cleanup при unwinding: [Rust Reference: panic](https://doc.rust-lang.org/reference/panic.html#unwinding). Для Nether здесь утверждён unwind; наличие Rust-подобного альтернативного abort-профиля не считается автоматически утверждённым.

---

# 93. Структуры и графы

Owning-рёбра ацикличны; cycles возможны через невладеющие связи при
проверенных lifetime либо weak. Group/region не легализует owning-cycle.
Независимое remove завершает lifetime немедленно, guards ограничивают
invalidation. См. NMM §12–§15.

---

# 94. Assignment и Copy

Для non-Copy existing place привязка создаёт alias. O передаётся только
в owning-контексте, не по последнему use. Атрибут Copy создаёт независимое
значение; Copy class имеет отдельную identity, borrowed компоненты сохраняют
origins. Copy + пользовательский destructor запрещён. См. NMM §6–§10.

---

# 95. Параметры и consuming contracts

Ordinary non-Copy параметр предоставляет R; `var` — W borrow и не потребляет
сам вход. Consuming-effect ordinary параметра выводится из owning sinks,
сохраняется через generics/modules/indirect calls и требует O от caller.
Alias не выбирает borrowed entry для обхода consuming-effect. O не повышает R
в W. Receivers остаются borrowed. Results сохраняют access provenance;
возврат внешнего view не создаёт владельца. См. NMM §11 и §20–§21.

---

# 96. Variadic packs

Выбраны типизированные packs с известной при compilation формой. Это параметризация функции числом и типами аргументов, а не автоматическое создание `Vec`, массива строк или heap-allocated argument list.

## 96.1. Объявление

Heterogeneous pack объявляется как `...Args` в конце generic parameter list и `...values: Args` в конце function parameter list:

```nether
fn print_all<...Args: Format>(...values: Args): () {
    for value in values {
        println("{}", value)
    }
}

print_all(10, "ready", true)
```

Каждый элемент pack имеет собственный конкретный тип. `Args: Format` в pack declaration требует Format у каждого элемента; поддерживается пересечение constraints той же формы, что §38. В function допускается один type pack и один соответствующий value pack; оба последние в своих списках. У pack нет generic default. Допускается пустой pack.

Homogeneous форма не требует отдельного type pack в исходнике:

```nether
fn sum(...values: i32): i32 {
    var total = 0
    for value in values {
        total = total + value
    }
    return total
}

let total = sum(1, 2, 3)
```

Здесь каждый argument имеет тип `i32`, а число элементов по-прежнему известно при compilation. Parameter после pack, несколько packs, `var` pack parameter и произвольные вычисления над pack types в первой версии не допускаются. Pack elements доступны readonly; существующие variadic interfaces форматирования требуют только чтения элементов. Mutable операции задаются отдельными `var` параметрами.

## 96.2. Использование и forwarding

Pack существует как compiler-known список значений, а не как публичный самостоятельный runtime type. Его можно обойти через `for`, получить `values.len()` как compile-time `usize` или передать далее через spread:

```nether
fn log<...Args: Format>(template: str, ...values: Args): () {
    println(template, ...values)
}
```

При heterogeneous iteration compiler проверяет тело для каждого element type и специализирует или разворачивает iteration. Если операция недопустима хотя бы для одного типа, функция не проходит проверку constraints. Однородный pack может lowering в обычный цикл или unrolled code; выбранная реализация не меняет порядок выполнения.

Spread `...value` в call поддерживается для pack, tuple и fixed array известной длины. Он подставляет отдельные arguments с их lifetime/capability, без deep copy или обязательного промежуточного контейнера. Аргументы вычисляются один раз слева направо, а правила Copy/move/view применяются по §94–95.

Для `Vec` или строки динамической длины spread в static pack недопустим. Для динамического набора используется обычная функция, принимающая контейнер, с borrowed/owned контрактом; pack не получает скрытый динамический dispatcher.

Pack нельзя сохранить или вернуть как неописанный runtime объект. Если нужен самостоятельный tuple или container, создаётся явное значение с его обычными lifetime и ownership правилами. Полученные в iteration views не переживают владельцев соответствующих аргументов.

## 96.3. Specialization и стоимость

Каждая форма вызова определяет длину и типы pack. AOT compiler создаёт применимую специализацию и её owned/borrowed entry contracts. В function value можно сохранить конкретную специализацию с фиксированной сигнатурой; C-style variadic ABI без типов не вводится.

Для постоянного format template число и совместимость аргументов проверяются при compilation. Динамический template может потребовать runtime parsing, но типы аргументов pack остаются известными; обработка некорректного динамического template задаётся отдельным library API.

Модель не требует allocation pack, boxing или преобразования каждого значения в `String`. Реальные allocations аргументов, итогового `String` и операции вывода сохраняют свою стоимость. Цена specialization — возможный рост compilation time и code size; compiler может объединять эквивалентные реализации.

Запись rest/spread знакома по [TypeScript functions](https://www.typescriptlang.org/docs/handbook/2/functions.html#rest-parameters-and-arguments), но static heterogeneous packs, compile-time iteration и отсутствие обязательного runtime array являются собственными правилами Nether.

---

# 97. Equality и class identity

Для совместимых class operands `==` сравнивает identity, а `!=` является отрицанием этого сравнения. Равенство полей не делает две отдельно созданные class identities равными. Move сохраняет identity; `new` и Copy существующего class value создают независимую identity (§94).

Сравнение identity является intrinsic readonly borrowed operation: operands не потребляются и не копируются, включая class с атрибутом Copy. Явный `same_identity(first, second)` принимает class views по тому же правилу и не сохраняет mutable capability. Это проверка identity существующих объектов, а не обычная value-передача с автоматическим Copy.

```nether
let first = new User()
let second = new User()
let equal = same_identity(first, second)  // false
```

View через base class или class-backed `dyn` сохраняет identity полного объекта; сравнение не зависит от адреса конкретной base части. Struct-backed `dyn` не приобретает class identity. Несвязанные типы без совместимого class view не сравниваются автоматически.

Числа совместимого типа, `bool` и `char` сравниваются по значению. Для floating-point сохраняются IEEE правила, включая `NaN != NaN` и равенство положительного/отрицательного нуля. `str` и `String` сравнивают содержимое UTF-8, в том числе между собой, без автоматической Unicode normalization, consuming move или копирования буфера.

Для пользовательского сравнения содержимого class используется явный метод, например `equals(other)`. Intrinsic class `==` не перенаправляется в такой метод. Для inline struct/tuple/enum и `Vec` identity-сравнение не вводится; автоматическая генерация structural equality из layout не является частью этой версии. Сравнение содержимого задаётся явной функцией или методом соответствующего типа.

Identity является логическим свойством. Язык не требует публичного raw address или глобального numeric ID каждого class object. Compiler может использовать stack/region allocation и устранить ненаблюдаемый объект, сохраняя результаты сравнения. Если создан weak handle, его domain/slot/generation различает identities по §89.

---

# 98. Пакеты, сборка и Nether ABI

`nether.toml` описывает пакет, его module entry и зависимости с допустимыми версиями или явными источниками. `nether.lock` фиксирует точные версии, источники и идентификаторы содержимого разрешённых зависимостей. Сборка по lock использует этот dependency graph; его обновление является отдельной явной операцией, а не скрытым изменением каждой сборки.

Путь `"app/users"` разрешается через package name/alias `app`, затем module path `users`. Имя текущего пакета и aliases зависимостей задаются manifest; alias однозначен внутри пакета. Относительные `"./users"` и `"../users"` разрешаются от импортирующего модуля и не могут выходить за module root пакета. Entry module определяет публичный API; доступ к private declarations запрещён независимо от формы import.

Module imports являются compile-time связями (§91). Циклы между type/function declarations модулей допустимы, если signatures и layouts можно разрешить без циклического вычисления. Рекурсивный inline layout бесконечного размера запрещён; рекурсия через class references допустима. Circular dependencies между пакетами в первой версии отвергаются. Runtime startup вызывается явно, без порядка побочных эффектов module imports.

Библиотека поставляет машинный код и metadata из §91. Metadata включает необходимые public layouts, Copy properties, owned/borrowed entries, lifetime/capability-контракты, cleanup/weak effects, cancellation и thread-transfer ограничения, а также generic representation для AOT specialization. Эти контракты являются частью публичного API: добавление consuming effect, сужение lifetime или несовместимое изменение layout может ломать вызывающий код даже при неизменном имени функции.

Compiler проверяет metadata format version, target, ABI и совместимость compiler/runtime contract до linking. Несовместимая библиотека требует пересборки либо использования явной foreign boundary; её metadata не игнорируется ради успешного link. Версионирование пакета должно учитывать breaking changes этих контрактов, а не только текстовые сигнатуры.

В первой версии Nether ABI привязан к совместимой toolchain и target. Стабильная binary compatibility произвольных версий compiler не обещается. Точное encoding metadata и способ доставки пакетов являются деталями tooling; они не изменяют семантику ownership. Для независимых внешних библиотек используется явно выбранный C ABI (§99).

Manifest описывает требования, lock — конкретное разрешение зависимостей; аналог разделения: [Cargo.toml vs Cargo.lock](https://doc.rust-lang.org/cargo/guide/cargo-toml-vs-cargo-lock.html).

---

# 99. Unsafe и FFI

Rust-like unsafe определяется [unsafe.md](unsafe.md) и NMM §19.
`unsafe fn` объявляет caller obligations; вызов требует явный unsafe block,
включая тело другой unsafe fn. Обычные NMM checks не отключаются.
FFI/raw storage требуют явных ABI, layout, provenance и release contracts;
unwind не пересекает обычную C ABI boundary. Source FFI пока не реализован.

---


## Текущее расширение 0.1: unsafe callable и statement loop

Реализован `unsafe (T) => R` с сохранением safety в generics, aggregates,
results и imports. Получение pointer не требует блока; вызов требует explicit
unsafe. Safe/unsafe types различны и пока не имеют implicit coercion.

`loop { ... }` реализован как цикл с истинным условием с обычными break/continue,
return и deterministic cleanup. Loop без break текущего уровня считается
непродолжающим выполнение. `break value` и value-result loop из целевого
синтаксиса выше ещё не реализованы; такой синтаксис отвергается.


## Текущее расширение 0.1: scalar raw pointers

`*T` / `*var T` — невладеющие Copy addresses. Реализованы `&place` / `&var place`
для scalar places, unsafe `*pointer`, `*pointer = value`, `pointer + isize` и
writable → readonly cast с неизменным pointee. Адресуемые scalar types:
integer/bool/char/f32/f64, в том числе поля и элементы fixed arrays.
Остальные raw/storage operations остаются открытыми; точные границы,
caller obligations и пример safe wrapper — в [unsafe.md](unsafe.md).
