# Nether Language Specification Draft v0.1

## 0. Назначение

Этот документ фиксирует базовый синтаксис и семантику языка до начала реализации компилятора.

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
- использует region-based memory management;
- поддерживает самостоятельное владение owned элементами контейнеров и регионы связанных групп;
- автоматически выводит ownership, `view`, lifetime и move с учётом последующих использований;
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
- не имеет closures;
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

`String` по умолчанию не является `Copy`: присваивание передаёт владение при допустимом move или создаёт view при последующем использовании источника (§94). View не дублирует буфер и не создаёт нового владельца. Отдельное содержимое строки требует явной операции копирования; compiler не подставляет её для обхода конфликта.

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

# 21. Ownership и автоматический move

Class object имеет одну identity и один логический owning context в каждый момент времени. Контекст создания является исходной точкой анализа; он не обязывает compiler физически выделять возвращаемый объект в локальном регионе.

Ownership принадлежит контексту; binding позволяет compiler отслеживать root или view. Результат присваивания определяется типом источника, наличием `Copy`, правом передачи ownership и последующими использованиями исходного значения (§94).

```nether
let user = new User()
let second = user
print(user.name)  // user ещё используется: second является view
```

```text
owning context → User identity
user          → root binding
second        → view на ту же identity
```

Для источника без `Copy`, если owned значение можно передать и его прежний binding больше не используется, присваивание выполняет move:

```nether
let user = new User()
let moved_user = user  // owned передача: user больше не используется
```

Инициализация новым литералом, `new` или свежим owned результатом передаёт получателю создаваемое значение. Пользователь не пишет `owns`, lifetime-параметры или обычный `move`.

Access mode выбирается независимо: `let` предоставляет readonly-доступ через binding. `var` может получить mutable view, fresh/Copy значение или non-Copy owned значение через допустимый move, включая источник с readonly binding. Получение mutable-доступа к owned данным требует проверки конфликтующих заимствований; readonly view не повышает права (§23–24).

```nether
fn make_user(): User {
    return new User()
}

let user = make_user()
```

Compiler может выделить объект сразу в подходящем регионе получателя, без временного объекта и копирования содержимого. Move ownership не означает обязательное физическое перемещение class identity.

```nether
fn add_user(var users: Vec<User>): () {
    users.push(new User())
}
```

Если позиция принимает owned объект, compiler выводит передачу в контекст назначения. Non-Copy view не превращается автоматически в owned объект. Для Copy-источника value assignment создаёт самостоятельное owned значение, сохраняя источник (§94).

После consuming move прежний источник нельзя читать до повторной инициализации. Все сохраняющиеся aliases проверяются относительно lifetime назначения; если безопасность передачи нельзя доказать, move отклоняется. Наличие будущих обращений к исходному binding приводит к borrowed варианту, если контракт операции его допускает (§95); иначе возникает compile error.

Переприсваивание owned root binding завершает lifetime предыдущего owned объекта, если владение не передано в другое место. Используемые views запрещают destruction. Переприсваивание borrowed binding меняет только view. Новое значение справа выбирается по тем же правилам move/view/Copy. Правила элементов контейнеров и связанных групп приведены в §93.

---

# 22. Automatic `view` и lifetime-контракты

`view` отсутствует в обычном пользовательском синтаксисе. Compiler выводит его при borrowed доступе и когда последующие использования не позволяют передать owned источник.

```nether
fn print_user(user: User): () {
    print(user.name)
}

print_user(user)
print(user.name)  // требуется borrowed вариант print_user
```

Для функции формируются применимые owned и borrowed варианты (§95). Выбор ownership режима выполняется статически по аргументам и их последующим использованиям. Access mode фиксируется объявлением параметра: обычный `user: User` readonly в обоих вариантах, `var user: User` требует mutable права. Lifetime и ownership annotations пользователю не требуются.

```nether
fn identity<T>(value: T): T {
    return value
}
```

В owned варианте non-Copy аргумент передаётся в результат. В borrowed варианте результат является view, связанным с lifetime аргумента. Для Copy значения value assignment и value return создают самостоятельное owned значение по §94. Новый объект, возвращаемый `make_user`, является owned результатом независимо от режима других параметров.

Compiler сохраняет связи lifetime между параметрами, receiver, результатом и местами сохранения ссылок. Это контракт отдельной компиляции (§91). Не каждый borrowed вариант допустим: borrowed вход нельзя сохранить как non-Copy owned объект или вернуть дольше lifetime цели.

View не может использоваться после завершения lifetime цели. Возврат view на локальный объект допустим только при доказанном обеспечении lifetime объекта, без скрытого clone или произвольного продления lifetime ресурса.

Views на `String` и элементы `Vec` подчиняются тому же правилу. Операция, инвалидирующая используемый позднее view, отклоняется; compiler не создаёт скрытую копию и не сохраняет старый буфер для обхода конфликта.

---

# 23. `let` / `var`: права доступа и уникальность

Ownership и доступ через binding проверяются отдельно. `let` предоставляет readonly-доступ через текущую переменную, `var` — mutable-доступ при доказанной уникальности. Оба могут обозначать owned значение или view. Readonly binding не замораживает owned объект навсегда.

| Форма | Binding | Доступ через binding | Ownership |
| --- | --- | --- | --- |
| `let a = value` | Без переприсваивания | Readonly | Owned или view |
| `var a = value` | Переприсваиваемый | Mutable при допустимом источнике | Owned или mutable view |
| Параметр `a: T` | Без переприсваивания | Readonly; owned значение можно передать дальше | Owned или readonly borrowed entry |
| Параметр `var a: T` | Переприсваиваемый | Mutable | Owned или unique mutable borrowed entry |

Non-Copy owned значение можно передать из readonly binding в новый mutable binding или `var` параметр. Для этого compiler доказывает допустимый move, отсутствие последующих использований прежнего binding и отсутствие заимствований, конфликтующих с получаемым mutable-доступом. Передача ownership не копирует объект и сохраняет identity. Readonly borrowed view нельзя превратить в mutable view или owner.

```nether
let original = new User()
var editable = original  // move: ownership и проверенный mutable-доступ
editable.set_name("Alex")
```

Наличие одного owning root само по себе не доказывает уникальность доступа: учитываются aliases через поля, контейнеры, результаты и async state. Mutable-доступ к owned объекту не повышает права на внешние borrowed объекты, на которые указывают его поля. Если отсутствие конфликта нельзя доказать, повышение доступа отклоняется.

```nether
var owner = new User()
let view = owner
println("{}", owner.name)  // owner используется позже: view является borrowed
var invalid = view        // compile error: readonly view не имеет ownership
```

Для non-Copy owned аргумента функция может стать consuming-only:

```nether
fn consume_and_rename(user: User): User {
    var local = user
    local.set_name("Alex")
    return local
}
```

У этой функции доступен owned entry. Readonly borrowed entry недоступен, потому что `var local = user` требует ownership. Прямая запись `user.name = ...` запрещена самим readonly binding в обоих entries. Обычный параметр `user: User` не обещает отсутствие последующей мутации после передачи владения; consuming effects являются частью выведенного API (§95).

Для изменения объекта, остающегося у вызывающего кода, используется mutable параметр:

```nether
fn normalize(var user: User): () {
    user.name = ""
}

var user = new User()
normalize(user)
println("{}", user.name)  // mutable borrowed entry
```

Call не имеет отдельного prefix аргумента. `var` в параметре задаёт требование, которое compiler удовлетворяет mutable view, допустимым owned move или независимым fresh/Copy значением. Readonly owned источник может удовлетворить его через consuming move; readonly view не может.

Readonly-заимствование временно запрещает конфликтующую мутацию исходному владельцу до последнего зависимого использования, включая сохранённые/возвращённые views. После завершения заимствования mutable владелец снова может изменять объект. Lifetime view не становится ownership и не выдаёт права на повышение доступа после своего окончания.

`var` не является постоянно активной блокировкой всего graph. Конкретная запись, mutable вызов или mutable view требует unique access к затронутой памяти. Readonly-доступ к полю/элементу даёт readonly view; если же consuming extraction действительно передаёт owned поле или элемент, новый владелец может получить mutable-доступ по тем же проверкам (§24, §88, §93).

Readonly не означает чистоту функции: допустимы изменения отдельных `var` аргументов, собственных locals и полученных во владение значений после их допустимого переноса в mutable binding. Для borrowed входов и receivers права остаются ограниченными их контрактами.

Compiler проверяет ограничения статически. Backend может считать данные неизменяемыми на доказанном участке readonly-доступа; допустимый переход к mutable ownership завершает такую гарантию. Runtime refcount или проверка каждого доступа не вводятся.

---

# 24. Объявления переменных, move и возврат mutable-доступа

```nether
let fixed = 10
var count = 0
count = count + 1
fixed = 20  // compile error: let binding нельзя переприсвоить
```

`var` заменяет прежнее написание `let mut`. Отдельное keyword `mut` больше не используется для declarations, parameters, patterns или call arguments. `var` допускает переприсваивание binding и mutable-доступ; `let` предоставляет readonly-доступ через этот binding.

```nether
let first = new User()
first.set_name("Alex")  // compile error: receiver readonly

var editable = first   // допустимый move при отсутствии конфликтующих views
editable.set_name("Alex")
editable = new User()  // замена owned значения с cleanup прежнего
```

Для `var destination = source` применяются следующие условия:

| Источник | Получение mutable-доступа |
| --- | --- |
| Свежий owned literal, `new` или owned результат | Допустимо с проверкой lifetime, aliases и ограничений типа |
| Non-Copy owned значение через `let` или обычный параметр | Допустимо через move при отсутствии будущего use и конфликтующих заимствований |
| Mutable view | Допустимое перезаимствование в пределах исходных прав и lifetime |
| Readonly non-Copy view | Compile error: нет ownership для повышения прав |
| Copy значение | Создаётся самостоятельная owned копия, с сохранением ограничений borrowed компонентов |

Если источник используется позже, правило автоматического view не превращает readonly доступ в mutable. Например, `var editable = original` при последующем использовании readonly `original` не может выполнить move и для non-Copy значения отклоняется. `let alias = mutable_owner` создаёт readonly binding/view; mutable право исходного владельца не исчезает, если ownership не был передан.

Повышение доступа относится к действительно принадлежащим получателю данным. Borrowed поля сохраняют права и lifetime внешних целей. Собственный mutable контейнер может хранить readonly views: возможность изменять его slots не даёт права менять объекты за этими views. Ограничения самого типа также сохраняются: `var text: str` позволяет заменять descriptor, но не записывать UTF-8 данные через `str`.

Copy создаёт самостоятельные owned данные. `var copy = readonly_copy_value` не изменяет исходные owned данные; копирование descriptor readonly view не создаёт mutable доступ к его цели. Instance receiver не копируется автоматически ради вызова mutable метода (§34).

Переприсваивание borrowed `var` binding меняет только его локальную привязку. Запись в поле через mutable view изменяет исходный объект. При замене view старая цель не уничтожается; замена owned значения выполняет cleanup по §93.

Owned результат функции можно принять как `let` или `var`, независимо от readonly bindings/параметров, через которые он был передан, при проверке оставшихся aliases и ограничений компонентов. Borrowed результат сохраняет readonly/mutable capability и lifetime источника. Возвращаемый тип `T` не стирает ownership и эти зависимости; смешанные ветви не получают owned или mutable contract, если его нельзя доказать для всех соответствующих путей.

```nether
fn validate_user(user: User): User {
    validate(user)
    return user
}

var user = validate_user(new User())
user.set_name("Alex")
```

Здесь owned entry возвращает владельца. В borrowed entry такой же `return user` возвращал бы readonly view. Необратимое замораживание не является эффектом `let` и отдельной конструкцией первой версии не вводится.

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

Функцию с параметром `var user: User` нельзя присвоить переменной типа `(User) => ()`, скрыв mutable parameter mode. Обычный `user: User` предоставляет readonly-доступ через свой binding, но owned entry может передать владение mutable local. Поэтому `(User) => ()` не обещает отсутствие consuming effects или наличие borrowed entry. Выбранный entry, ownership/result и lifetime-контракты сохраняются вместе с callable metadata и проверяются при обычном `operation(user)`, без обязательных пользовательских generic annotations. Для unbound method pointer receiver всегда borrowed (§80).

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

# 29. Non-capturing lambdas

Разрешено:

```nether
let double: (i32) => i32 = (x) => x * 2
```

Это синтаксический sugar для generated static function.

Параметры non-capturing lambda используют те же access modes: `(value) => ...` задаёт readonly binding, `(var value) => ...` — mutable binding по контракту функции. Owned значение обычного параметра можно перенести в mutable local по §24; readonly borrowed вход так не преобразуется. Ожидаемый function type может задавать тип аргумента, но не подставляет отсутствующий `var` и не отменяет consuming requirements.

Compiler может преобразовать его в:

```nether
fn __generated_double(x: i32): i32 {
    return x * 2
}
```

Никакого environment object не создаётся.

---

# 30. Capturing closures запрещены

Недопустимо:

```nether
let offset = 10

let add: (i32) => i32 = (x) => x + offset
```

Ошибка:

```text
error: function expression captures local variable 'offset'

capturing closures are not supported
```

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

Instance call не подставляет автоматический Copy или consuming move receiver ради доступа к mutable методу. Через существующий readonly binding нельзя вызвать `var this` метод даже при owned или Copy значении. Сначала создаётся `var editable = value`: это проверенный move для owned non-Copy либо самостоятельный Copy. Метод на mutable Copy receiver изменяет этот receiver, а не неявную временную копию. Свежий receiver expression может получить требуемое методом право с обычной проверкой lifetime/aliases.

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

Consuming операция над экземпляром целиком записывается как free function или static method с явным value parameter, например `fn transform(var value: User): User`. Остальные параметры instance метода могут иметь owned/borrowed entries по общим правилам. Mutable receiver может извлечь принадлежащий контейнеру owned элемент или поле при сохранении корректного состояния владельца (§93); это не consuming move самого receiver.

Borrowed результат метода сохраняет lifetime receiver/других входов. Owned результат может быть fresh или извлечённым owned значением, но возврат non-Copy `this` даёт только view. Нельзя вернуть view на уничтожаемый temporary receiver; compiler проверяет время жизни соответствующего владельца (§22).

---

# 35. Inheritance

Только class поддерживает concrete inheritance.

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

Runtime polymorphism должен быть явным для interfaces и class inheritance:

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

# 53. Regions

Компилятор автоматически группирует class allocations с совместимыми lifetime и контрактами destruction. Группировка является реализацией логического ownership; она не разрешает ссылки на уже уничтоженные объекты.

Независимо удаляемые элементы не обязаны жить до завершения общего контекста приложения или контейнера. Связанные объекты с общим lifetime могут принадлежать отдельному региону группы (§93). Физический allocator или pool может быть общим для нескольких логических owning contexts.

```nether
fn handle_request() {
    let user = new User()
    let order = new Order()
    let response = new Response()
}
```

может использовать:

```text
RequestRegion
├── User
├── Order
└── Response
```

---

# 54. Region allocation

Вместо нескольких:

```text
malloc
malloc
malloc
```

возможен:

```text
region_ptr += aligned_size
```

А в конце:

```text
region.reset()
```

---

# 55. Region inference

Compiler определяет regions:

```text
ApplicationRegion
WorkerRegion
TaskRegion
RequestRegion
FunctionRegion
LoopRegion
TemporaryRegion
```

Они не обязаны существовать в пользовательском синтаксисе. Для owned результатов и consuming calls compiler учитывает контекст назначения ещё при выборе места выделения (§21). Lifetime и наблюдаемое destruction должны следовать семантическому контракту, а не произвольному решению оптимизатора.

Для долгоживущих контейнеров compiler выделяет самостоятельные логические owning contexts owned элементов. Связанные объекты с общим lifetime могут объединяться в регион группы. Самостоятельное владение и правила освобождения описаны в §93; пользователь не объявляет эти контексты вручную.

---

# 56. Loop region

```nether
for item in items {
    let temp = new Buffer()

    process(temp)
}
```

Если `temp` не escape:

```text
allocate
process
reset iteration region
```

Memory может переиспользоваться каждой итерацией.

---

# 57. Ленивые async operations

```nether
async fn load_user(id: i64): User {
    let record = await database.find_user(id)
    return new User(record.name)
}

let user = await load_user(10)
```

`User` в объявлении является типом результата `await`. Вызов `load_user(10)` создаёт ленивую типизированную async operation; тело не начинает выполняться до `await` или запуска через `spawn`.

Compiler преобразует операцию в state machine. Обычный `await` не требует создания отдельной scheduler task и обязательной heap allocation: состояние может размещаться inline в вызывающей state machine или в подходящем регионе.

Передача и хранение аргументов в async state учитываются lifetime-контрактом уже при создании операции. Выходной owned объект передаётся получателю через автоматический move (§21).

Пользователю не требуется писать конкретный внутренний future/state-machine type.

---

# 58. Async state

Compiler сохраняет только данные, необходимые после возобновления операции. Это включает используемые после `await` значения и состояние ресурсов, требующее destruction.

```text
LoadUserState
├── continuation state
├── live parameters / locals / views
├── resource cleanup state
└── task / operation region, если необходим
```

Операция и spawned task привязаны к текущему потоку и автоматически не мигрируют. Поэтому обычные class views могут использоваться в задачах одного потока при соблюдении lifetime и unique-access правил.

Отмена является кооперативной и наблюдается на `await`, включая явную передачу управления scheduler:

```nether
await yield_now()
```

Compiler не добавляет обязательную проверку отмены в каждую итерацию обычного вычислительного цикла. Длительный CPU-bound код должен периодически выполнять `await yield_now()`, если ему нужны отзывчивая отмена и совместное выполнение с другими задачами.

Пока код задачи выполняется или её дети ещё используют state, память не освобождается. После остановки операции и завершения необходимых child barriers выполняется синхронный cleanup. Отмена ленивой операции до первого запуска уничтожает захваченные owned аргументы, не исполняя тело. Compiler не копирует class objects для переноса async state между потоками.

Отмена не гарантирует ограниченного времени завершения: некооперативный код, блокирующий foreign call или незавершающийся child может задержать scope. Цена механизма — состояние отмены и взаимодействие со scheduler в точках приостановки; обычный синхронный код не получает фонового runtime контроля.

---

# 59. Await boundary

Активный уникальный mutable view не может пересекать `await`. Объявление `var` само по себе не означает наличие такого view: owned mutable значение и borrowed mutable доступ проверяются отдельно.

```nether
async fn process(var user: User): () {
    user.set_loading()
    await network.send()
    user.set_complete()
}
```

Если `user` является borrowed mutable параметром и его доступ должен сохраниться через приостановку, borrowed entry недоступен. Последующее обращение после `await` не может заново получить право из readonly alias. При owned entry объект может храниться в async state, если нет пересекающего `await` mutable loan и соблюдены ограничения остальных aliases.

Readonly views могут пересекать `await` при доказанном lifetime цели и отсутствии конфликтующих действий задач. Захват параметров ленивой операции учитывает эти права уже при создании operation (§57).

---

# 60. Mutable owned state между await

Owned локальный объект может сохранять право изменения между приостановками, когда каждый отдельный unique access завершается до `await`:

```nether
async fn process(): () {
    var user = new User()
    user.set_loading()

    await network.send()

    user.set_complete()
}
```

```text
owned mutable state
→ temporary unique access
→ end access
→ await
→ new temporary unique access
```

Здесь ownership остаётся в async state. Если метод вернул mutable view и он используется после приостановки, доступ не завершён и `await` запрещён. Owned значение обычного параметра `user: User` также можно перенести в mutable local по §24; такой body не получает readonly borrowed entry. Readonly view не восстанавливает mutable capability. Для mutable borrowed входа, включая `var this` async метода, применяется §59.

---

# 61. Structured async

```nether
async fn handle(user: User): () {
    task_scope {
        spawn load_profile(user)
        spawn load_settings(user)
    }
}
```

`spawn` запускает child task на текущем потоке и допускается внутри `task_scope`. Обычный `await` и `spawn` различаются: первый выполняет ленивую операцию как часть вызывающей операции, второй создаёт конкурентную задачу и требует работы scheduler.

`task_scope` не заканчивается до завершения всех child tasks. Lifetime родительских объектов покрывает все разрешённые child views. Совместное чтение допустимо; конфликтующий mutable access запрещается даже при выполнении на одном потоке.

При обычном достижении конца body scope ждёт нормального завершения детей. Ранний выход через `return`, выходящий за scope `break`/`continue` или propagation через `?`, а также отмена и panic запрашивают кооперативную отмену оставшихся детей. Родитель не завершает scope и не уничтожает доступные детям ресурсы до прекращения их выполнения и cleanup.

Panic из body или child сохраняется как первая наблюдаемая panic scope. Scheduler/state machine выполняет cancellation/join barrier; после прекращения доступа всех детей родитель продолжает unwinding (§92). Это часть async control flow, а не `await` внутри destructor. Обычные ошибки `Result.Err` не отменяют соседние задачи: отмена возникает при фактическом раннем выходе из scope, например через `?`.

Несколько независимо возникших child panics сами по себе не означают double panic: первая остаётся основной, остальные могут быть сохранены как диагностика. Panic непосредственно во время уже идущего unwinding/cleanup имеет отдельное правило abort (§92). Scope с некооперативным child может ждать неограниченно долго; освобождение памяти до его остановки не допускается.

---

# 62. Detached tasks

Первая версия использует `spawn` внутри `task_scope`. Detached запуск не является неявным эффектом `spawn`.

Если будет добавлен отдельный detached API, его задача не сможет сохранять прямой view на контекст, который она переживает. Owned аргументы должны иметь самостоятельный lifetime; конкретный detached API пока не утверждён.

---

# 63. Multithreading

Threads не делят обычные class objects. Async task не означает новый поток. Для выполнения на другом потоке используется явная граница worker/channel; обычные задачи автоматически не мигрируют.

Модель:

```text
Thread A
   ↓
Channel
   ↓
Thread B
```

---

# 64. Channel

Базовый channel — bounded SPSC queue: один sender и один receiver, с явно заданной положительной capacity.

```nether
var (tx, rx) = Channel.bounded<Job>(256)
```

API:

```text
tx.send(value: T)       -> async Result<(), SendError<T>>
tx.try_send(value: T)   -> Result<(), TrySendError<T>>
tx.reserve()           -> async Result<SendPermit<T>, ChannelClosed>
rx.receive()           -> async Option<T>
tx.close()             -> ()
rx.close()             -> ()
```

`SendError<T>` содержит исходное owned сообщение. `TrySendError<T>` имеет variants `Full(T)` и `Closed(T)`. `try_send` не приостанавливает выполнение. `send` при заполненной очереди ждёт свободное место: это backpressure, а не неограниченное увеличение буфера.

Payload-параметр `value: T` предоставляет readonly binding, но отправка передаёт ownership сообщения в operation/queue. `receive`, `SendError` и `TrySendError` передают принадлежащий им payload новому владельцу, который может получить mutable-доступ через `var` после consuming extraction и проверок §24, §88. Readonly binding отправителя не замораживает сообщение навсегда. Transferability, ограничения borrowed компонентов и единственность ownership сохраняются; сам `transfer` не превращает readonly borrowed view во владельца.

```nether
await tx.send(job)?
let next: Option<Job> = await rx.receive()
```

Закрытие или destruction sender прекращает новые отправки; receiver сначала извлекает накопленные сообщения, затем получает `Option.None`. Закрытие или destruction receiver прекращает новые отправки и уничтожает непрочитанные owned сообщения. Ожидающие операции пробуждаются: send без commit возвращает `SendError` с сообщением, receive закрытой пустой очереди — `None`. Close идемпотентен.

Endpoints являются unique non-Copy capabilities. Они могут передаваться через move, но не клонируются автоматически. Их state-changing operations требуют mutable receiver, поэтому endpoints объявлены через `var`. Один endpoint допускает только одну незавершённую операцию; живой `SendPermit` также занимает sender capability. До завершения операции/permit endpoint нельзя перемещать или использовать конфликтующим образом. Async операции channel используют специальную библиотечную operation capability; она не предоставляет unique mutable view на произвольные пользовательские данные через `await` (§59).

Queue имеет одного владельца в окружающем owning scope, покрывающем endpoints и подключённые workers. Compiler проверяет их lifetime и завершение workers до освобождения queue. Передаваемый endpoint является специальной межпоточной capability стандартной библиотеки; он не разрешает обычным class views пересекать thread boundary. Внутренняя синхронизация channel не вводит shared ownership через ARC.

Буфер queue, atomics и scheduler wakeups являются стоимостью channel. Inline сообщения не требуют отдельной heap allocation или deep clone на каждую отправку. Multi-producer/multi-consumer channel не является неявным режимом этого API: ему нужен отдельный явный API и контракт синхронизации.

Пример bounded queue с backpressure и обработкой закрытия: [Tokio bounded channels](https://docs.rs/tokio/latest/tokio/sync/mpsc/). Ограничение SPSC и ownership endpoints являются выбором Nether.

---

# 65. Thread messages

Предпочтительно использовать struct:

```nether
struct Job {
    id: i64
    kind: JobKind
}
```

Такое сообщение может храниться непосредственно в channel storage без отдельной class allocation. Inline layout сам по себе не гарантирует допустимость межпоточной передачи.

Проверка transferability рекурсивно учитывает поля struct, tuple, enum и элементов контейнеров. Struct с обычной class reference или view на память отправителя не становится transferable только потому, что это struct. Для owned буферов требуется разрешённый transfer; обычное сообщение не получает скрытый deep clone или автоматическую сериализацию для обхода lifetime-конфликта.

---

# 66. Cross-thread class references

Запрещено:

```nether
let user = new User()

worker.send(user)
```

если `User` является обычным class.

---

# 67. Transferable memory

Специальный transferable Buffer может передавать ownership между memory domains:

```nether
await tx.send(transfer buffer)?
```

Создание consuming send operation забирает payload уже при capture аргумента, даже если операция ленивая. Источник больше нельзя использовать. Compiler проверяет все aliases до transfer; неустранимый конфликт является compile error. Transfer относится к специальным transferable buffers, а не разрешает передачу произвольного class graph.

Отправка имеет единственную commit boundary — успешное помещение сообщения в queue. До commit сообщение принадлежит send operation; после commit — queue, затем получателю. Одновременное наличие двух владельцев не допускается. Успешный send возвращает `Result.Ok(())`; закрытие до commit возвращает сообщение через `SendError`, не восстанавливая прежний binding автоматически.

Отмена до commit уничтожает принадлежащий операции payload. После commit отмена отправителя не отзывает сообщение из queue. Закрытие receiver после commit может уничтожить ещё непрочитанное сообщение по §64; успешная отправка не гарантирует обработки получателем.

Чтобы ожидание свободного места не потребляло buffer заранее, используется reservation:

```nether
var permit = await tx.reserve()?
SendPermit.send(permit, transfer buffer)?
```

`reserve` не забирает buffer. `SendPermit.send(var permit: SendPermit<T>, value: T)` — static method без receiver с consuming-only контрактом обоих non-Copy аргументов. Он потребляет permit и сообщение, выполняется синхронно без `await` и возвращает `Result<(), SendError<T>>`. Такая форма сохраняет правило, что instance receiver всегда borrowed (§34). Свободное место уже зарезервировано, но при закрытии receiver до commit сообщение возвращается в ошибке. Drop/cancellation неиспользованного permit освобождает reserved slot. После получения permit до синхронного send нет точки кооперативной отмены, если пользователь сам её не добавит.

Reservation сохраняет buffer до принятия решения об отправке; при выходе из его owning scope buffer всё равно уничтожается по обычным правилам. Она не возвращает ранее consuming payload в исходную переменную.

Это специальная граница memory domains, а не обычный class assignment.

Аналог reservation API: [Tokio Sender.reserve](https://docs.rs/tokio/latest/tokio/sync/mpsc/struct.Sender.html#method.reserve). Commit, cancellation и ownership payload выше являются обязательным контрактом Nether.

---

# 68. Destruction

Обычный пользовательский код не содержит:

```text
delete
free
destroy
```

Lifetime выводится компилятором.

Завершение owned lifetime, включая удаление или замену owned элемента контейнера, запускает destruction без GC, ARC и поиска последней ссылки. Используемые views запрещают такое завершение статически; существующие weak fields не удерживают объект (§93).

---

# 69. Destructors

Для внешних ресурсов:

```nether
class File {
    destructor {
        os.close(handle)
    }
}
```

Destructor вызывается deterministic в соответствии с lifetime-контрактом и до освобождения памяти объекта. Физическая группировка allocations не должна произвольно менять наблюдаемый момент закрытия ресурса.

Destructor является специальным lifecycle body с implicit exclusive `this`, без явного receiver parameter. Cleanup выполняется и для readonly owned объекта: завершение ownership разрешает уничтожение и необходимое изменение его собственного завершаемого состояния после завершения зависимых views. Это не позволяет обычному методу изменить readonly объект и не повышает права на внешние borrowed цели. Автоматический cleanup не требует от пользователя объявлять каждый resource через `var`.

Последнее использование view завершает заимствование, но само по себе не запускает destructor владельца. Не перенесённые owned locals живут до выхода из owning scope. Move передаёт ответственность новому владельцу; замена/удаление owned значения и несохранённый owned результат завершают соответствующий lifetime (§93). Compiler может сократить физическое storage, только сохранив наблюдаемые cleanup effects.

Порядок cleanup:

- owned roots одного scope уничтожаются в обратном порядке успешной инициализации;
- для объекта сначала выполняется body его пользовательского destructor, затем owned поля в порядке объявления; borrowed/weak поля не уничтожают цели;
- для class сначала завершается cleanup производной части, затем base части с её destructor и owned полями;
- array/tuple elements уничтожаются в порядке индексов, enum — owned payload активного variant; контейнер фиксирует порядок в своём API, для `Vec` это текущий порядок элементов;
- при panic во время construction уничтожаются только успешно инициализированные owned части; destructor незавершённого объекта не вызывается;
- оставшиеся объекты региона группы уничтожаются в обратном порядке успешной инициализации внутри группы. Ownership transfer внутрь группы добавляет объект в её cleanup order в момент принятия ownership.

Тот же порядок применяется при normal exit, cancellation и unwinding. Cleanup state учитывает уже moved/уничтоженные части, чтобы исключить повторное уничтожение. При panic body destructor cleanup оставшихся owned полей продолжается в рамках unwinding, пока не возникла повторная panic (§92).

Удаление или замена owned элемента без передачи владения запускает его destructor при завершении операции, до повторного использования памяти. Если операция возвращает owned результат, destruction относится к lifetime нового владельца. Уничтожение региона группы выполняет cleanup всех оставшихся живых owned объектов группы; уже уничтоженные объекты повторно не уничтожаются (§93).

Destructor синхронен: он не может содержать `await`, запускать работу, переживающую cleanup, или возвращать `Result`. Закрытие, которое требует ожидания либо обработки ошибки, выполняется явной операцией, например `finish(): Result<(), Error>` или async `finish`. Destructor остаётся синхронной fallback cleanup и учитывает уже закрытое состояние ресурса.

Destructor не может обращаться к уже уничтоженному объекту через view или weak lookup. Compiler проверяет destructor access effects вместе с cleanup order. Обычный цикл graph views допустим; цикл зависимостей destructors, требующий живого доступа в обе стороны, отвергается статически. Если безопасный порядок нельзя доказать, нужен явный `finish` при ещё живой группе, снимающий такие зависимости до автоматического cleanup. Compiler не добавляет GC или runtime поиск подходящего порядка.

Цена ресурсов — необходимый cleanup code/state и статические ограничения зависимостей. Значения без наблюдаемого cleanup не требуют вызова пустого destructor. Сравнение deterministic cleanup: [Rust Reference: destructors](https://doc.rust-lang.org/reference/destructors.html); конкретный порядок группы закреплён выше для Nether.

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

Declaration:

```nether
fn add(a: i32, b: i32): i32 {
    return a + b
}
```

Function type:

```text
(i32, i32) => i32
```

Function reference:

```nether
let operation: (i32, i32) => i32 = add
```

Non-capturing function expression:

```nether
let add: (i32, i32) => i32 = (a, b) => a + b
```

Capturing closure:

```nether
let offset = 10

let add = (x) => x + offset
```

запрещена.

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

Параметры без access modifier в AST по умолчанию имеют readonly binding; owned entry может переносить их значения в mutable locals. Receiver method AST хранит отдельный первый параметр с типом содержащего типа, mode `readonly`/`mutable` и обязательным borrowed ownership; отсутствие receiver обозначает static method. Return contract различает owned результат и borrowed capability/lifetime, сохраняя ограничения компонентов.

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

# 81. Основные memory invariants

### Invariant 1

Каждый class object имеет одну identity и один текущий owning context. Owned передача меняет логическое владение, сохраняя identity.

### Invariant 2

`new` создаёт новую class identity.

### Invariant 3

Value assignment создаёт owned копию для Copy, выполняет допустимый move для non-Copy owned источника без последующих использований или создаёт view при borrowed доступе. Выбор не выполняется через runtime refcount.

### Invariant 4

Реальные aliases являются views и не могут использоваться после уничтожения цели. Copy class создаёт новую identity, а move сохраняет identity и требует проверки сохраняющихся aliases.

### Invariant 5

`let` и обычный параметр предоставляют readonly-доступ через binding. Owned значение можно перенести в mutable binding при допустимом move и доказанном отсутствии конфликтующих заимствований. Readonly borrowed view не получает mutable права. Ограничения внешних borrowed компонентов сохраняются; mutable операция требует unique access.

### Invariant 6

Активный unique mutable view не переживает `await`. Owned `var` state может сохраняться между приостановками без такого view (§59–60).

### Invariant 7

Struct является inline value.

### Invariant 8

Struct не имеет concrete inheritance.

### Invariant 9

Class имеет максимум одного class parent.

### Invariant 10

Runtime polymorphism для interfaces и class inheritance требует `dyn`. Статически доказанный конкретный тип использует прямую специализацию.

### Invariant 11

Capturing closures отсутствуют.

### Invariant 12

Обычные class references не пересекают thread boundary.

### Invariant 13

Runtime не использует hidden GC.

### Invariant 14

Runtime не использует hidden ARC.

### Invariant 15

Owned элемент контейнера имеет самостоятельное владение. Удаление или замена завершает его lifetime, если владение не передаётся результату; lifetime контейнера сам по себе не удерживает удалённый объект.

### Invariant 16

Уничтожение цели запрещено, пока используется зависящий от неё view. Существование weak field не запрещает уничтожение; полученный из weak обычный view подчиняется той же статической проверке.

### Invariant 17

Связи графа могут быть циклическими, но отношение владения не образует циклов. Группа с общим lifetime освобождается целиком без анализа достижимости её объектов.

### Invariant 18

Завершение последнего использования view не завершает owning lifetime. Наблюдаемый cleanup следует owning scope, consuming transfer и правилам удаления/замены (§69).

### Invariant 19

Parent scope не уничтожает память, к которой ещё могут обращаться child tasks/workers. Отмена является запросом остановки, а не разрешением немедленно освободить state (§58–61, §64).

### Invariant 20

Сообщение имеет одного владельца на каждой стадии отправки: operation, queue или получатель. Commit передаёт ownership; failure/cancellation не создают второго владельца (§67).

### Invariant 21

Foreign ABI не выводит safety из неизвестного тела. Raw interop требует явного unsafe контракта, а plain C boundary не допускает unwind (§99).

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

# 84. Основная философия

Пользователь должен видеть достаточно простой код:

```nether
let user = new User("Alex")

let values: Vec<i32> = [1, 2, 3]

let fixed: {f64; 3} = {1.0, 2.0, 3.0}

let predicate: (i32) => bool = (value) => value > 0

process(user)
```

При этом compiler знает:

```text
User
→ class identity
→ owner context known
→ region allocation

values
→ dynamic contiguous storage

fixed
→ inline 3 × f64

predicate
→ static function
→ no captured environment

process(user)
→ view inferred
```

Конечная модель языка:

```text
Rust-like primitive types
+
C#-like classes/interfaces
+
inline structs
+
explicit new
+
automatic ownership/view inference
+
explicit let/var access modes + unique mutable access
+
Rust-style enum/match
+
TS-style function types
+
no closures
+
region allocation
+
structured async
+
channel-only threading
+
AOT static specialization
```

Главная идеология языка:

> Бесплатная простота при максимальной скорости.

Пользователь явно выбирает readonly/mutable-доступ через `let`/`var` и receiver `this`/`var this`; compiler выводит ownership, views, move, lifetime и необходимые специализации, сохраняя объявленные права. Выбранная абстракция не должна добавлять обязательные копирования, аллокации или runtime checks сверх необходимых для её операции. Атрибут Copy явно выбирает стоимость создания самостоятельного значения; compiler может устранять ненаблюдаемые копии, сохраняя семантику.

Основная сложность переносится в статический анализ и compilation. Если безопасную реализацию нельзя доказать, compiler объясняет конкретный конфликт; он не скрывает проблему через clone, GC или ARC.

Статически доказанный доступ не требует runtime lifetime-check. Динамические операции имеют собственную стоимость: `dyn` может требовать dispatch, `spawn` — scheduler, а `weak` — проверку действительности. Простота синтаксиса не означает отсутствие стоимости этих явно выбранных операций.

Максимальная скорость является целью проектирования, а не обещанием оптимального машинного кода для каждой программы.

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

Принятие таблицы не вводит автоматически отсутствующий в Nether синтаксис raw pointers, borrow operators или Rust closures. Prefix `await` разбирается как unary form Nether; его операнд включает call/index/member access. Например, `await load_user(id)` ожидает результат вызова. `var` является модификатором declaration/parameter/pattern/receiver, а не unary expression и не prefix call argument. `this` является receiver expression внутри instance method; формы `this` / `var this` в parameter list разбираются по §34.

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

# 88. Pattern matching и права bindings

`match` поддерживает wildcard `_`, destructuring, rest `..` и guards. Все возможные случаи должны быть покрыты; типы результатов expression branches согласованы. Guards выполняют readonly inspection, не потребляют payload и не получают mutable-доступ к проверяемым данным.

```nether
match result {
    Result.Ok(value) if is_valid(value) => consume(value),
    Result.Ok(_) => handle_invalid(),
    Result.Err(error) => handle_error(error),
}
```

Identifier pattern имеет форму `[ref] [var] name`:

| Pattern | Значение |
| --- | --- |
| `name` | Readonly binding с автоматически выведенным move/view |
| `var name` | Mutable binding через допустимый owned move или mutable view |
| `ref name` | Явный readonly view на payload |
| `ref var name` | Явный unique mutable view на payload |

`var` в pattern имеет ту же access semantics, что и в declaration. Consuming pattern может перенести owned payload из readonly binding в mutable binding при проверках §24. Readonly borrowed payload нельзя повысить до mutable или превратить во владельца. `ref var` не выполняет move: ему необходимы mutable-доступное место и доказанная уникальность; для readonly owned источника сначала требуется отдельная передача в `var`. Scope bindings ограничен конструкцией/ветвью; consuming/mutable bindings начинают действовать после принятия guard.

```nether
var point = (1, 2)
let (var x, y) = point
x = x + 1

var result = load_result()
match result {
    Result.Ok(ref var value) => {
        normalize(value)
    },
    Result.Err(ref error) => report(error),
}
```

В `let pattern = value` немаркированные bindings readonly, отдельные `var` patterns задают mutable binding. В `var pattern = value` немаркированные bindings получают mutable-доступ при допустимом источнике: `var (x, y) = (1, 2)`. Явный `ref name` остаётся readonly в обеих формах. В match/if let/while let patterns без `var` создают readonly bindings. Field forms: `Point { var x, y }` и `Point { x: ref var current_x, y }`.

Для function parameter destructuring внешний `var` задаёт mutable контракт аргумента, например `fn change(var (x, y): (i32, i32)): ()`. Внутренний pattern не расширяет права readonly borrowed параметра. Consuming extraction owned частей в mutable bindings допустима в owned entry; если она требует ownership, readonly borrowed entry недоступен. Named parameter `var user: User` следует §23.

Payload extraction сохраняет особое правило Nether: без автоматического Copy/clone. Compiler выбирает move или view и проверяет access mode pattern. Owned move может передать payload в mutable binding, но readonly borrowed extraction даже для Copy типа не подставляет копию ради повышения прав; независимая mutable копия создаётся последующим обычным присваиванием `var copy = value` по §94.

Можно сочетать move и ref bindings только при доказанном отсутствии конфликта полей. Частично moved объект нельзя использовать целиком; оставшиеся поля сохраняют свои права. Для типов с destructor нельзя извлекать owned поле, оставляя destructor некорректно инициализированный объект. `var` не отменяет эти ограничения.

Destructuring, explicit ref и guards сохраняют Rust-подобную структуру; `let`/`var` access modes и автоматический move/view являются правилами Nether.

---

# 89. `weak` только внутри class

`weak` разрешён только как модификатор поля, непосредственно объявленного внутри class. Цель weak reference также должна быть class identity.

```nether
class Node {
    value: i32
    weak parent: Node

    constructor(initial_value: i32, initial_parent: Node) {
        value = initial_value
        parent = initial_parent
    }

    fn has_parent(this): bool {
        return match parent.get() {
            Option.Some(parent_node) => true,
            Option.None => false,
        }
    }
}
```

Поле `parent` хранит weak reference, не удерживающую цель во владении. `parent.get()` получает `Option` с обычным view на живую цель. Пока полученный view используется, цель не может быть уничтожена; тот же статический lifetime-контракт применяется к subsequent operations.

Нельзя объявить weak field в struct или enum, weak local variable, weak function parameter или weak generic container element. `weak` не превращает inline struct в ссылочный объект.

Рекурсивные class graphs внутри общего owning region могут использовать обычные views: цикл ссылок сам по себе не требует ARC или weak. Weak нужен для ссылки, lifetime хранения которой может превышать lifetime цели.

Weak field допускает независимое уничтожение цели и не требует сохранять её память до завершения lifetime содержащего поля. После destruction цели `get()` возвращает `Option.None`, в том числе после повторного использования её прежнего места другим объектом. Реализация различает identities через domain/slot/generation handle (§89.1). Конкретная разрядность handle и layout registry остаются деталями backend.

Успешный `get()` не создаёт нового владельца и не вводит скрытый retain/release. Он создаёт обычный view со статическим заимствованием владельца цели или консервативного множества возможных владельцев. Runtime проверка действительности handle сама по себе не заменяет этот lifetime-контракт. Последующие value assignments такого view следуют §94: Copy target может быть явно скопирован в самостоятельную identity, non-Copy target остаётся borrowed.

## 89.1. Стабильный handle

Выбранная модель weak reference — `(domain, slot, generation)` в registry потока. Domain не является owning region объекта: он хранит только служебные записи identities и действует до завершения потока. Обычные class objects и weak fields не передаются в другой поток (§63–67).

Для объекта, на который создан weak field, compiler/runtime регистрирует запись с адресом, generation и состоянием `alive`. Для объектов, не участвующих в weak references, registry entry не требуется. Handles относятся к identity, а не только к адресу памяти.

При начале destruction запись перестаёт быть `alive`, до исполнения пользовательского destructor. Повторное использование slot получает другую generation. При исчерпании generation slot выводится из повторного использования, чтобы старый handle никогда не совпал с новым объектом. Move owner обновляет owner provenance, сохраняя identity и handle; допустимое физическое перемещение обновляет адрес registry entry и запрещено при используемом view на старый адрес. Copy class получает новую identity и отдельную запись, если для копии создаётся weak field.

Registry slots остаются безопасно доступными для проверки до конца domain и могут переиспользоваться с новой generation; память уничтоженного объекта не удерживается weak fields. Освобождение owning region инвалидирует связанные живые записи, а не уничтожает registry из-под сохранившихся handles.

## 89.2. Проверка и статический lifetime

`get()` проверяет domain, slot, generation и состояние. Несовпадение или неживая цель возвращает `Option.None`; совпадение возвращает `Option.Some` с view. Проверяется handle, а не память уже уничтоженного объекта.

Compiler сохраняет provenance возможных owning roots/хранилищ через weak assignment и контракты функций. На время использования результата `get()` запрещены destruction, замена, move с несовместимым lifetime, инвалидирующее перемещение и конфликтующий mutable access к возможной цели.

Если конкретное хранилище доказано, ограничение относится к нему. Если происхождение динамической цели не удаётся сузить, заимствование консервативно относится ко всем возможным weak-addressable объектам domain. Любая операция, которая может уничтожить или инвалидировать их, включая неявный cleanup при выходе из scope и эффекты вызванных функций, должна пройти статическую проверку. Compiler не снимает это ограничение на основании одного успешного runtime lookup.

```nether
match observer.target.get() {
    Option.Some(target) => {
        println("{}", target.name)
        cache.remove(id)  // допустимо только после последнего доступа к target
    },
    Option.None => handle_missing(),
}
```

Если после удаления используется `target`, а cache может владеть этой целью, возникает compile error. В первой версии `weak.get()` возвращает readonly view: mutable доступ к class, содержащему weak field, не предоставляет mutable capability к внешней цели. Изменение цели требует отдельного mutable пути от её владельца с обычной проверкой aliases; lookup сам не повышает права. Readonly views через weak могут пересекать `await` только при доказанном отсутствии конфликтующих действий других задач; автоматической runtime pinning нет.

## 89.3. Стоимость и ограничения

Стоимость weak access — registry metadata, lookup и проверки slot/generation/state. Статическое заимствование не добавляет счётчиков retain/release или runtime locks. Метаданные registry могут сохранять high-water capacity потока; payload уничтоженных объектов не сохраняется.

При несуженном provenance ограничения domain могут запрещать удаление даже фактически независимого объекта. Это сознательная цена первой модели: простой `get()` и отсутствие ARC в обмен на консервативные compile errors. Оптимизация provenance может сужать ограничения, сохраняя ту же семантику.

Версионированные ключи как способ безопасного повторного использования slots описаны в [slotmap documentation](https://docs.rs/slotmap/latest/slotmap/). Статическое заимствование domain и запрет переполнения generation являются собственными правилами Nether.

`weak` не является обещанием бесплатного raw pointer access. Момент уничтожения определяется owned lifetime или регионом группы (§93), а не количеством weak references.

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

# 93. Долгоживущие структуры: владение элементами и регионы групп

Утверждённая модель сочетает самостоятельное владение owned элементами, статические views и регионы связанных групп. Cache, graph и изменяемые контейнеры не требуют GC, ARC или поиска последней ссылки. Пользователь не пишет отдельные ownership и region annotations.

## 93.1. Самостоятельное владение элементом

Новый объект, переданный в owned позицию контейнера, получает один логический owning context элемента. Контейнер управляет этим контекстом, но не объединяет lifetime всех независимо удаляемых элементов со своим lifetime.

Удаление или замена owned элемента без передачи владения завершает lifetime старого объекта. Выполняются его destructor и cleanup принадлежащих ему owned данных, включая owned buffers. Views на внешние объекты не уничтожают свои цели.

При consuming извлечении владение автоматически передаётся результату. Такой объект продолжает жить у нового владельца, несмотря на отсутствие в контейнере. Если owned результат операции не сохраняется и не передаётся дальше, он уничтожается при завершении инструкции. Здесь `remove` обозначает операцию cache с таким owned результатом; наличие ключа обрабатывается контрактом конкретного API.

Полученный owned элемент можно принять в `var` с проверками §24, даже если до помещения в контейнер он был доступен через readonly binding. Lookup, возвращающий readonly view, такого права не даёт. Сам `remove` изменяет контейнер через mutable borrowed receiver; ownership контейнера остаётся у caller, передаётся только элемент.

```nether
cache.remove(id)                 // owned результат не сохраняется: cleanup
let removed = cache.remove(id)   // owned результат получает новый lifetime
```

Удаление borrowed элемента уничтожает только место хранения view, а не внешний объект. Обычное alias assignment не превращается в shared ownership. Owned поле или root binding при замене следует тем же правилам, что owned элемент.

## 93.2. Views ограничивают уничтожение

View является статическим заимствованием, не удерживающим объект через runtime refcount. Compiler учитывает его последние использования, включая ссылки через поля, результаты функций и async state.

```nether
let user = cache.get(id)
println("{}", user.name)
cache.remove(id)  // допустимо: последнее использование view завершено
```

Здесь `get` обозначает borrowed lookup существующего элемента; обработка отсутствующего ключа опущена. Lifetime view может закончиться до конца лексического scope.

```nether
let user = cache.get(id)
cache.remove(id)
println("{}", user.name)  // compile error: view используется после destruction
```

Недопустимая операция не превращается автоматически в clone, отсроченное уничтожение или перенос старого объекта в ApplicationRegion. Для consuming извлечения сохраняющиеся views отдельно проверяются по правилам move и инвалидирования памяти контейнера (§21–22).

Если другой class или child task продолжает использовать view, уничтожение цели также запрещено. Async scope должен завершить зависимые обращения перед удалением объекта. Когда отсутствие конфликта нельзя доказать, compiler может консервативно запретить изменение всего соответствующего хранилища вместо runtime lifetime-check на обычных views.

## 93.3. Связанные группы и циклические графы

Объекты, которым требуется общий lifetime, могут объединяться compiler в отдельный owning region группы. У группы есть один owning root; её lifetime не обязан совпадать с lifetime приложения. Views внутри группы могут образовывать циклический graph.

Связи графа и владение памятью являются разными отношениями. Цикл views допустим; цикл owning contexts не допускается. Уничтожение root группы завершает lifetime оставшихся owned объектов группы без обхода ссылок для определения достижимости. Destructors и необходимый cleanup по-прежнему выполняются.

Внешние обычные views на группу должны завершиться до её уничтожения. Обычные views между независимо живущими группами или элементами допустимы только при доказанном lifetime цели. Ссылка на цель, которая может исчезнуть раньше содержащего её class, должна быть weak field (§89).

Удаление ребра или потеря достижимости отдельного узла внутри живой группы сама по себе не освобождает узел. Группа удерживает принадлежащие ей объекты до завершения общего lifetime. Если узлы должны удаляться независимо, каждый получает самостоятельного владельца в хранилище; связи учитывают статические lifetime или используют weak fields. Owned удаление узла хранилищем завершает его lifetime по §93.1.

Compiler не объединяет независимо удаляемые owned элементы в единую группу приложения ради обхода конфликтов views или упрощения освобождения памяти.

## 93.4. Destruction, allocator и стоимость

Самостоятельный логический контекст элемента не требует отдельного системного `malloc` или отдельной arena на каждый элемент. Compiler может использовать общий allocator, pool, reusable slots или подходящие регионы групп, сохраняя identity, lifetime и момент destruction.

После cleanup независимо уничтоженного элемента его место должно быть доступно для безопасного повторного использования или освобождения. Повторная замена элементов не должна удерживать все прежние версии только потому, что контейнер или ApplicationRegion продолжает жить. Перемещение буфера контейнера также не продлевает lifetime уничтоженных owned объектов.

Возврат места allocator не означает обязательного немедленного возврата страниц операционной системе: pool может сохранять свободную capacity. Удержание памяти живой группы, fragmentation и capacity отличаются от удержания всех уничтоженных версий элементов.

Runtime стоимость состоит из реально необходимых allocator operations, destructors и metadata выбранного хранилища. Обычные views не требуют счётчиков ссылок или проверок каждого доступа. Weak lookup может требовать metadata и проверки identity; её стоимость относится к явно выбранному weak access.

Цена статической модели — ограничения операций при сохраняющихся views и возможные консервативные compile errors. Полностью произвольный graph не получает автоматический сбор недостижимых узлов. Групповое освобождение и независимое owned удаление являются разными выбранными lifetime-моделями; compiler не скрывает одну за другой.

Конкретные allocator layouts и разрядность/layout weak handles остаются деталями реализации. Модель domain/slot/generation и статического заимствования weak цели закреплена в §89; порядок и ограничения взаимозависимых destructors — в §69. Модель владения и условия удаления, приведённые выше, утверждены.


---

# 94. Value assignment, move, view и `Copy`

Правило применяется к инициализации binding, переприсваиванию, value arguments, owned позициям полей/контейнеров и value return. Compiler анализирует право владения и будущие использования текущего значения источника по control-flow graph, а не только по следующей строке исходника.

Выбор ownership и проверка access mode выполняются совместно. Следующая таблица описывает владение; для mutable destination дополнительно проверяется допустимость owned move или передаваемого mutable view по §23–24:

| Источник | Результат value assignment |
| --- | --- |
| Свежий owned literal, `new` или owned результат вызова | Owned значение передаётся прямо получателю |
| Существующее значение типа с `Copy`, включая borrowed источник | Новое самостоятельное owned значение; источник остаётся доступен |
| Non-Copy owned источник, который допустимо передать и который больше не используется | Move; прежний источник становится недоступен до повторной инициализации |
| Non-Copy источник, который используется позже, или borrowed источник без права move | View, если контракт назначения допускает borrowed доступ |
| Non-Copy источник без допустимого move, когда назначению требуется owns | Compile error |

`let` destination и обычный параметр предоставляют readonly binding. `var` destination/параметр может получить fresh/Copy owned значение, mutable view или non-Copy owned значение через допустимый move, включая readonly owned источник. Для последнего compiler проверяет отсутствие конфликтующих заимствований. Readonly borrowed источник не становится mutable; если будущий use исключает move и mutable view получить нельзя, операция отклоняется.

Instance receiver всегда borrowed по §34: readonly `this` либо mutable `var this`. Последнее использование не превращает его в owned entry. Нет неявного Copy receiver или consuming move ради вызова mutable метода через readonly binding. Остальные value arguments следуют таблице выше.

Owned return допускает получение mutable-доступа новым владельцем по общим проверкам, в том числе после forwarded `identity(value: T)` (§95). Borrowed return сохраняет capability/lifetime. Contracts полей/контейнеров различают owned данные и внешние borrowed цели; ownership контейнера или `dyn` не повышает права readonly ссылок на такие цели.

Свежий Copy literal сразу создаёт значение назначения: обязательная промежуточная копия не требуется. Правило Copy для существующего значения имеет приоритет над last-use move. Само статическое знание `owns` не разрешает move при будущих обращениях к тому же исходному значению.

Future use учитывает ветвления, повторные итерации, вложенные scopes, результаты и async state. Запись нового значения в source не является чтением старого; до неё source после move нельзя читать. Сохраняющиеся aliases отдельно проверяются по lifetime назначения. При неоднозначном control flow compiler выбирает безопасный borrowed режим или отклоняет consuming операцию; скрытого динамического выбора owner не возникает.

```nether
let first: Vec<i32> = [1, 2, 3]
let second = first
println("{}", first.len())  // first используется позже: second является view

let third: Vec<i32> = [4, 5]
let moved = third           // third больше не используется: move
```

## 94.1. Атрибут `Copy`

Пользователь объявляет атрибут как `#[Copy]`:

```nether
#[Copy]
struct Point {
    x: i32
    y: i32
}

let first = Point { x: 1, y: 2 }
let second = first  // новое owns даже если first больше не используется
```

Copy создаёт самостоятельное owned значение назначения без consuming invalidation источника. Для class с допустимым Copy создаётся новая class identity; это не alias исходной identity. Конструктор повторно не вызывается. Compiler может убрать ненаблюдаемое копирование или allocation, но не менять независимость значений.

Copy является проверяемым свойством, а не разрешением произвольного deep clone. Все копируемые owned компоненты должны допускать Copy; тип с пользовательским destructor или некопируемым owned ресурсом не может получить атрибут. Копирование compound value рекурсивно применяет правила его Copy-компонентов: owned Copy class component получает независимую identity. Это может требовать создания дополнительных объектов; Copy не обещает побитовое копирование или постоянную стоимость для любого типа. `Vec` и `String` со своими owned buffers не являются Copy. Нельзя копировать owning descriptor буфера, создав двух владельцев одного allocation.

Копия компонента-view сохраняет его lifetime-зависимость и не приобретает ownership цели. Поэтому Copy compound value может владеть собственным inline storage, продолжая зависеть от внешних borrowed данных. Weak fields копируют handle, а не целевой объект, и остаются разрешены только внутри class.

Numeric primitives, `bool`, `char`, unit и function pointers имеют встроенное свойство Copy. Значение `str` копирует descriptor, сохраняя lifetime UTF-8 данных. Tuple и fixed array имеют Copy, если все их компоненты имеют Copy; пользовательские struct/enum/class получают Copy через проверяемый атрибут. Атрибут не наследуется автоматически производным class.

## 94.2. Inline и ссылочные типы

`struct`, tuple, enum и fixed array имеют inline layout. Оно не означает обязательного Copy: non-Copy inline value передаёт своё содержимое через move или используется через view. Ссылки в inline payload сохраняют собственные ownership/lifetime свойства.

`Vec` и `String` являются отдельными ссылочными типами Nether с descriptors и owned buffers. Для них move передаёт существующее владение, а view обращается к тому же контейнеру/строке. Их специальные literal/factory forms не требуют `new`; требование `new` для пользовательских class остаётся прежним.

Views не становятся Copy значениями автоматически только потому, что представляются pointer. Копирование view descriptor не освобождает compiler от его lifetime. Материализация Copy-значения из view допустима только при действительной цели и разрешённом чтении.

Извлечение payload через pattern matching сохраняет ранее утверждённое отдельное правило §88: оно выбирает move/view без автоматического Copy. Атрибут влияет на последующее обычное value assignment/return извлечённого значения, а не добавляет Copy в саму операцию извлечения.

---

# 95. Owned/borrowed entries, access modes и consuming contracts

Для обычных параметров compiler независимо проверяет ownership и доступ через binding:

| Параметр | Owned entry | Borrowed entry |
| --- | --- | --- |
| `a: T` | Owned значение через readonly binding; можно переместить новому владельцу | Readonly view без права move/promotion |
| `var a: T` | Owned значение через mutable binding | Unique mutable view без присвоения ownership |

Тело проверяется для каждого применимого entry; runtime выбора «owner или view» не требуется. Непосредственная мутация через обычный `a: T` запрещена. В owned entry допустим перенос `a` в `var local` с проверкой уникальности. В readonly borrowed entry такой перенос non-Copy значения запрещён и делает entry недоступным. Таким образом, обычный параметр не является обещанием навечно сохранить readonly после передачи ownership.

```nether
fn consume_and_rename(user: User): User {
    var local = user
    local.set_name("Alex")
    return local
}

var renamed = consume_and_rename(new User())  // owned entry

var source = new User()
consume_and_rename(source)  // compile error: требуется consume, но source нужен ниже
println("{}", source.name)
```

При readonly `let source`, больше не используемом после вызова, тот же consuming call допустим, если move и unique access безопасны. Compiler не передаёт caller-owned объект в изменяющий owned entry по borrowed пути и не подставляет clone. Причина недоступности borrowed entry и consuming requirement сохраняются в diagnostics/metadata, в том числе для function pointers и generics.

```nether
fn identity<T>(value: T): T {
    return value
}

var source = new User()
let borrowed_result = identity(source)
println("{}", source.name)  // readonly borrowed entry

var owned_result = identity(new User())
owned_result.set_name("Alex")  // ownership вернулся получателю
```

Owned return передаёт владение без необратимого readonly ограничения. Borrowed return сохраняет capability и lifetime входа. Для получения mutable owned результата не требуется объявлять промежуточную read/validation функцию с `var` параметром. Ограничения borrowed полей и незавершённых заимствований сохраняются.

```nether
fn rename(var user: User): () {
    var some = user
    some.name = ""
}

var user = new User()
rename(user)
println("{}", user.name)  // mutable borrowed entry
```

В owned entry `some` получает ownership через допустимый move, в borrowed entry — mutable view с исходными lifetime/unique-access ограничениями. `let some = user` предоставляет readonly-доступ через `some`; непосредственно изменить его нельзя. Последующий перенос из такого `some` в `var` допустим только при owned источнике или независимом Copy, а не для полученного readonly view.

Copy arguments создают самостоятельные owned значения по §94. Readonly Copy параметр можно скопировать в mutable local и в borrowed entry, поскольку изменение копии не меняет исходные owned данные. Borrowed компоненты Copy сохраняют свои ограничения. Async mutable borrowed entry дополнительно проверяется по §59.

Обычные instance receivers являются исключением из owned/borrowed специализации: `this` всегда readonly borrowed, `var this` всегда mutable borrowed (§34). Сам экземпляр не потребляется. Методы могут иметь обычные остальные параметры с owned/borrowed entries. Для consuming операции над экземпляром целиком используется free function или static method с явным value parameter. Unbound method pointer сохраняет borrowed receiver contract.

Для нескольких обычных параметров ownership проверяется независимо; возможны смешанные комбинации и дополнительные специализации. Declared access modes bindings сохраняются, а доступность entry зависит от операций тела. Эквивалентные тела compiler может объединять. Option, fields, generics и `dyn` не скрывают consuming effects, ownership или ограничения borrowed доступа.

Function value фиксирует конкретный entry contract при связывании pointer. Сигнатура без `var` сама по себе не гарантирует наличие borrowed entry: consuming-only pointer требует допустимого owned аргумента при каждом вызове. Runtime адаптация и скрытый выбор ownership не вводятся (§26, §74).

Metadata содержит применимые entries, consuming requirements, access modes, owned/borrowed результаты, права borrowed компонентов, lifetime, Copy requirements и weak-provenance effects. Generic compiler-readable body позволяет специализацию; отдельная компиляция использует опубликованные contracts (§91).

Стоимость модели — статический анализ, дополнительные entries и возможный рост code size. Readonly-to-mutable owned move не требует Copy, refcount или runtime проверки уникальности: если статического доказательства нет, операция отклоняется.

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

# 99. FFI и C ABI boundary

Foreign declarations имеют явный ABI:

```nether
extern "C" {
    fn c_write(data: ConstPtr<u8>, length: usize): isize
}
```

`ConstPtr<T>` и `MutPtr<T>` являются raw pointer types для unsafe interop, а не обычными class views. Pointer не владеет целью и не продлевает её lifetime. Вызов foreign declaration, raw pointer dereference и другие непроверяемые операции требуют явного `unsafe {...}`. Unsafe допуск не отменяет проверки обычных Nether ownership операций; он обозначает место, где часть гарантий обеспечивает автор кода.

Обычный пользователь вызывает safe Nether wrapper с обычными views, owned values и `Result`. Wrapper проверяет параметры и явно обеспечивает foreign lifetime/ownership contract. Compiler не выводит безопасность неизвестной C реализации из одного объявления.

Для записи в borrowed данные foreign wrapper требует `var` аргумент/receiver и доказанный unique access. Consuming-only wrapper может получить owned значение через обычный параметр, передать его в mutable local по §24 и затем предоставить writable pointer. Получение raw pointer или `unsafe` сами по себе не повышают права readonly borrowed входа; такая запись нарушает контракт wrapper. Самостоятельная Copy-копия проверяется отдельно.

Через C ABI передаются только ABI-safe scalars, raw pointers, opaque handles и `#[repr(C)]` POD structs с ABI-safe полями. POD не содержит owned ресурсов, destructors, class views или weak fields. Fixed-width integers сопоставляются соответствующим C integer types; `usize`/`isize` используются только при совпадении с target-specific C size types. Не каждый scalar языка автоматически совместим с C: Unicode `char`, а также представление `bool` требуют явно выбранного foreign mapping.

```nether
#[repr(C)]
struct CPoint {
    x: f64
    y: f64
}
```

`repr(C)` фиксирует layout по выбранному target C ABI, а не общий layout для всех платформ. Результат `()` соответствует отсутствию результата C функции. Class, `String`, `Vec`, `dyn`, default enum/tuple layout и `Result`/`Option` непосредственно через C ABI не передаются. Строки и buffers преобразуются явно в pointer + length с согласованной encoding; владение внешним ресурсом представляется opaque handle и согласованной release function. Типизированные packs §96 не являются C varargs.

По умолчанию foreign call может использовать borrowed buffer только до своего возврата. Retention pointer, callback после возврата и передача ownership требуют отдельного явно реализованного протокола wrapper: самостоятельного owned storage/handle, срока действия, завершения внешней работы и release. Raw pointer не является способом безопасно сохранить view на уже завершившийся scope. Если C выполняет callback на другом потоке, обычные class views туда не передаются; используются разрешённые transferable данные/channel capabilities.

Panic не пересекает plain C ABI. Экспортируемый entry/callback использует boundary adapter, который перехватывает panic, выполняет предусмотренный cleanup и преобразует отказ в согласованный C результат: error code, status + out parameter или иной явный протокол. Mapping задаётся автором wrapper; compiler не придумывает универсальное значение ошибки. Для boundary без допустимого способа сообщить отказ выбирается abort до выхода в C.

Foreign C++ exception перехватывается на стороне C++ shim и преобразуется в обычный C результат до возврата в Nether. Нельзя пропускать exception, panic или foreign non-local exit через Nether frames с живыми owned ресурсами. Прямой async C export в первой версии не вводится: интеграция с внешним scheduler требует отдельного протокола запуска, callback и завершения lifetime.

Стоимость FFI определяется выбранным ABI, преобразованием данных, boundary handling и cleanup при ошибке. Wrapper может передать уже подходящий buffer без копирования, если foreign contract это допускает. Отсутствие GC/ARC не делает retaining pointer или неизвестный callback автоматически безопасными.

Разделение ABI и unwind contracts: [Rust Nomicon: FFI and unwinding](https://doc.rust-lang.org/nomicon/ffi.html#ffi-and-unwinding). Для Nether plain C boundary выше всегда запрещает межъязыковой unwind.
