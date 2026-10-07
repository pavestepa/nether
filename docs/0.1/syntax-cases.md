# Проверяемые синтаксические примеры 0.1

Контракт для будущих parser/typecheck tests по [профилю](profile.md).
Эти примеры пока не запускаются: parser отсутствует. `pass` означает ожидаемый
результат после реализации соответствующей формы, а не уже пройденный тест.

## S01 — fixed array, empty array и block (pass)

```nether
fn empty(): {i32; 0} {
    return {}
}

fn choose(enabled: bool): i32 {
    let values: {i32; 2} = {10, 20}
    if enabled {}
    return values[0]
}
```

`return {}` — ArrayLiteral, `{}` после if — Block. Индекс получает `usize`
по контексту. Все значения Copy, allocations и observable cleanup отсутствуют.

## S02 — newline, comments и optional semicolon (pass)

```nether
fn sum(): i32 {
    let first = 1 /* newline внутри комментария завершает инструкцию
    */ let second = 2
    let total = first +
        second
    let other = 4; let extra = 5
    return total + other + extra;
}
```

Результат 12. Проверить исходники с LF, CRLF и CR: AST одинаков, byte spans
соответствуют своему исходнику. После `+` перенос не завершает initializer.

## S03 — return не поглощает следующую строку (type-fail)

```nether
fn broken(): i32 {
    return
    10
}
```

Parser: Return(Unit), затем отдельный expression statement. Typechecker:
несовпадение `()` и `i32` на return, не неявный возврат 10.

## S04 — value branches и явный return (pass)

```nether
fn choose(enabled: bool): i32 {
    return if enabled { 10; } else { 20 }
}
```

Результат 10/20; optional semicolon не стирает локальное значение if.
Отдельно `fn broken(): i32 { 10 }` — type-fail: нет явного return.

## S05 — нет скрытого продолжения call/index (unsupported syntax)

```nether
fn zero(): i32 { return 0 }
fn broken(): () {
    zero()
    (zero)
    [0]
}
```

Первые две строки — два expression statements, не `zero()(zero)`.
Последняя форма — unsupported dynamic array literal `E0105`, не index
результата предыдущего выражения. До реализации unsupported recovery parser
может остановиться на ней; typecheck всей функции не запускается.

## S06 — две простые инструкции требуют separator (parse-fail)

```nether
fn broken(): i32 {
    let first = 1 let second = 2
    return first + second
}
```

Ошибка на втором `let`: ожидался `;`, newline или конец тела.

## S07 — operator precedence (pass)

```nether
fn operators(): bool {
    return 1 + 2 * 3 == 7 && (1 << 3) == 8
}
```

Результат true; `*` раньше `+`, comparisons раньше `&&`.
`1 < 2 < 3` — parse-fail: chained comparison запрещён.

## S08 — const, cast и signed minimum (pass / type-fail)

```nether
const LOW: i8 = -128
fn cast(): u16 {
    return LOW as u16
}
```

Результат 65408. `const HIGH: i8 = 128` — out-of-range.
`const BAD: i8 = 127 + 1` — overflow независимо от release mode.
`const BAD: i8 = -128 % -1` — overflow в обоих режимах.

## S09 — attribute, receiver и member continuation (pass)

```nether
#[Copy]
struct Point {
    x: i32
    fn read(this): i32 { return this.x }
}

fn read(): i32 {
    let point = Point { x: 3 }
    return point
        .read()
}
```

Attribute относится к struct. `.read` продолжает цепочку через newline;
receiver borrowed readonly; возвращаемый i32 Copy. `point` — owned inline
local, завершается в конце функции, observable destructor отсутствует.

## S10 — цикл без implicit return (pass)

```nether
fn total(): i32 {
    var index: usize = 0
    var sum = 0
    let items = {1, 2, 3}
    while index < 3 {
        sum += items[index]
        index += 1
    }
    return sum
}
```

Результат 6. Индекс проверяется при каждом доступе; `while` не даёт значение.
Мутация разрешена только для index/sum. Все locals inline Copy.

## S11 — обязательные unsupported diagnostics

Каждая строка — самостоятельный случай, для которого ожидается указанный code:

| Форма | Code |
| --- | --- |
| `async fn work(): () {}` | E0100 |
| `fn work(...values: i32): () {}` | E0101 |
| `fn work(value: dyn Printable): () {}` | E0102 |
| `fn work(): () { for value in values {} }` | E0103 |
| `fn work(): () { let value = (x) => x }` | E0104 |
| `fn work(): () { let values = [1, 2] }` | E0105 |
| `fn work(): () { let value = operation()? }` | E0107 |

Ни один из этих случаев не должен становиться успешно скомпилированной
пустой операцией или попадать в backend с неполной семантикой.
