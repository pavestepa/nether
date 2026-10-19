# Замена inline объекта во время его destructor

Архитектура: [Nether → Rust](../rust-backend.md). Правила памяти сохраняются; прямой LLVM backend удалён.

Статус: утверждён вариант 1 — запрет whole replacement завершаемого target (выбор пользователя).

## Пересечение принятых правил

`main.md` §69 задаёт implicit exclusive `this` в destructor, body до cleanup
owned полей и exactly-once destruction. Принятое правило safe inline views
разрешает `*view = value`; class views замену whole object не разрешают.
Замена root обновляет порядок cleanup после успешной инициализации.

Для inline struct эти правила пока не определяют следующее:

```nether
struct Resource {
    value: i32
    destructor {
        if value == 1 {
            *this = Resource { value: 2 }
        }
    }
}
```

`this` — mutable view на место, старое значение которого уже уничтожается.
Простой сброс drop flag перед body предотвращает двойной вызов destructor,
но не определяет судьбу нового значения и старых owned полей. При обычной
замене `owner = fresh` новое значение из destructor также может оказаться
перезаписанным следующим commit. Одного bool initialized для такой семантики
недостаточно. Запись отдельных полей не создаёт этой проблемы: их собственное
уничтожение ещё не началось.

## 1. Запретить whole replacement уничтожаемого объекта (рекомендация)

Пока выполняется destructor, его target нельзя целиком заменить, повторно
инициализировать или передать в операцию, способную сделать такую замену.
Ограничение относится к target, поэтому сохраняется через aliases и calls.
Обычный consuming move из borrowed `this` уже запрещён.

Можно читать и изменять поля, заменять живые owned поля по обычным правилам,
вызывать методы с совместимыми effects и выполнять явный finish/close. Объект
проходит свой destructor один раз; затем очищаются оставшиеся initialized поля.

Compiler отличает capability завершаемого target и проверяет call effects.
Если отсутствие whole replacement в вызываемой функции не доказано, такой
mutable вызов консервативно отклоняется. Правила записи через обычный mutable
view вне destructor не меняются. Перепривязка локального alias сама по себе не
заменяет target и не является whole replacement.

## 2. Разрешить replacement с отдельным lifecycle protocol

`*this = fresh` устанавливает новое значение, последующие обращения через
`this` видят его. Старый destructor не запускается повторно, но его body
завершается, после чего очищаются сохранённые старые owned поля. Новое значение
получает новый cleanup lifetime и собственный destructor.

Потребуются отдельное состояние running-destructor, сохранение старых owned
частей и поддержка повторной инициализации в generated glue. Операция,
обещающая завершить slot lifetime (drop/replace/remove), должна очистить и
значения, созданные при self-replacement, прежде чем считать slot свободным.
Для примера body Resource(1) завершается до destructor Resource(2). Destructor,
который безусловно создаёт новое значение, может сделать cleanup незавершающимся.

## Требования к реализации

Нужны compile-pass/fail проверки прямой записи, alias и helper call; positive
случаи изменения/замены поля; журналы body/field cleanup, panic и повторной panic.
Ограничение или lifecycle protocol должны присутствовать в entry effects и
metadata, а не только в локальной синтаксической проверке.

Пока source destructors остаются отклоняемыми. Runtime cleanup/slot primitives
сами по себе не считаются реализацией выбранной здесь семантики.
