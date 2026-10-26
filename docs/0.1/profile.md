# Nether 0.1 — профиль языка

Редакция 2026-10-25. Версия компилятора — 0.1.0; модель памяти — `nmm-0.1`.
Это новая базовая линия 0.1. Старые профили выбора move по последнему use
и owned mutable entries отменены. NMM — единственная модель памяти.

## Нормативные документы

- [NMM](../core-memory-managment.md): ownership, доступ, lifetime, cleanup.
- [Unsafe](../unsafe.md): граница проверки и обязательства автора.
- [Основная спецификация](../main.md): синтаксис, типы, арифметика и модули.
- [Состояние реализации](status.md): что действительно исполняет компилятор.

Целевой контракт языка не означает наличие всех возможностей в текущей сборке.
Нереализованная операция отклоняется, включая использование внутри `unsafe`.

## Память

Для non-Copy существующего места `let b = a` создаёт R alias, `var b = a` —
W reborrow при наличии W у источника. Последний use завершает borrow, но не
создаёт move и не уничтожает owner. `let` не повышается до `var` через передачу O.
Copy создаёт независимое значение; borrowed компоненты сохраняют lifetime.

Обычный non-Copy параметр предоставляет R; `var` — W borrow. Выведенный
consuming-effect требует O от caller и не даёт W. Alias в consuming API —
`NMM_NO_OWNER_TOKEN`. API, хранящее borrowed descriptor, должно выводиться как
непотребляющее, например через локальный alias параметра. Нельзя выбирать
borrowed ABI для обхода объявленного consuming-effect.

Owning-рёбра ацикличны. Каждый owned lifetime имеет одного владельца и один
cleanup. Регионы — способ размещения; они не разрешают owning-циклы.
Нет GC, strong ARC, runtime borrow checker и скрытого clone.

## Основной синтаксис

```text
function = ["private" | "public"], ["unsafe"], "fn", name,
           [generics], "(", [parameters], ")", ":", type, block;
parameter = ["var"], name, ":", type;
receiver = ["var"], "this";
statement = ("let" | "var"), pattern, [":", type], "=", expression
          | "return", [expression]
          | "break" | "continue"
          | "unsafe", block
          | expression;
```

Параметры вызова не имеют префиксов `move`, `&`, `&mut` или `var`.
`unsafe fn` вводит обязательство caller; её тело всё равно требует явных
`unsafe { ... }` для unsafe-вызовов. `main` обязана быть safe.
`unsafe`-блок пока является инструкцией; его return/break/continue и cleanup
имеют обычную семантику. Полный контракт raw операций — [unsafe.md](../unsafe.md).

Типы: `()`, bool, char, целые i8/i16/i32/i64/isize/u8/u16/u32/u64/usize,
f32/f64, tuple, `{T; N}`, struct, class, enum, function values `(T) => U`.
Поддерживаются type/const generics, nominal interfaces и constraints.
Raw типы `*T`/`*var T` зарезервированы спецификацией, но пока отклоняются checker.

Class создаётся через `new`, struct — через literal. Receiver задаётся явно:
`this` — R, `var this` — W. Деструктор синхронный; body выполняется до полей.
Private member доступен только из объявляющего class/struct.

Числовые правила, приоритеты, literals и pattern syntax определены в
[main.md](../main.md) и [syntax-cases.md](syntax-cases.md). Целые операции
проверяют overflow по выбранной политике; CLI позволяет checked/wrapping.
Null и неинициализированная память не являются safe значениями.

## Статус возможностей

| Возможность | Текущая реализация |
| --- | --- |
| NMM aliases, reborrow, readonly, consuming O | Реализовано для поддержанного subset; приёмка в memory-cases |
| Cleanup, borrowed storage/results, generics, function values | Реализованы; консервативные отказы допустимы |
| `unsafe fn`, statement `unsafe` block, direct/generic/method calls | Реализованы; safe NMM проверки сохраняются |
| Unsafe function pointers | `unsafe (T) => R`, safety сохраняется; indirect call требует unsafe block |
| Statement `loop` | break/continue, nested loops и cleanup; `break value` пока не поддержан |
| Raw pointers | Scalar address/read/write, element offset и writable → readonly cast; явный unsafe для операций с целью |
| Allocation/storage | Owned scalar/Unit subset через `core/intrinsic`; arbitrary T и borrowed mode не завершены |
| FFI | Контракт определён; реализация не завершена |
| Inheritance/upcast/dyn | Проверка части declarations; исполнение derived objects не завершено |
| Weak, capturing closures, channels/async | Целевой NMM контракт; source implementation не завершена |
| Пользовательские Vec/String | Не завершены |

Эта таблица описывает поддержку, а не альтернативные модели языка.
Выпуск требует критериев [плана 0.1](mvp_plan.md); номер версии сам по себе
не означает стабильность, production readiness или доказанную soundness.
