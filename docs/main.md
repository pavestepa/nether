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
- автоматически выводит ownership, `view`, lifetime и move;
- имеет явный уникальный `mut`;
- поддерживает `class`;
- поддерживает `struct`;
- поддерживает interfaces;
- поддерживает single class inheritance;
- использует Rust-подобные `enum` и `match`;
- не имеет closures;
- имеет ленивые async operations и structured `async/await`;
- привязывает async tasks к потоку без автоматической миграции;
- использует channel-only communication между потоками;
- использует статический polymorphism по умолчанию;
- использует `dyn` для runtime polymorphism;
- мономорфизирует generics при AOT.

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

`String` является owned динамической UTF-8 строкой стандартной библиотеки.

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

Tuple является inline compound value.

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

`Vec<T>` является стандартным динамическим contiguous container.

Пример:

```nether
let values = [1, 2, 3]

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

Struct не имеет обязательной отдельной heap allocation.

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

```nether
let user = new User()
let second = user
```

Обычное alias assignment не переносит ownership. `second` получает view на тот же объект. Ownership принадлежит контексту; `user` является binding, через который compiler отслеживает созданный ownership root, а не независимым механизмом управления памятью.

```text
owning context → User identity
user          → root binding
second        → view на ту же identity
```

В передаче owned результата, сохранении нового owned объекта и consuming call compiler автоматически выводит move. Пользователь не пишет `owns`, lifetime-параметры или обычный `move`.

```nether
fn make_user(): User {
    return new User()
}

let user = make_user()
```

Здесь результат передаётся во владение получателя. Compiler может выделить объект сразу в подходящем регионе получателя, без временного объекта и копирования его содержимого. Move ownership не означает обязательное физическое перемещение class identity.

```nether
fn add_user(mut users: Vec<User>): () {
    users.push(new User())
}
```

Если контейнер принимает новый owned объект, compiler выводит его передачу в owning context назначения. Передача существующего view не превращает его автоматически в owned объект.

После consuming move прежний owned источник нельзя использовать как владельца. Все сохраняющиеся aliases должны оставаться действительными в пределах lifetime назначения; если это нельзя доказать, операция отклоняется. Автоматическое копирование объекта для обхода ошибки не допускается.

Переприсваивание root binding не делает другие views владельцами и не разрешает их использование после уничтожения цели. Точный момент освобождения заменённого объекта в долгоживущей структуре остаётся открытым вопросом (§93).

---

# 22. Automatic `view` и lifetime-контракты

`view` обычно отсутствует в пользовательском синтаксисе.

```nether
fn print_user(user: User): () {
    print(user.name)
}
```

Compiler выводит `user: view User`, если функция лишь получает доступ к существующему объекту.

```nether
fn identity<T>(value: T): T {
    return value
}
```

`return` сам по себе не требует move: для borrowed аргумента результат может быть view, связанный с lifetime аргумента; для owned входа compiler может специализировать consuming вариант. Новый объект, возвращаемый из `make_user`, является owned результатом (§21).

Compiler выводит и сохраняет связи lifetime между параметрами, receiver, результатом и местами сохранения ссылок. Это часть контракта функции, доступная вызывающему коду и отдельной компиляции (§91).

View не может использоваться после завершения lifetime цели. Возврат view на локальный объект допустим только при доказанном обеспечении lifetime объекта, без скрытого clone или произвольного продления lifetime ресурса.

Views на `String` и элементы `Vec` подчиняются тому же правилу. Операция, инвалидирующая используемый позднее view, отклоняется; compiler не создаёт скрытую копию и не обещает хранить старый буфер для обхода конфликта.

---

# 23. `mut` и уникальный доступ

`mut` означает уникальный mutable access, а не ownership transfer.

```nether
fn normalize(mut user: User): () {
    user.name = user.name.trim()
}

normalize(mut user)
```

На время действия capability никакой конфликтующий alias не может обращаться к изменяемой памяти. Само существование alias не является ошибкой, если его обращения не конфликтуют с capability.

Compiler выводит необходимый mutable receiver у изменяющего метода. Пользователь вызывает его обычным способом:

```nether
user.set_age(25)
values.push(4)
```

Вызов получает временный уникальный доступ. Для явного mutable параметра обычной функции используется `mut` в параметре и в аргументе вызова. Изменение binding по-прежнему отдельно регулируется `let mut` (§24).

Capability заканчивается после последнего зависимого доступа; если метод возвращает mutable view, доступ может продолжаться через результат. Проверка учитывает aliases через поля, контейнеры и вызовы функций. Доступ к вложенному объекту требует проверки его собственных aliases: `mut` корня не является безусловным разрешением изменять весь достижимый граф.

```nether
let first = user
let second = user
update(mut first, second)
```

Если `update` использует `second` для конфликтующего чтения или записи во время изменения того же объекта через `first`, вызов является compile error. Mutable effects метода сохраняются в контракте библиотеки.

Backend может использовать доказанную уникальность для alias analysis; конкретные backend attributes должны соответствовать реально доказанным ограничениям.

---

# 24. Binding mutation

Переприсваиваемая переменная:

```nether
let mut count = 0

count = count + 1
```

Следует отличать:

```nether
let mut value
```

от:

```nether
fn foo(mut value: T)
```

Первое означает mutable binding.

Второе — unique mutable memory access.

---

# 25. Functions

Объявление обычной функции:

```nether
fn add(a: i32, b: i32): i32 {
    return a + b
}
```

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
(mut User) => ()
(mut User) => User
```

Функцию, требующую mutable access, нельзя присвоить переменной типа `(User) => ()`, скрыв это требование. Ownership/view выводятся compiler и учитываются в lifetime-контракте; они не становятся обязательными пользовательскими аргументами generic type.

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
    fn invoke(value: T): R
}
```

```nether
struct AddValue : Mapper<i32, i32> {
    amount: i32

    fn invoke(value: i32): i32 {
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

    fn get_name(): str {
        return name
    }

    fn set_age(new_age: i32): () {
        age = new_age
    }
}
```

В контексте class отсутствует пользовательское `this`. Собственные поля и методы доступны по имени; receiver существует во внутреннем представлении compiler.

Локальные имена и параметры затеняют поля. Поэтому параметры конструктора в примере имеют отличающиеся имена. `name = name` при параметре `name` не считается инициализацией одноимённого поля.

Все объявления и члены по умолчанию public. `public` можно писать явно; закрытый доступ объявляется через `private`.

```nether
class Token {
    private value: String

    constructor(initial_value: String) {
        value = initial_value
    }
}
```

Конструктор должен инициализировать все обязательные поля до использования полностью созданного объекта. Производный class вызывает базовый конструктор через `super(...)` (или `super()` без аргументов).

---

# 34. Methods

Все методы используют snake_case:

```nether
fn get_user(): User
fn calculate_total(): f64
fn send_message(message: str): ()
```

---

# 35. Inheritance

Только class поддерживает concrete inheritance.

```nether
class Animal {
    virtual fn speak(): str {
        return ""
    }
}

class Dog : Animal {
    override fn speak(): str {
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
    fn print(): ()
}
```

Class:

```nether
class User : Printable {
    fn print(): () {
        ...
    }
}
```

Struct:

```nether
struct Point : Printable {
    fn print(): () {
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

```nether
class Box<T> {
    value: T
}
```

Функция:

```nether
fn identity<T>(value: T): T {
    return value
}
```

Constraint:

```nether
fn print_value<T: Printable>(value: T): () {
    value.print()
}
```

Пользовательский синтаксис generics сохраняет форму `Type<T>`. Mutable capability учитывается при проверке операций и параметров; `owns`/`view` не требуется указывать как generic arguments. При необходимости compiler генерирует owned и borrowed специализации с соответствующими контрактами.

Const-параметры для fixed arrays используют Rust-подобное объявление:

```nether
fn length<T, const N: usize>(values: {T; N}): usize {
    return N
}
```

`N` является compile-time значением и частью типа массива. Рекурсивные структуры данных используют class references; бесконечное inline-вложение struct или enum не допускается. `weak` разрешён только для полей class (§89).

Точная грамматика нескольких constraints и дополнительные TypeScript-подобные возможности generics требуют отдельного описания; они не подразумеваются автоматически.

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

`dyn` не отменяет lifetime и mutable constraints. Borrowed представление struct через `dyn` не требует скрытого owned boxing; owned упаковка и её правила хранения требуют отдельного описания.

---

# 41. Enum

Enum использует Rust-style algebraic data model.

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
    Shape.Circle { radius } => {
        3.14159 * radius * radius
    },

    Shape.Rectangle { width, height } => {
        width * height
    },
}
```

Match обязан быть exhaustive.

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
let values: Vec<i32> = [2, 4, 5, 1]

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

Новые `User` передаются в owning context назначения через автоматически выведенный move. Существующие views сохраняют зависимость от lifetime их целей. Правила раннего освобождения удаляемых элементов долгоживущего контейнера пока не выбраны (§93).

---

# 53. Regions

Компилятор автоматически группирует class allocations с совместимыми lifetime и контрактами destruction. Группировка является реализацией логического ownership; она не разрешает ссылки на уже уничтоженные объекты.

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

Эти названия не определяют решение для освобождения заменённых объектов в долгоживущем регионе; вопрос остаётся открытым (§93).

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

Отмена является кооперативной. Пока выполняется код задачи, её состояние и память не освобождаются. После остановки операции или завершения задачи выполняется необходимый cleanup. Compiler не копирует class objects для переноса async state между потоками.

---

# 59. Await boundary

Активный уникальный `mut` не может пересекать `await`.

Запрещено:

```nether
async fn process(mut user: User): () {
    await network.send()
    user.complete()
}
```

если unique capability должна оставаться активной.

---

# 60. Reacquire mut

Правильный вариант:

```nether
async fn process(user: User): () {
    set_loading(mut user)

    await network.send()

    set_complete(mut user)
}
```

Получаем:

```text
mut
release

await

mut
release
```

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

При отмене scope его дети получают кооперативную отмену; scope ждёт прекращения их выполнения до cleanup. При panic scope обеспечивает cleanup и прекращение доступа детей до unwinding родительского состояния. `Result.Err` остаётся обычным значением, а не неявной panic или автоматической отменой соседних задач.

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

```nether
let channel = new Channel<Job>()
```

Отправка:

```nether
channel.send(job)
```

Получение:

```nether
let job = await channel.receive()
```

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

Большой Buffer может поддерживать:

```nether
channel.send(transfer buffer)
```

После transfer sender больше не может обращаться к переданной памяти ни через источник, ни через его aliases. Compiler проверяет aliases до передачи; неустранимый конфликт является compile error. Transfer относится к специальным transferable buffers, а не разрешает передачу произвольного class graph.

Обработка закрытого канала, backpressure, ёмкость и возврат буфера при неуспешной отправке должны быть определены контрактом channel API; конкретная политика пока не выбрана.

Это специальная граница memory domains, а не обычный class assignment.

---

# 68. Destruction

Обычный пользовательский код не содержит:

```text
delete
free
destroy
```

Lifetime выводится компилятором.

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

Destructor вызывается deterministic в соответствии с lifetime-контрактом и до освобождения памяти объекта. Для завершающегося региона выполняется cleanup требующих его живых объектов. Физическая группировка allocations не должна произвольно менять наблюдаемый момент закрытия ресурса.

Unwinding также выполняет cleanup (§92). Порядок взаимозависимых destructors и раннее destruction заменённых объектов долгоживущего региона требуют отдельного решения (§93).

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
WeakField<ClassTarget>

Dyn<Interface>
```

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

```text
(mut User) => ()
```

```text
FunctionType {
    parameters: [{ type: User, capability: unique_mut }],
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

---

# 81. Основные memory invariants

### Invariant 1

Каждый class object имеет одну identity и один текущий owning context. Owned передача меняет логическое владение, сохраняя identity.

### Invariant 2

`new` создаёт новую class identity.

### Invariant 3

Обычное alias assignment не переносит ownership. Для consuming операции или owned результата compiler автоматически выводит move.

### Invariant 4

Class aliases являются views и не могут использоваться после уничтожения цели. Move требует проверки всех сохраняющихся aliases.

### Invariant 5

`mut` означает unique mutable access.

### Invariant 6

Активный `mut` не переживает `await`.

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

---

# 82. Базовая программа

```nether
interface Printable {
    fn print(): ()
}

struct Point : Printable {
    x: f64
    y: f64

    fn print(): () {
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

    fn print(): () {
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
unique mut
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

Пользователь пишет высокоуровневый код, а compiler выводит ownership, views, move, lifetime, mutable receiver и необходимые специализации. Выбранная абстракция не должна добавлять обязательные копирования, аллокации или runtime checks сверх необходимых для её операции.

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

После условия `if`/`while`, заголовка `for`/`fn`, `task_scope`, в теле `match` и block-ветви match parser ожидает тело соответствующей конструкции. В позиции initializer, аргумента или результата `{...}` является inline array. `Point {...}` распознаётся как struct literal. Пустой массив без контекста типа не получает произвольный element type.

Самостоятельный вложенный bare block не является отдельной формой выражения. Blocks, связанные с конструкциями управления, функциями и match branches, разрешены, включая вложенные конструкции. Это явное отличие от общего block-expression синтаксиса Rust.

Expression statements, включая вызовы методов и присваивания, разрешены: parser не ограничивает тело функции только `let` и control-flow constructs. Подробная лексическая грамматика разделения инструкций переносами строк и опциональным `;` ещё требует фиксации. Escape-последовательности `\n` и `\r` внутри строк/символов не заменяют это правило.

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

Принятие таблицы не вводит автоматически отсутствующий в Nether синтаксис raw pointers, borrow operators или Rust closures. Prefix `await` и `mut` разбираются как unary forms Nether; их операнд включает call/index/member access. Например, `await load_user(id)` ожидает результат вызова.

Основа таблицы и порядка вычисления: [Rust Reference: expressions](https://doc.rust-lang.org/reference/expressions.html#expression-precedence). Операнды обычных операций вычисляются слева направо; `&&`/`||` используют short-circuit evaluation.

---

# 87. Арифметика и control flow

На этой стадии принимается Rust-подобная семантика арифметики: числовые типы не получают неявных преобразований между уже типизированными значениями; явный numeric cast записывается через `as`. Тип литерала определяется контекстом, иначе integer/floating defaults — `i32`/`f64`.

`usize`/`isize` являются pointer-sized integer types. Индексы контейнеров и размеры используют `usize`. Деление целых на ноль вызывает panic. Проверки integer overflow по умолчанию включены в debug и отключены в release, с возможностью явно выбрать режим проверки. При отключённых проверках переполнение `+`, `-`, `*` выполняет wrapping по разрядности типа; при включённых — вызывает panic. Конкретные compiler flags и полный набор casts Nether ещё должны быть перечислены. Signed `MIN / -1` и `MIN % -1` требуют panic даже при отключённых обычных overflow checks.

Семантическая основа: [Rust Reference: operator expressions](https://doc.rust-lang.org/reference/expressions/operator-expr.html).

`if`, `while`, `for`, `loop`, `break`, `continue` и `return` используют Rust-подобный control flow с синтаксисом типов и массивов Nether. Conditions имеют тип `bool`. `for value in values` выполняет iteration, а `break`/`continue` относятся к соответствующему циклу. `loop` может возвращать значение через `break value`.

`if` и `match` могут возвращать значения; типы продолжающих выполнение ветвей должны согласовываться. Последнее выражение разрешённого block может задавать его результат и неявный результат функции. Явный `return` сохраняется. Инструкция, явно завершённая `;`, не является tail result.

Отсутствие самостоятельных bare block expressions и использование newline в существующих примерах являются отличиями Nether; полный grammar §86 должен определять, где заканчивается tail expression.

Семантическая основа результата block: [Rust Reference: block expressions](https://doc.rust-lang.org/reference/expressions/block-expr.html).

---

# 88. Pattern matching и извлечение payload

`match` поддерживает Rust-подобные wildcard `_`, destructuring и guards. Guards не разрешают consuming extraction до принятия ветви. Все возможные случаи должны быть покрыты, а типы результатов ветвей согласованы.

```nether
match result {
    Result.Ok(value) if is_valid(value) => consume(value),
    Result.Ok(_) => handle_invalid(),
    Result.Err(error) => handle_error(error),
}
```

Извлечение payload не выполняет автоматический Copy/clone объекта. Compiler выбирает move при consuming использовании owned payload или view при borrowed доступе, сохраняя lifetime-контракт. После move соответствующее исходное значение или поле нельзя использовать до восстановления допустимого состояния.

`mut` определяет право на уникальный mutable access к payload и входит в проверку pattern bindings; он сам по себе не означает ownership transfer. Конкретная грамматика mutable patterns ещё требует фиксации. Эти правила являются отличием от автоматического Copy для некоторых типов Rust.

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

    fn has_parent(): bool {
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

Реализация должна безопасно обнаруживать уничтоженную цель. Это может требовать проверяемого handle и дополнительных metadata; конкретный механизм пока не выбран. `weak` не является обещанием бесплатного raw pointer access и не определяет момент уничтожения цели. Освобождение долгоживущих graphs остаётся открытым (§93).

---

# 90. Форматирование и interpolation

Поддерживаются Rust-подобные format placeholders и JS-подобные interpolated literals:

```nether
println("{}", value)
println(`${value}`)
let message: String = fmt("value = {}", value)
```

Formatted functions используют variadic pack с ограничением форматирования, а не обязательное преобразование каждого аргумента в отдельный `String`. Концептуальный контракт `println` — format template, pack форматируемых значений и результат `()`; точное написание generic pack в объявлении функции требует отдельной grammar.

Для постоянного шаблона compiler разбирает placeholders, проверяет число/типы аргументов и генерирует запись непосредственно в writer. `println` не требует промежуточной строки для каждого значения. `fmt` создаёт один итоговый owned `String`; allocation для результата является стоимостью выбранной операции.

Interpolated literal lowering учитывает назначение: при непосредственном formatted output compiler может писать части напрямую, а в контексте owned `String` формируется строка. Вызов произвольной функции должен соблюдать её реальный контракт и не получает автоматически writer semantics.

Выражения interpolation вычисляются однократно в порядке появления. Выведенное форматирование использует views и не получает hidden ownership transfer. `\n`, `\r` и остальные утверждённые escape sequences применяются внутри соответствующих литералов; полная grammar escaping и динамических шаблонов пока требует описания.

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
- выведенные ownership, consuming effects и lifetime-связи;
- mutable receiver и capabilities параметров;
- async effects и ограничения thread transfer;
- представление generic bodies, необходимое для специализации.

Вызывающий код проверяется по этим контрактам без чтения исходного тела обычной функции. Для generic specialization поставляется compiler-readable представление. Пользователь обычно не пишет выведенные контракты; IDE может их показывать. Изменение такого контракта может менять совместимость публичного API.

Правила поиска пакетов, versioning, circular imports, разрешения private access при inheritance и формат binary metadata ещё требуют описания. Для FFI и внешнего ABI понадобится отдельный явный контракт, не полагающийся на анализ неизвестного тела.

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

При unwinding выполняются необходимые destructors и cleanup regions. Async scope не освобождает память, пока child tasks продолжают обращаться к ней (§61). Конкретный порядок destructors, повторная panic во время cleanup и FFI unwinding boundaries требуют отдельной спецификации.

Основа cleanup при unwinding: [Rust Reference: panic](https://doc.rust-lang.org/reference/panic.html#unwinding). Для Nether здесь утверждён unwind; наличие Rust-подобного альтернативного abort-профиля не считается автоматически утверждённым.

---

# 93. Открытый вопрос: долгоживущие изменяемые структуры

Статус: решение не выбрано. Этот вопрос не закрывается автоматическим move или выбором места allocation.

Требуется определить, когда освобождаются заменённые и удалённые объекты cache, graph и постоянно обновляемых контейнеров, если owning context продолжает жить. Нельзя считать утверждённым ни удержание всей памяти до конца приложения, ни автоматическое раннее освобождение каждого недостижимого объекта.

Нужно согласовать:

- физическое освобождение памяти и наблюдаемый момент destruction;
- действительность существующих views и weak references;
- удаление частей циклического graph;
- lifetime независимых элементов контейнера и их buffers;
- стоимость metadata, allocator operations и возможных checks;
- гарантии ограниченного потребления памяти при длительных обновлениях.

Для предложений по этому вопросу необходимо явно показывать новые пользовательские ограничения и runtime стоимость. Ни GC, ни ARC не вводятся как неявное решение. Выбор политики будет отдельным изменением спецификации.
