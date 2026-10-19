# Состояние реализации Nether 0.1

## Переход на Rust backend — 2026-10-13

Утверждена [новая концепция](rust-backend.md): Nether генерирует Rust; прямой LLVM
backend отключён и удалён. Обычный rustc разрешён. Внутренний unsafe допустим для
сохранения проверенных Nether ownership, views и cleanup contracts. Цель скорости —
сопоставимый вручную написанный Rust при той же модели памяти; это ещё требует
performance acceptance, универсальная эквивалентность производительности не доказана.

## Реализовано

- Rust workspace без сторонних dependencies; frontend, lexer/parser, diagnostics,
  modules/imports/privacy и разрешение имён.
- Числовые контракты, constants, typed HIR и CFG MIR с places, initialization,
  partial moves, local views, reborrow, lifetime и uniqueness checks.
- Inline structs, enums, arrays, tuples; type/const generics и abstract body audit;
  static/instance struct methods; owned/borrowed entry specializations.
- Function values с фиксированными owned/borrowed signatures (включая non-Copy), generic specialization по expected type,
  higher-order functions и indirect calls. Capturing closures вне scope 0.1.
- Два эталонных интерпретатора HIR/MIR. MIR cleanup scopes и порядок последних
  успешных инициализаций. Effects whole replacement/field writes через aliases
  и recursive calls — основа finalizing capability.
- Rust source emission для поддержанного value subset: typed locals, raw views,
  arithmetic checks, enum payloads, calls, control flow и panic source positions.
- CLI `syntax`, `check`, `eval`, `emit-rust`, `build`; rustc host executable/object,
  release optimization и независимое управление overflow checks.
- Native tests перенесены на host rustc O0/O3 и сравнение с обоими interpreters.
  Clang, Docker и Linux VM больше не требуются для portable subset.

## Ограничения и следующий этап

Source classes/new/constructors реализованы: payload struct хранится в Box,
class view хранит identity, move передаёт pointer, Copy создаёт новый объект.
Статические interface implementations и constrained generic dispatch реализованы в C4.1.
Class inheritance, static upcast и dyn dispatch остаются пунктами C4.2–C4.4.
Struct concrete inheritance не вводится.

Не завершены inheritance/dyn dispatch, FFI/raw storage/syscalls,
groups/weak и пользовательские Vec/String. Принятые memory decisions сохраняются.
Backend migration не заменяет реализацию этих пунктов и не закрывает весь roadmap.

Baseline Rust lowering объединяет straight-line MIR blocks и выражает оставшийся CFG через typed block dispatch. Требуются проверка
optimized code и performance benchmarks против handwritten Rust; при обнаружении
остаточного dispatch нужно structured lowering. Нет heap interpreter/value boxing
в generated code, но отсутствие регрессий скорости ещё не доказано.

Предыдущие результаты C/LLVM tests относятся к удалённому backend и не являются
доказательством корректности Rust backend. Актуальные Rust-проверки: workspace Debug, 360 native сравнений (90 программ ×
checked/wrapping × O0/O3), четыре группы object compilation и CLI tests прошли.
Rust runtime tests прошли, включая diagnostic mode и subprocess abort checks.
Workspace Release, Clippy со всеми features без warnings, formatting и проверка
относительных ссылок документации также прошли.

Runtime primitives перенесены в crate `nether-runtime`: allocation, selected/root
cleanup, replacement order и finalizing state. Исторические C cases сохранены
только в `runtime/tests/reference/` и не входят в build. Rust representation test
проверяет static/dyn class upcast без смены allocation identity; это не source
class conformance.

Performance probe `arithmetic_loop_against_handwritten_rust`: на текущем macOS
ARM host медиана 9 измерений составила 3,828 ms Nether против 3,610 ms Rust
(отношение 1,060; wrapping arithmetic, rustc O3, 2 000 000 iterations). Это один
сценарий с шумом измерения, не подтверждение отсутствия регрессий во всех программах.
Запуск: `cargo test -p nether-core --test performance --offline -- --ignored --nocapture`.

## Реализация плана MVP

Основной трек — [mvp_plan.md](mvp_plan.md). C1 реализован: source destructors,
type/const generic specialization, Copy/duplicate/partial-move restrictions и
transitive finalizing effects. Cleanup подключён к обоим interpreters и generated
Rust: явные lifecycle flags, порядок roots после replacement, body перед полями,
active enum payload, return staging, partial construction и panic continuation.
Повторная panic вызывает abort в native программе. Differential tests сравнивают
результаты и последовательности вызовов destructor; native cases выполняются при O0/O3.

C2 реализован 2026-10-15: source class/new/constructors, recursive types,
type/const generic classes, readonly/mutable methods и class views. Constructor
initialization проверяется по typed MIR и alias provenance. Незавершённый объект
очищает только завершённые поля; собственный destructor не вызывается.
Отдельные identity создаются при Copy, в том числе внутри inline aggregates.
`==`, `!=` и `same_identity` сравнивают class identity, не копируя operands.
Move и возврат non-Copy не создают нового allocation. Generated Rust не использует Rc/ARC.
Native instrumentation при O0/O3 проверяет allocation/free balance и identity;
HIR/MIR сравниваются по результатам и порядку destructor calls.

Class views пока консервативно зависят от исходного владельца. Fallible OOM
обработка относится к C5: текущая Box allocation использует стандартную политику Rust.
C0–C3 закрыты. C4–C9 и полный MVP ещё не завершены.


В C3 реализованы borrowed returns (C3.1), recursive entry result/effect fixed point
(C3.3) и origin/conflict/later-use diagnostics (C3.5). Capability и зависимость от
аргументов сохраняются через generics, imports, if/match и взаимную рекурсию.
Возврат view работает в обоих interpreters и generated Rust; существующие cleanup
и finalizing contracts применяются к returned aliases. Copy-results остаются owned.
Non-Copy owned/borrowed function values, unbound receivers и callable contracts
подключены. Parameter/result/component origins и may-write effects сохраняются
через higher-order calls, returned pointers и выбор совместимых functions.
Пример: `examples/borrowed_results/main.nr` (результат 16).

В C3.2 работают readonly поля struct/tuple/enum и элементы fixed arrays. Static
storage modes проходят через argument/result specialization функций, методов,
generics, imports/reexports и recursive entries. Перемещение оболочки переносит
descriptors; borrowed target не копируется и не уничтожается. Component provenance
сохраняет lifetime нескольких внешних targets, включая nested wrappers. Возврат
view локального owner внутри wrapper запрещён. Native tests проверяют cleanup
и partial/panic paths. Пример: `examples/borrowed_storage/main.nr` (результат 24).

Mutable borrowed aggregate arguments уже имеют статические may-write contracts:
переназначение полей проверяется через методы, generics, imports, рекурсию и panic.
Локальные/owned targets callee не могут сохраниться в caller storage. После вызова
зависимости объединяются с прежними; гарантированный overwrite пока не сокращает
lifetime. Пример: `examples/borrowed_storage_write/main.nr` (результат 10).

C3.2 закрыт, включая class fields и пользовательские destructors таких оболочек.
Constructor, методы и destructor используют согласованный physical class layout.
Implicit destructor reads продлевают lifetime targets до cleanup; анализ запрещает
разрушение target раньше holder, включая aliases, joins и panic paths.
Известные локальные enum variants уже уточняются через construction/move/root
replacement, joins и loops. Доказанно недостижимый payload не участвует в loans,
initialization, result/effect inference и generated Rust. Source-ветви всё равно
проверяются по типам. Calls и projected/aliased writes сбрасывают знание variant;
уточнение arbitrary enum places остаётся консервативным. Вывод storage modes через
параметры, mutable aliases, returned values и if/match реализован. Для named locals будущие присваивания
уточняют initializer representation до fixed point, включая nested fields и
связанные wrapper bindings. Array mode согласуется по всем элементам; runtime
вычисляет каждый initializer один раз. Пример: `examples/storage_modes/main.nr` (11).
Пример: `examples/enum_refinement/main.nr` (результат 3).

Приёмка C3 проверяет workspace Debug/Release, native O0/O3 и точное совпадение
HIR/MIR результатов и drop traces. Отдельные сценарии покрывают borrowed class
payload, destructor reads, root replacement, panic, function pointers, higher-order
calls, generic imports/reexports и отрицательные lifetime/ABI случаи.
Новые примеры: `examples/borrowed_classes/main.nr` и
`examples/borrowed_callables/main.nr`, оба возвращают 14.

Консервативные ограничения: may-write call не сокращает старые lifetime dependencies;
произвольные enum places/call discriminants не уточняются; cyclic reborrowing
диагностируется. Owning-group cycles/dependency scheduling относятся к C7.
Производительность всего языка ещё требует отдельной приёмки C9.

Итог проверки 2026-10-20: по 159 tests прошли в workspace Debug и Release,
0 failures; один performance probe остаётся opt-in/ignored. Native acceptance:
109 borrowed-storage, 29 borrowed-destructor/class/callable и 19 borrowed-result
программ при O0/O3, 53 lifecycle/indirect-call программы при O0/O3 и прежние
360 baseline сравнений. Clippy all-targets/all-features с `-D warnings`, formatting,
whitespace и локальные ссылки документации проверены.


## C4.1 — статические interfaces, 2026-10-20

Явные nominal implementations class/struct проверяются по interface signatures,
включая public access, receiver capability, generic methods, type/const arguments
и defaults. Conjunction `T: I & J & Copy` даёт только заявленные возможности;
matching concrete methods без `: I` не делают тип реализацией interface.
Abstract generic audit при специализации не допускает concrete extra methods
или неуказанный Copy. Зависимые constraints поддержаны и у method generics.

Static methods, function values, imports/reexports и borrowed method results
используют прежние concrete entries. Interface не добавляет runtime vtable,
allocation, tag, refcount или clone. Method const expressions должны совпадать
символически после переименования параметров; алгебраического solver нет.
Пример — [interfaces](../../examples/interfaces/main.nr), результат 8.
C4.2–C4.4 (inheritance/upcast/dyn) остаются открытыми.


Проверка C4.1: workspace Debug/Release — по 163 tests, 0 failures,
один performance probe opt-in/ignored. Все 23 interface-программы прошли
HIR/MIR comparison и native O0/O3 с проверкой результатов и порядка destructor
calls; отдельно проверены imports/reexports и отрицательные constraints.
Clippy all-targets/all-features с `-D warnings`, formatting, whitespace и
локальные Markdown links проверены. Пример interfaces даёт 8 в eval и native release.


## C4.2a — lexical member access, 2026-10-20

Исправлен прежний module-level доступ к private fields/methods: теперь доступ
ограничен объявляющим class/struct, как требует §91 спецификации. Проверка
работает в constructors/destructors, generic/borrowed entries, aliases,
literal construction, patterns и method function values. Ошибка указывает доступ
и private declaration. Generic/imported helpers не получают привилегии caller.
Top-level private functions сохраняют доступ внутри своего модуля.

Method pointers поддерживают явные owner/method type arguments. Публичный factory
может вернуть private callable; caller получает capability на indirect call,
не право именовать private method напрямую. Проверка откладывает callers до
готовности specialization возвращаемой функции, сохраняя function IDs в HIR/MIR.
Пример — [private_members](../../examples/private_members/main.nr), результат 7.
C4.2 ещё не закрыт: hierarchy, embedded base, super и override остаются следующей работой.


Проверка C4.2a: workspace Debug/Release — по 167 tests, 0 failures,
один performance probe opt-in/ignored. 13 новых программ прошли HIR/MIR
comparison и native O0/O3 с точным порядком destructor calls; отдельно проверены
отрицательные access cases и imports/reexports с declaration notes.
Clippy all-targets/all-features с `-D warnings`, formatting, whitespace и локальные
Markdown links проверены. Пример private_members возвращает 7 в eval и native release.


## C4.2b/c — hierarchy и override declarations, 2026-10-20

Parser сохраняет virtual/override в AST. До построения typed entries проверяются
single class-parent, его положение перед interfaces, cycles, parent argument
count/kinds/defaults, member hiding и override chains. Override доступен только
для public class instance method и сохраняет receiver/parameter/result contracts,
method generics/constraints/defaults после подстановки parent arguments.
Method binders freshen при каждом inheritance edge; module constants сохраняют
lexical meaning, в том числе при совпадении имён у descendant generics.
Закрытые const expressions вычисляются, symbolic expressions сравниваются
структурно после переименования параметров. Diagnostics указывают inherited
method declaration и сохраняются через imports/reexports.

Root-class virtual methods компилируются статически; проверены mutable receiver,
interfaces, unbound/generic function pointers, borrowed results и destructor calls.
Пример — [virtual_methods](../../examples/virtual_methods/main.nr), результат 8.
Наличие корректного графа не означает готовое выполнение derived objects:
embedded base, super, inherited body lowering, derived/base cleanup и upcast
ещё не реализованы. Nongeneric concrete inheritance и создание generic derived
object по-прежнему отклоняются на границе lowering с E0900. C4.2 остаётся открытым.
Declaration type expressions сравниваются консервативно после подстановки;
нормализация implicit nested nominal defaults и typed base entry contracts
остаётся частью следующего этапа.

Проверка declaration layer C4.2b/c: workspace Debug/Release — по 174 tests,
0 failures; один performance probe opt-in/ignored. Проверены 19 положительных
и 35 отрицательных hierarchy cases, imports/reexports с source notes и семь
новых root-virtual программ в HIR/MIR и native O0/O3 с точным cleanup trace.
Clippy all-targets/all-features с `-D warnings`, formatting, whitespace и
локальные Markdown links проверены. Пример virtual_methods возвращает 8
в eval и native release. Derived-object runtime acceptance остаётся открытым.
