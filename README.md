# Nether

Nether компилируется в Rust. Собственные frontend, type/ownership checker и MIR
сохраняют модель памяти языка; Rust backend генерирует source, который собирает
обычный `rustc`. Прямого LLVM backend и зависимости от Clang больше нет.

[Модель памяти NMM](docs/core-memory-managment.md) ·
[Новая концепция](docs/0.1/rust-backend.md) · [Спецификация](docs/main.md) ·
[План MVP](docs/0.1/mvp_plan.md) · [Roadmap](docs/0.1/roadmap.md) · [Состояние реализации](docs/0.1/status.md).

NMM от 2026-10-21 — основополагающий контракт памяти с приоритетом над прежними
правилами. Описанная ниже реализация ещё требует перехода на новую модель;
план перехода и различия закреплены в документе NMM.

Работают значения, generics, inline aggregates, local views, owned/borrowed entries,
struct methods, function values с owned/borrowed-сигнатурами (включая non-Copy) и source destructors с
детерминированным cleanup, включая panic и частичное создание aggregates.
Работают class/new/constructors, рекурсивные классы, методы и class views:
move сохраняет identity, Copy создаёт отдельный объект, Rust хранит payload в Box.
Возврат views сохраняет lifetime и права доступа через calls, generics и рекурсию.
Structs, tuples, arrays и enum payload хранят readonly views; их зависимости
сохраняются при передаче и возврате через функции, generics и imports. Mutable
borrowed-параметры могут переназначать эти поля со статической проверкой времени
жизни новых targets, включая panic paths. Проверки и Rust backend исключают
доказанно недостижимые ветви локальных enum. Режимы локального storage учитывают
поздние присваивания, aliases и требования параметров/результатов; массивы согласуют
mode всех элементов. Borrowed class fields, destructors таких оболочек и borrowed
function pointers реализованы. Статические interfaces поддерживают явные реализации,
conjunction constraints и проверку generic bodies по объявленным ограничениям.
Пример — [interfaces](examples/interfaces/main.nr). Private members доступны только
внутри объявляющего class/struct; generic method pointers поддерживают явные
type arguments ([пример](examples/private_members/main.nr)). Проверяются hierarchy и
override declarations; root-class virtual methods используют static calls
([пример](examples/virtual_methods/main.nr)). Далее — embedded base/super и inherited methods,
static upcast и dyn dispatch. Производительность на уровне эквивалентного Rust — цель проверки,
а не уже доказанное свойство всего языка.

## Использование

Нужен Rust 1.86+ и системный linker, используемый rustc. Сторонних crates нет.
`NETHER_RUSTC` задаёт путь к compiler executable.

```sh
cargo run -p nether -- check examples/arithmetic/main.nr
cargo run -p nether -- eval examples/arithmetic/main.nr
cargo run -p nether -- emit-rust examples/arithmetic/main.nr > /tmp/arithmetic.rs
cargo run -p nether -- build examples/arithmetic/main.nr -o /tmp/arithmetic
cargo run -p nether -- build examples/arithmetic/main.nr --release --emit=object -o /tmp/arithmetic.o
```

Сборка выполняется для 64-bit host. `--release` включает Rust optimization level 3
и wrapping arithmetic; `--overflow-checks=on|off` переопределяет арифметику Nether.
`eval` остаётся ограниченным эталонным интерпретатором.

## Проверки

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo test --workspace --locked --offline
cargo test --workspace --release --locked --offline
```

Native suite собирает и запускает Rust-программы на host при O0/O3 и сравнивает
их с HIR/MIR interpreters. Docker и отдельный LLVM toolchain не нужны.

## Структура

- `compiler/semantics/` — числовые, layout и storage contracts.
- `compiler/frontend/` — lexer, parser, diagnostics и modules.
- `compiler/core/` — HIR, MIR, ownership checker, interpreters и Rust source backend.
- `cli/` — `syntax`, `check`, `eval`, `emit-rust`, `build`.
- `runtime/` — инфраструктура allocation и deterministic cleanup.
- `stdlib/`, `examples/`, `docs/` — библиотека, примеры и контракты языка.
