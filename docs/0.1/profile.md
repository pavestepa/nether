# Исполнимый профиль Nether 0.1

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
| `interface I { fn ... }` | Статический constraint; signatures, без runtime vtable |

Generic declarations: `<T, U: I & Copy, const N: usize>`; defaults типа
`<T = i32, const N: usize = 4>`. Обязательные параметры идут раньше defaults;
default использует только предыдущие параметры. N допускает литерал, const name
или проверяемое целочисленное const expression. Неподтверждённый размер и
бесконечный inline layout — ошибки. Generic recursion не разрешает бесконечную
цепочку специализаций; такая цепочка должна диагностироваться.

После имени class/struct `: I, J` перечисляет interfaces. Если имя разрешается
в class/struct base, выдаётся unsupported-inheritance, а не случайный layout.
`Copy` — проверяемый compiler constraint; пользовательские типы получают его
через `#[Copy]`. `#[repr(C)]` предназначен для POD struct.

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
Invalid shift/overflow нельзя передавать в host/LLVM как UB или poison.

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
| `E0102` | Наследование class/struct, super, virtual/override, dyn |
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
