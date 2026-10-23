# Nether

> **Статус: proof of work.** Язык находится на стадии доказательства
> концепции. Синтаксис, семантика и компилятор нестабильны и могут меняться
> без обратной совместимости. Не используйте Nether в продакшене.

Nether — компилируемый язык со статической типизацией, цель которого —
производительность уровня Rust при простом синтаксисе. Компилятор
(frontend, type/ownership checker, MIR) написан на Rust и генерирует Rust-код,
который собирает обычный `rustc`.

## Основные идеи

- **Без GC и без скрытого подсчёта ссылок.** Память освобождается детерминированно.
- **Ссылочная семантика для non-Copy значений.** `let b = a` не копирует объект,
  а создаёт ещё одну ссылку на тот же. Ключевых слов `move`, `&`, `&mut` и явных
  lifetime нет.
- **Права доступа выводятся компилятором.** Ошибки памяти ловятся при компиляции.
- **Классы, структуры, enum, generics, interfaces, деструкторы** — в составе языка.
- **Замыкания и async** описаны в спецификации и ещё не реализованы
  (см. [замыкания](docs/core-memory-managment.md), [async](docs/async-await-runtime.md)).

## Пример

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

    var first = owner       // изменяемое заимствование
    first.increment()
    var second = first
    advance(second)
    first.increment()
    owner.increment()

    return owner.read() - initial // 4
}
```

Создан один `Counter`, копий нет, объект уничтожается один раз при выходе
из `main`.

## Модель управления памятью (NMM)

Полная спецификация — [docs/core-memory-managment.md](docs/core-memory-managment.md).
Кратко:

- **Три независимых понятия:** **O** (ownership — обязанность завершить
  жизнь объекта), **R** (право читать), **W** (эксклюзивное право писать).
  Владение не даёт права записи: `O ≠ W`.
- **Один владелец.** У каждого живого объекта ровно один owner-token. Ссылок
  может быть много, они не создают новых владельцев и не клонируют объект.
- **`let` — чтение, `var` — запись.** Из readonly пути нельзя получить `var`
  (`let a = ...; var b = a` — ошибка), даже если `a` больше не используется.
- **Заимствования.** Допустимы несколько читателей или один писатель.
  `var b = a` — последовательное перезаимствование: пока жив `b`, `a`
  приостановлен. Заимствование заканчивается на последнем использовании.
- **Передача владения выводится автоматически** (возврат, сохранение в поле
  или контейнер, отправка по каналу). Пользовательского `move` нет.
- **Детерминированный cleanup.** Деструкторы вызываются в известном порядке
  при выходе из области, замене значения, panic. Последнее чтение ссылки
  объект не уничтожает.
- **Циклы.** Owning-связи образуют дерево, циклы запрещены. Невладеющие связи
  выражаются полем `weak`; результат доступа через него — `Option`.
- **Потоки и задачи.** Изменяемые данные передаются между изолированными
  контекстами по каналам с передачей владения.
- **Если безопасность нельзя доказать статически — программа отклоняется.**
  GC, strong ARC, runtime borrow checker и скрытый clone не добавляются.

## Состояние проекта

Реализация частично соответствует прежней модели памяти. Переход на NMM
описан в [документе NMM](docs/core-memory-managment.md), раздел 26.
Производительность уровня Rust — цель, а не доказанное свойство.

Артефакты стадии proof of work собираются в каталоге [proof-of-work](proof-of-work/).

## Использование

Нужен Rust 1.86+ и системный linker, используемый rustc. Сторонних crates нет.

```sh
cargo run -p nether -- check examples/arithmetic/main.nr
cargo run -p nether -- eval examples/arithmetic/main.nr
cargo run -p nether -- emit-rust examples/arithmetic/main.nr > /tmp/arithmetic.rs
cargo run -p nether -- build examples/arithmetic/main.nr -o /tmp/arithmetic
```

Проверки:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo test --workspace --locked --offline
```

## Структура

- `compiler/semantics/` — числовые, layout и storage contracts.
- `compiler/frontend/` — lexer, parser, diagnostics, modules.
- `compiler/core/` — HIR, MIR, ownership checker, интерпретаторы, Rust backend.
- `cli/` — команды `check`, `eval`, `emit-rust`, `build`.
- `runtime/` — allocation и deterministic cleanup.
- `stdlib/`, `examples/` — библиотека и примеры.
- `proof-of-work/` — материалы стадии proof of work.
- `docs/` — спецификации.

## Документация

[Модель памяти NMM](docs/core-memory-managment.md) ·
[Async runtime](docs/async-await-runtime.md) ·
[Спецификация языка](docs/main.md) ·
[Rust backend](docs/0.1/rust-backend.md) ·
[План MVP](docs/0.1/mvp_plan.md) ·
[Roadmap](docs/0.1/roadmap.md) ·
[Состояние реализации](docs/0.1/status.md)
