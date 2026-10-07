# Nether

Новая реализация версии 0.1 по [roadmap](docs/0.1/roadmap.md).
Работает первый компилятор значений: frontend, проверка типов, эталонный
интерпретатор и LLVM backend для `x86_64-unknown-linux-gnu`.
Локальные inline views поддерживают readonly/mutable доступ, reborrow и проверки lifetime.
Работают non-Copy inline aggregates, owned/borrowed варианты вызовов и instance
методы structs. Runtime содержит allocator и обработку panic.
Полная модель владения, runtime ресурсов и стандартная библиотека ещё не готовы;
зависимые от них формы отклоняются. Полный статус — [здесь](docs/0.1/status.md).

## Документация

- [Спецификация языка](docs/main.md).
- [Roadmap 0.1](docs/0.1/roadmap.md).
- [Профиль синтаксиса и числовой семантики](docs/0.1/profile.md).
- [Состояние реализации и ограничения](docs/0.1/status.md).
- [Layout](docs/0.1/layout.md), [ABI](docs/0.1/abi.md), [storage contracts](docs/0.1/storage-contracts.md).

## Запуск

Нужен Rust 1.86+. Для object emission и LLVM tests нужен LLVM/Clang 18.
`NETHER_CLANG` задаёт путь к clang; сторонних Rust dependencies нет.

```sh
cargo run -p nether -- check examples/arithmetic/main.nr
cargo run -p nether -- eval examples/arithmetic/main.nr
NETHER_CLANG=clang-18 cargo run -p nether -- build examples/arithmetic/main.nr --emit=object -o /tmp/arithmetic.o
```

На Linux сборка executable выполняется без `--emit=object`. `NETHER_LINKER`
задаёт C compiler driver (по умолчанию `cc`), который связывает объект,
встроенный C runtime и libm.
На других host нужен настроенный Linux linker/sysroot; emitted object остаётся
Linux x86-64, независимо от host. `--release` включает `-O2` и wrapping arithmetic;
`--overflow-checks=on|off` явно переопределяет арифметический режим.
`eval` — ограниченное эталонное выполнение, не запуск backend-программы.

## Проверка

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
NETHER_CLANG=clang-18 cargo test --workspace --locked --offline
NETHER_CLANG=clang-18 cargo test --workspace --release --locked --offline
NETHER_CLANG=clang-18 cargo test -p nether-core --test native --locked --offline -- --ignored --nocapture
```

Последняя команда требует Linux x86-64. На другом host тест поддерживает
`NETHER_LINUX_DOCKER=1` и образ `gcc:14-bookworm` через Docker. CI явно запускает
native suite: пропуск этого теста в переносимой suite не считается его прохождением.

## Структура

- `compiler/semantics/` — integer/layout oracle и solver режимов storage.
- `compiler/frontend/` — исходники, диагностика, lexer, parser и модули.
- `compiler/core/` — типизированный HIR, CFG MIR, generics, checker, интерпретаторы и LLVM lowering.
- `cli/` — команды `syntax`, `check`, `eval`, `emit-llvm`, `build`.
- `runtime/` — allocation, panic reporting и диагностический allocation log.
- `stdlib/` — место для стандартной библиотеки следующих этапов.
- `examples/` — исполняемые примеры.
- `docs/` — спецификация, контракты и план.
