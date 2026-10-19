# Исполнимый профиль Nether 0.1

Архитектура: [Nether → Rust](rust-backend.md). Правила памяти сохраняются; прямой LLVM backend удалён.

Статус: контракт этапа 0 для новой реализации. Этот документ фиксирует
синтаксический subset и числовую семантику; он не означает, что parser,
ownership checker или backend уже готовы. Реализованная часть отмечена в
[статусе](status.md). Приоритет: уточнения [roadmap](roadmap.md), затем этот
профиль, затем [основная спецификация](../main.md).

## 1. Лексический контракт

Исходник — UTF-8. Spans задаются полуоткрытыми диапазонами **байтов** исходного
файла; позиции для человека — строка и столбец с 1. Столбец считается в Unicode
scalar values, табуляция учитывается отдельно при отрисовке diagnostics.
LF, CRLF и CR считаются одним переносом строки каждый. BOM допускается только
в начале файла и не меняет исходные byte offsets.

В начальном профиле identifiers: `[A-Za-z_][A-Za-z0-9_]*`. `_` в pattern —
wildcard. Unicode допускается в строках, char и комментариях; Unicode identifiers
отложены и диагностируются, а не нормализуются неявно. Имена типов/функций
следуют §2 main.md; нарушение naming convention не меняет смысл токенов.

Комментарии: `//` до переноса, вложенные `/* ... */`. Переносы внутри block
comment сохраняют влияние на разделение инструкций. Незакрытый comment — ошибка.

Integer literals: decimal, `0b`, `0o`, `0x`; `_` только между digits.
Float literals: decimal `digits.digits`, `digits[eE][+-]?digits` или обе формы
вместе. Начальные/конечные точки `.5`/`5.` не обозначают float: нужно `0.5`/`5.0`.
Числовых suffix в этом профиле нет: тип задаётся контекстом, annotation или `as`.
Знак не входит в token числа. Невалидная цифра основания и неизвестный suffix
диагностируются на всём числовом token, а не разбиваются на корректный префикс
и имя. Значение literal проверяется до приведения к машинному типу.

String — `"..."`, char — `'...'`. Escapes: `\\`, `\"`, `\'`, `\n`, `\r`,
`\t`, `\0`, `\xNN` (ASCII, 00–7F), `\u{H...}` (1–6 hex digits, только
Unicode scalar: не surrogate и не больше 10FFFF). Raw newline в этих literals
запрещён. Char содержит ровно один decoded scalar. String имеет статические
UTF-8 данные и тип `str`; неявная allocation `String` в профиле 0.1 не требуется,
используется библиотечная factory. Backtick interpolation диагностируется.

Lexer сохраняет newline tokens; решение о завершении инструкции принимает
parser с учётом контекста. Все ошибки получают source span, code и сообщение.

## 2. Типы и формы объявлений

| Форма | Контракт |
| --- | --- |
| `i8/i16/i32/i64/i128`, `u8/u16/u32/u64/u128` | Заданная разрядность; signed — two's complement |
| `usize`, `isize` | Разрядность target pointer; для начального target 64, независимо от host |
| `f32`, `f64` | IEEE binary32/binary64; без fast-math по умолчанию |
| `bool`, `char`, `()` | Boolean, Unicode scalar, единственное unit значение |
| `str` | Readonly UTF-8 descriptor; Copy descriptor сохраняет lifetime данных |
| `(T,)`, `(T, U)` | Inline tuple; `(T)` — скобки вокруг типа |
| `{T; N}` | Inline array, N — compile-time `usize`; `{T; 0}` допустим |
| `*T`, `*var T` | Raw read/write pointer; не safe view и не owner |
| `(T, var U) => R` | Function pointer с конкретным entry contract |
| `Name<T, N>` | Именованный тип; generic type/const arguments |
| `struct S { ... }` | Inline fields, methods; литерал `S { field: expr, ... }` |
| `class C { ... }` | Identity, constructor/destructor, allocation через `new C(...)` |
| `enum E { A, B(T), C { field: T } }` | Tagged inline union; active payload |
| `interface I { fn ... }` | Реализованный статический constraint; dyn — отдельный этап C4.4 |

Generic declarations: `<T, U: I & Copy, const N: usize>`; defaults типа
`<T = i32, const N: usize = 4>`. Обязательные параметры идут раньше defaults;
default использует только предыдущие параметры. N допускает литерал, const name
или проверяемое целочисленное const expression. Неподтверждённый размер и
бесконечный inline layout — ошибки. Generic recursion не разрешает бесконечную
цепочку специализаций; такая цепочка должна диагностироваться.

После имени class/struct `: I, J` перечисляет interfaces. Class может объявлять
один class-parent первым, затем interfaces. Declaration checker проверяет граф,
циклы и override signatures; полноценный lowering concrete inheritance пока
отвергается с E0900. Struct не может наследовать concrete class/struct.
`Copy` — проверяемый compiler constraint; пользовательские типы получают его
через `#[Copy]`. `#[repr(C)]` предназначен для POD struct.

`virtual fn` / `override fn` допускаются только у public class instance methods.
`override` требует доступного inherited virtual/override method с тем же receiver
mode, mutability параметров, результатом, generic kinds/constraints/defaults.
Параметры method generics могут переименовываться; порядок conjunction constraints
не важен. Неявное hiding доступного inherited member запрещено. Private base
method нельзя override; независимый одноимённый member в derived scope допустим.
Parent type/const arguments подставляются без захвата method binders или module
constants. Для symbolic const signatures нет solver алгебраической эквивалентности.
Root-class virtual methods уже компилируются как обычные static entries;
это ещё не реализация inherited methods, super, base upcast или dyn dispatch.

Private members class/struct доступны только из методов, constructor и destructor
объявляющего типа; нахождения в том же модуле недостаточно. Это lexical context:
free helper и другой type не получают права caller, generic specialization и
receiver aliases не меняют declaring type. Чтение/запись поля, named patterns,
literal construction и создание method pointers проходят одну проверку E0352
с note на declaration. Private top-level declarations остаются module-private.
Публичный метод вправе вернуть function pointer на private implementation;
полученный pointer вызывается по его обычному callable contract.

Interfaces реализованы номинально: matching methods без `: I` не удовлетворяют
constraint. Все требуемые методы публичны, совпадают по receiver/mutability,
типам параметров и результата, method generics, constraints и defaults.
Поддержаны generic interfaces с type/const arguments, defaults, conjunction
`T: I & J & Copy`, static methods и function values. Повтор одного constraint
допустим; общие методы conjunction должны иметь совместимую сигнатуру.
Generic body при специализации проверяется на abstract types: доступны только
объявленные constraints, дополнительные методы конкретного типа недоступны.
Обычные owned/borrowed entries сохраняют provenance, lifetime и may-write effects.
Method-level const expressions в сигнатурах должны совпадать после переименования
параметров; произвольная алгебраическая эквивалентность не доказывается.
Interface value и dyn storage пока не реализованы.

Компактная grammar (обозначение `list(X)` — comma-separated, optional trailing
comma; newline не заменяет comma):

```ebnf
module       = { import | reexport | declaration } ;
import       = "import", "{", list(name), "}", "from", string, end ;
reexport     = "export", "{", list(name), "}", "from", string, end ;
declaration  = { attribute }, [visibility],
               (function | aggregate | enumeration | interface | constant | foreign) ;
visibility   = "public" | "private" ;
attribute    = "#[Copy]" | "#[repr(C)]" | "#[link(", string, ")]" ;
function     = "fn", name, [generics], "(", list(parameter), ")", ":", type, block ;
parameter    = ["var"], pattern, ":", type | ["var"], "this" ;
constant     = "const", name, ":", type, "=", expression, end ;
aggregate    = ("struct" | "class"), name, [generics], [interfaces],
               "{", { field | method | constructor | destructor | constant }, "}" ;
field        = [visibility], ["weak"], name, ":", type, end ;
method       = [visibility], function ;
constructor  = "constructor", "(", list(parameter), ")", block ;
destructor   = "destructor", block ;
enumeration  = "enum", name, [generics], "{", list(variant), "}" ;
variant      = name, ["(", list(type), ")" | "{", list(name, ":", type), "}"] ;
interface    = "interface", name, [generics], "{", {signature, end}, "}" ;
foreign      = "extern", '"C"', ("{", {signature, end}, "}" | function) ;
signature    = "fn", name, [generics], "(", list(parameter), ")", ":", type ;
block        = "{", {statement}, "}" ;
statement    = ("let" | "var"), pattern, [":", type], "=", expression, end
             | "return", [expression-on-same-line], end
             | ("break" | "continue"), end
             | "unsafe", block
             | expression, end ;
```

Это grammar table, а не готовый parser generator input. Ограничения контекста:
receiver допустим только первым параметром instance method/interface signature;
constructor не имеет явного receiver. Constructor — только class; struct
инициализируется литералом. Destructor допустим для class/struct и не принимает
параметров. `weak` — только class field с class target. Foreign signatures не
имеют generics, receivers, default arguments или неподдержанных Nether aggregates;
POD repr(C) structs разрешены по [ABI-контракту](abi.md).
Link attribute относится к extern-блоку; строка — имя библиотеки, не shell code.

Выражения: литералы, names/paths через `.`, tuple, fixed array, struct literal,
enum construction `E.V(...)`/`E.V { ... }`, `new`, call, member, index, unary,
binary, cast, assignment, `if condition block [else block-or-if]`,
`while condition block`, `match expression { list(arm) }`.
`while` имеет тип `()`, `break` не принимает значения. Conditions — только bool.
В condition/scrutinee перед body неоднозначный struct literal требует скобок:
`if (Point { x: 1 }).is_valid() { ... }`. Generic calls — `f<T>(...)`;
parser признаёт type-argument list только при корректном закрытии и следующем
`(`, иначе `<` — comparison. В type context `>>` делится на два `>`.

Pattern: literal, `_`, `[ref] [var] name`, tuple, fixed array, enum positional/
named payload, struct fields; один rest `..` на уровень. `arm` — pattern,
optional `if guard`, `=>`, expression или block. Guard только читает, payload
move/mutable loan начинается после его принятия. Guard не засчитывается как
безусловное покрытие variant; match должен быть exhaustive. Extraction не
добавляет Copy. Комбинации move/ref на пересекающихся places запрещены.

Function values в обязательном объёме — ссылки на именованные функции и unbound
methods. `loop`, range expressions, `?`, lambda syntax, `if let`/`while let`
и другие формы за пределами таблицы не нужны для обязательного объёма roadmap;
до отдельного расширения профиля они диагностируются как неподдерживаемые.
Это не заменяет function pointers или обычный exhaustive `match`.

## 3. Инструкции, массивы и результат

`{...}` в expression position всегда array literal; `Name {...}` — struct/enum
literal. В позиции тела fn/if/else/while/match/unsafe это block. Bare block не
вводится. `{}` без ожидаемого array type не позволяет вывести тип элемента.
У tuple из одного элемента нужна comma; array literal не использует `[...]`.

Простая инструкция завершается `;`, newline или закрывающей `}` тела.
Две инструкции на одной строке требуют `;`. Переносы игнорируются внутри
parentheses/index/data literals, после `=`, infix operator, `=>` и comma,
перед ожидаемым body, между `}` и связанным `else`, между attribute и declaration,
перед member `.`. Начальные `(` и `[` на новой завершённой строке не продолжают
предыдущий call/index. Newline после `return`/`break`/`continue` завершает их.

Функция возвращает полезное значение только через `return`; достижение конца
non-unit функции — ошибка. `return` без значения возвращает `()`.
If/match в expression context получают локальное значение последнего expression
statement каждой продолжающей ветви, независимо от optional `;`.
Non-unit if требует else; типы ветвей должны совпасть. В statement context
результат отбрасывается с необходимым cleanup.

## 4. Приоритеты и evaluation order

От сильного к слабому; postfix цепочки группируются слева:

| Операторы | Ассоциативность / ограничение |
| --- | --- |
| `.`, call `(...)`, index `[...]` | Последовательная postfix цепочка |
| Prefix `-`, `!`, `*`, `&`, `&var` | Справа; raw pointer операции требуют unsafe; `*safe_view` сохраняет capability |
| `as` | Слева |
| `* / %` | Слева |
| `+ -` | Слева |
| `<< >>` | Слева |
| `&` | Слева |
| `^` | Слева |
| `\|` | Слева |
| `== != < > <= >=` | Без chaining: нужны скобки |
| `&&` | Слева, short circuit |
| `\|\|` | Слева, short circuit |
| `= += -= *= /= %= &= ^= \|= <<= >>=` | Справа; результат `()` |

Операнды и аргументы вычисляются слева направо. Assignment сначала вычисляет
destination place (включая index) один раз, затем RHS. Compound assignment
проверяет mutable access, читает старое значение и применяет ту же arithmetic
operation; повторного вызова index expression нет. Нужные проверки проходят
до записи. Для owned replacement уничтожение/передача старого значения требует
отдельного memory-контракта, который эта числовая таблица не подменяет.

## 5. Числовая семантика

Литерал получает ожидаемый тип; без контекста integer — `i32`, float — `f64`.
Между типизированными числовыми значениями нет implicit casts. Arithmetic и
comparison требуют одинаковый тип (даже `u64` и `usize` различны). Shift RHS
может быть любым integer type. `!` для integer — bitwise NOT, для bool — NOT.
Unary `-` для unsigned запрещён. `-128` допустим как `i8`: parser/typechecker
проверяет знак и magnitude совместно; `128` как положительный `i8` — ошибка.

| Ситуация | Checked | Wrapping |
| --- | --- | --- |
| Integer `+ - *`, unary signed `-` за границей типа | Panic overflow | Modulo 2^bits |
| Integer `/ 0` и `% 0` | Panic division-by-zero | Та же panic |
| Signed `MIN / -1`, `MIN % -1` | Panic overflow | Та же panic, включая remainder |
| Shift count < 0 или >= ширины LHS | Panic invalid-shift | Count по modulo ширины LHS |
| Биты, вытесненные left shift | Отбрасываются | Отбрасываются |
| Signed right shift | Sign extension | Sign extension |
| Unsigned right shift | Zero extension | Zero extension |
| Индекс >= array length | Panic bounds | Та же panic |

Signed division округляет к нулю, remainder имеет знак dividend (или равен 0).
Debug default — checked; release default — wrapping. Планируемая опция
`--overflow-checks=on|off` переопределяет default независимо от оптимизации.
Const evaluation **всегда checked**, ошибки становятся compile diagnostics.
Invalid shift/overflow проверяется до Rust операции; правила Nether не зависят от rustc overflow defaults.

Float arithmetic работает в заявленной разрядности с round-to-nearest ties-even;
IEEE NaN/Inf/±0 допустимы. Float `/ 0` не integer panic. `%` — remainder с
truncated quotient, не Euclidean modulo. Comparison с NaN: `!=` true,
остальные false. Нет неявной reassociation, fast-math или fused multiply-add.
Точность NaN payload и знак NaN не являются переносимым контрактом.

| `as` | Результат |
| --- | --- |
| Integer → integer | Narrow: младшие биты; widen: sign/zero extension исходного типа; same width сохраняет bits |
| Integer → float | Ближайшее представимое, ties-even; overflow → соответствующая infinity |
| Float → integer | Truncate к нулю и saturate к min/max; NaN → 0; ±Inf → граница |
| f32 → f64 | Точное значение для finite |
| f64 → f32 | Round ties-even; overflow → infinity |
| bool → integer | false → 0, true → 1 |
| char → integer | Unicode code point, затем integer narrowing |
| u8 → char | Соответствующий Unicode scalar |

Другие numeric/bool/char casts отвергаются: например, integer → bool или
arbitrary integer → char. Pointer casts имеют отдельные unsafe/provenance
обязанности и не следуют автоматически из integer cast table.

`compiler/semantics` реализует integer operations, comparisons, integer casts
и float-to-int cast как oracle. Он возвращает структурированную ошибку вместо
запуска Nether panic: frontend превратит её в compile diagnostic, MIR/backend —
в соответствующий runtime failure path. Float arithmetic, parser и lowering
этим модулем не реализованы.

## 6. Диагностика неподдерживаемых форм

Коды здесь — контракт будущего frontend; наличие таблицы не означает готовый
парсер. Диагностика указывает конструкцию и поддержанный способ выразить задачу,
если он есть. Unsupported syntax не доходит до codegen.

| Код | Формы / причина |
| --- | --- |
| `E0100` | async/await/spawn/task_scope, transfer, thread/channel API профиля языка |
| `E0101` | `...` packs/varargs/forwarding |
| `E0102` | Неподдержанные super/dyn; virtual/override разбираются и проверяются, concrete inheritance lowering пока даёт E0900 |
| `E0410` | Некорректный class-parent, несколько родителей, inheritance cycle |
| `E0411` | Некорректный virtual/override, несовместимая сигнатура или member hiding |
| `E0103` | for, loop, range expressions; использовать while |
| `E0104` | Lambda/capture; использовать named fn и явное состояние |
| `E0105` | Backticks/interpolation, dynamic array literal; библиотечные операции |
| `E0106` | Associated types/GAT, пользовательские lifetime/ownership annotations |
| `E0107` | `?`, if-let/while-let; использовать match |
| `E0108` | Иная неизвестная/неподдерживаемая конструкция или attribute |
| `E0110` | Не-ASCII identifier в начальном профиле |

`Option`, `Result`, `Vec`, `String` и произвольные пользовательские типы не
распознаются по имени ради обхода checker. До реализации обязательной возможности
её отдельная диагностика должна говорить «ещё не реализовано», а не «исключено
из 0.1». Ошибки arithmetic oracle: `TypeMismatch`, `LiteralOutOfRange`,
`Overflow`, `DivisionByZero`, `InvalidShift`, `UnsignedNegation`.

### Function values и specialization

Named/static function может использоваться как value. Для method values
допустима явная специализация owner и method, например `Box<i32>.method<bool>`. Для generic function value
в текущем subset необходим concrete expected function type: например,
`let f: (i32) => i32 = identity` или `identity<i32>`. Type/const arguments выводятся
из этой signature, после чего проверяется обычный specialization и абстрактное тело.
В expression context generic argument list допускается также перед концом value
(`)`, `]`, `}`, comma, semicolon, newline, EOF), а не только перед call/member access.
Неприменимые function value entry contracts диагностируются; runtime выбора
owned/borrowed варианта по одному function pointer нет.
Function values сохраняют статический owned/borrowed ABI, capability и provenance
результата, component origins и may-write effects. Работают unbound receivers,
generic/higher-order functions, returned pointers и выбор совместимых pointers.
Значение занимает один code pointer без environment; metadata есть только в compiler.
Несовместимые owned/borrowed result ABI отвергаются, implicit adapter не создаётся.

### Borrowed returns: реализованный C3.1

Для non-Copy результата compiler выводит owned либо borrowed контракт выбранного
entry. Borrowed контракт включает readonly/mutable capability и зависимость от
параметров с projection paths. Работают direct/generic/imported calls, методы,
ветвления и рекурсивные группы. Например, `fn id<T>(x:T):T {return x}` при borrowed
аргументе возвращает view, при owned аргументе передаёт владение. Copy-result
копируется по обычным правилам.

`let ref var result = owner.method()` допустим для mutable borrowed result;
readonly результат не повышает права при связывании с `var`. View не может
пережить локального владельца, owned parameter или temporary receiver. Разные
ветви одного entry не могут смешивать owned и borrowed return. Для dynamic index
зависимость консервативно относится ко всему содержащему месту. Recursive result
и target-effect contracts выводятся до стабильного состояния.

Borrowed result можно сохранить в readonly поле локального aggregate (см. ниже).
Ошибки конфликтов показывают место владельца, создание view и последующее
использование, поддерживающее заимствование. `E0395` обозначает несовместимый
result contract или выход view за lifetime владельца.


### Readonly aggregate storage: реализованный C3.2

Structs, tuples, fixed arrays и enum payload могут хранить readonly
views non-Copy значений. Source type остаётся прежним: внутреннее представление
поля выводится статически из initializer. Поля struct/tuple и variant payload могут
иметь разные режимы; все элементы одного массива имеют одинаковый режим.

```nether
struct Item {value:i32;destructor {}}
struct Holder {value:Item}
fn main():i32 {
    let owner=Item {value:7}
    let ref view=owner
    let holder=Holder {value:view}
    let elements={view,view}
    return holder.value.value+elements[0].value
}
```

Перенос оболочки переносит descriptors; копирование Copy tuple/array копирует
только descriptors. Target не копируется и не уничтожается при cleanup оболочки.
Owned соседние поля очищаются по обычным правилам, включая частичное создание и
panic. Извлечение поля сохраняет readonly capability и исходный lifetime, в том
числе через несколько вложенных оболочек и view на всю оболочку.

В mutable оболочке `holder.value = another_owner` перепривязывает descriptor.
`holder.value.value = 1` не разрешается: mutable оболочка не даёт права изменять
readonly target. Новый target должен оставаться жив при последующих чтениях.
Режим готового места не меняется при присваивании. Несовместимые owned/view
режимы диагностируются (`E0396`); runtime ownership tag не вводится.

Такие оболочки передаются и возвращаются через ordinary functions, методы,
generics и imports/reexports. Entry специализируется по physical storage types
аргументов. Result contract отдельно хранит пути borrowed компонентов и их
зависимости от параметров; inference достигает fixed point, включая рекурсивные
вызовы. Возврат оболочки с view локального/owned target запрещён. View на target
внутри полученного по значению wrapper может пережить сам wrapper, если target
принадлежит внешнему владельцу. Descriptor slot и его target имеют разные lifetime.

Mutable borrowed aggregate arguments поддерживают переназначение readonly
компонентов через `var`-параметр или `var this`. Static may-write contract связывает
каждое изменяемое поле с входными origins. Запись view локального или owned target,
уничтожаемого в callee, запрещена даже при последующей panic. Контракт проходит
через calls, generics, imports и рекурсию; caller учитывает новые зависимости на
normal и panic edges. Массивные dynamic indices расширяются до возможных slots.
Это не даёт mutable-доступа к readonly targets и не меняет physical storage mode.

Анализ консервативен: после вызова прежние зависимости полей сохраняются наряду
с возможными новыми; гарантированный overwrite пока не освобождает старый target
от lifetime restrictions. Обычная передача Copy aggregate по значению остаётся
копированием descriptor storage; `var` у такого параметра сам по себе не означает
изменение caller storage. Mutable receiver/borrowed non-Copy entry меняет target.
Class payloads поддерживают те же static storage modes; constructor, методы и
destructor используют согласованный layout. Destructor оболочки удерживает loans
до implicit reads при cleanup; target обязан уничтожаться позже оболочки.
Извлечённый view можно передавать и возвращать, в том числе через function pointer.
Для именованных локальных переменных режимы также выводятся из последующих
прямых присваиваний: к полям struct/tuple, элементам array и wrapper целиком.
Требование readonly компонента переносится на initializer и связанные локальные
wrapper bindings до fixed point. Это работает и для nested fields, mutable aliases,
параметров, returned values, if/match и function values. Parameter/result requirements
распространяются между entries; final representation каждого entry фиксирована.
Destructor dependencies не меняют установленный порядок cleanup. Внутренние циклы
между owning частями одной группы относятся к C7.

У массива единый mode выводится из требований всех элементов, включая повторные
uses одного target. Порядок элементов не заставляет выбирать owned mode раньше,
чем обнаружен readonly компонент. Initializers при этом вычисляются ровно один
раз в исходном порядке; числовые literals сохраняют contextual type первого
элемента. Если для согласования нужен view временного owning значения, программа
отвергается: compiler не создаёт скрытого владельца/heap allocation. Уже созданное
storage имеет один mode; никакого runtime переключения при присваиваниях нет.

Для локальных enum известный variant переносится через создание, move и root
replacement. Если доказано `e = E.Empty`, чтение borrowed payload в недостижимой
`E.Item`-ветви больше не создаёт ложный lifetime conflict. Анализ учитывает joins
и повторные проходы loops: при разных возможных variants остаются обе ветви.
Те же доказанные переходы используются для initialization, result/effect contracts
и generated Rust. Даже недостижимые source-ветви проходят обычную проверку типов.

Calls и записи через projected/aliased places сбрасывают знание variants;
произвольные вложенные enum places пока не уточняются. Поэтому после таких
операций безопасная программа ещё может быть отклонена консервативно. Эти
ограничения могут консервативно отклонять безопасные программы; C3 закрыт.
Примеры: [borrowed_storage](../../examples/borrowed_storage/main.nr), результат 24;
[borrowed_storage_write](../../examples/borrowed_storage_write/main.nr), результат 10;
[enum_refinement](../../examples/enum_refinement/main.nr), результат 3;
[storage_modes](../../examples/storage_modes/main.nr), результат 11.

## Source classes: реализованный C2

`new C(...)` вызывает единственный constructor класса. У класса без полей
допустим неявный пустой constructor. Класс с полями требует constructor;
все поля должны быть инициализированы на каждом успешном пути выхода.
Typed CFG учитывает aliases и exhaustive branches; чтение поля до initialization
и передача неполного `this` в вызов отвергаются с `E0393`.

Class move сохраняет identity. `#[Copy] class` требует Copy-поля и создаёт
независимый объект; destructor у Copy-типа запрещён. `==`, `!=` и intrinsic
`same_identity(a,b)` сравнивают совместимые class operands через readonly borrow,
включая Copy classes. Сравнение не создаёт новый объект и не потребляет operands.

Class views допускают поля, methods и rebind по capability. `*view = new C(...)`
отвергается с `E0394`: view не даёт права заменить owning slot. Inheritance и dyn
ещё не реализованы. Пример: [classes/main.nr](../../examples/classes/main.nr).
