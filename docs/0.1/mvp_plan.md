# Nether MVP: исполняемый план

Обновлён 2026-10-20. Это основной трек реализации; [roadmap](roadmap.md) содержит
полные требования, [Rust backend](rust-backend.md) — утверждённую архитектуру.
Чекпоинт закрывается только работающим source API и проверками всех затронутых
слоёв. Runtime primitive или положительный parser test не закрывает source feature.

## Зафиксированные решения

Rust source → обычный rustc; внутренний проверенный unsafe разрешён. Нет GC/ARC,
скрытых clone и runtime выбора ownership. Class → payload + trait + Box;
static specialization по возможности, dyn при необходимости. Upcast сохраняет
identity. Mutable inline view заменяет target через `*view`; class view не
заменяет owning slot. Roots очищаются обратно последней успешной инициализации
в исходном scope. Finalizing target нельзя заменить даже через aliases/calls.
Container owned/readonly-view representations специализируются статически.

## C0 — работающая базовая компиляция

- [x] Lexer/parser, source diagnostics, imports/exports/privacy.
- [x] Примитивы, const evaluation, arithmetic/casts, control flow и recursion.
- [x] Inline structs/enums/tuples/arrays, patterns и exhaustiveness.
- [x] Type/const generics, struct methods, function values с Copy contracts.
- [x] HIR/MIR interpreters и Rust source/backend CLI.
- [x] Move/partial initialization и local view checking для текущего subset.
- [x] 90 native cases × checked/wrapping × O0/O3; Debug/Release/Clippy.

## C1 — source destructors и deterministic cleanup

- [x] C1.1: lifecycle declarations, typed destructor entries, запрет Copy + destructor.
- [x] C1.2: transitive finalizing capability, включая aliases и recursive helpers.
- [x] C1.3: type-directed drop plan: body, поля, arrays/tuples, active enum payload.
- [x] C1.4: explicit initialization/drop state, partial moves, replacement через views.
- [x] C1.5: cleanup normal exit/return/break/continue, temporary и return staging.
- [x] C1.6: construction failure, panic cleanup, continuation после первой panic, double panic abort.
- [x] C1.7: одинаковые observable effects в обоих interpreters и generated Rust.

Приёмка: ресурс уничтожается ровно один раз; replacement меняет root order;
поля не переставляют root; partial failure не читает неинициализированную память.

## C2 — class identity и construction

- [x] C2.1: class types, new, constructors, heap identity и recursive class types.
- [x] C2.2: constructor initialization checking и partial construction cleanup.
- [x] C2.3: move сохраняет allocation/identity; Copy class создаёт независимую identity.
- [x] C2.4: readonly/mutable methods и class views, запрет whole slot replacement через view.

Приёмка: создание/возврат/замена class работают без утечек, double free и скрытого clone.

## C3 — полные borrowed contracts

- [x] C3.1: borrowed returns с provenance и lifetime через несколько вызовов.
- [x] C3.2: borrowed поля/aggregate representations и static storage modes.
- [x] C3.3: fixed-point entry effects на recursive call groups.
- [x] C3.4: non-Copy/borrowed function-value contracts.
- [x] C3.5: диагностики origin/conflict/последующего use.

Приёмка: escaped view, invalidation, readonly promotion и alias conflicts
отвергаются в local, generic и imported code одинаково.

## C4 — interfaces и class inheritance

- [x] C4.1: interface constraints, conjunction, generic body checking, static dispatch.
- [ ] C4.2: single class inheritance, embedded base, override/access rules.
- [ ] C4.3: B вместо A со static specialization; transitive upcast без allocation.
- [ ] C4.4: dyn traits и heterogeneous storage, receiver contracts и concrete drop.

Разбиение C4.2:

- [x] C4.2a: lexical private access внутри объявляющего class/struct;
  generic/borrowed entries, constructors/destructors, function values и imports.
- [ ] C4.2b: single-parent hierarchy, embedded base и `super(...)`, включая
  initialization и cleanup при частичном создании производного объекта.
- [ ] C4.2c: inherited methods, virtual/override signatures и запрет доступа
  производного class к private-членам base; native/differential acceptance.

Готовая часть C4.2b/c:

- [x] Declaration hierarchy: один class-parent первым в списке, запрет cycles,
  invalid parents, struct inheritance и неоднозначного member hiding.
- [x] Parser/AST для virtual/override; проверка цепочек override, receiver/access,
  parameter/result signatures, method type/const generics, constraints и defaults.
- [x] Capture-free подстановка parent arguments/defaults через несколько уровней;
  lexical module constants не захватываются generic-параметрами потомка.
- [x] Root-class virtual methods выполняются через обычные static entries;
  graph diagnostics работают до lowering и через imports/reexports.

Приёмка: статический и динамический путь дают одинаковый результат и identity;
вызов derived destructor и base cleanup происходит в определённом порядке.

## C5 — storage primitives и unsafe

- [ ] C5.1: raw pointers/address/deref/arithmetic/casts и unsafe context checking.
- [ ] C5.2: layout, allocation/deallocation, typed initialize/move/drop.
- [ ] C5.3: safe views из storage с доказанными bounds/provenance/invalidation.
- [ ] C5.4: RawBuffer<T>, initialized prefix, grow через allocate/move/free.
- [ ] C5.5: OOM, alignment, overlap, ZST, size overflow и diagnostic accounting.

Приёмка: буфер ресурсов растёт и освобождается; views блокируют несовместимый grow/remove.

## C6 — библиотечная и платформенная граница

- [ ] C6.1: extern C, repr(C) POD, callbacks, linking и panic boundary.
- [ ] C6.2: Linux x86-64 direct syscall adapter и checked platform wrappers.
- [ ] C6.3: metadata signatures/generic bodies/ownership/effects, version/target checks.
- [ ] C6.4: separate library compilation; client checking без обычных function bodies.

Приёмка: C callback и прямой syscall выполняются; несовместимая metadata отвергается.

## C7 — связанные структуры

- [ ] C7.1: logical owning groups, cycles из views, external lifetime restrictions.
- [ ] C7.2: destructor dependencies и проверяемый cleanup order.
- [ ] C7.3: weak class fields, readonly lookup, domain/slot/generation, ABA prevention.
- [ ] C7.4: independent removal не удерживает удалённый объект до конца группы.

Приёмка: циклические связи очищаются без tracing/refcount; stale weak не оживает.

## C8 — стандартная библиотека на Nether

- [ ] C8.1: Option/Result и numeric/layout utilities.
- [ ] C8.2: Vec<T> — push/pop/get/get_mut/replace/remove/clear/grow/destructor.
- [ ] C8.3: String — UTF-8 storage, append, length и borrowed str.
- [ ] C8.4: allocator/file wrappers, explicit finish/close и fallback cleanup.
- [ ] C8.5: второй контейнер и структура с рекурсивными связями.
- [ ] C8.6: многомодульное приложение: read file → collections → write file.

Приёмка: библиотека использует доступные пользователю primitives; compiler
не распознаёт имена Vec/String специально. Non-Copy ресурсы корректны во всех операциях.

## C9 — выпуск и производительность

- [ ] C9.1: вся feature matrix имеет positive/negative/runtime conformance.
- [ ] C9.2: инструментальные memory checks generated programs и long-running workloads.
- [ ] C9.3: benchmarks против эквивалентного Rust: CPU, allocations, code size.
- [ ] C9.4: structured lowering там, где остаточный CFG dispatch мешает производительности.
- [ ] C9.5: воспроизводимая сборка, docs/examples/CI и все memory acceptance cases.

MVP завершён только после C1–C9. Async, threads, capturing closures, for/iterators,
arenas/pools и прочие исключённые возможности не добавляются в этот план.

## Текущая работа

C0–C3 реализованы. Borrowed contracts включают class payloads, пользовательские
destructors оболочек, межпроцедурный вывод storage modes и function pointers.
Layout, ABI, result origins и may-write effects выбираются до исполнения.
C4.1 реализует статические interfaces и проверку generic bodies по constraints.
C4.2 начат: private members доступны только внутри объявляющего class/struct.
Далее — embedded base, super и override; затем static upcast и dyn dispatch.
C4.2–C9 остаются открытыми; весь MVP ещё не завершён.

Результат C1:

- [x] Typed destructor entries, включая type/const generic specialization и body audit.
- [x] Copy/duplicate/partial-move restrictions и transitive finalizing effects.
- [x] Type-directed cleanup: body, declaration-order fields, indexed elements, active enum.
- [x] Незавершённые named aggregates сохраняют written evaluation order, но очищают
  завершённые поля в declaration order без вызова незавершённого parent destructor.
- [x] HIR/MIR execution и typed generated Rust drop glue; moves, partial moves,
  replacement через views, return staging, break/continue, first/double panic.
- [x] Differential source tests и native resource cases при O0/O3; отдельная проверка abort.

Generated lifecycle flags — initialization state, а не runtime выбор ownership.
Resource-free программы не получают lifecycle context. Borrowed aggregates
проверяются в C3.2, storage primitives — отдельно в C5.

Результат C2:

- [x] Source class/new и typed constructor entries, type/const generic instances.
- [x] Отдельный nominal handle с pointer layout; recursive payload registry.
- [x] Проверка definite initialization на успешных путях typed MIR с provenance
  aliases; все поля готовы перед return, чтением whole this или внешним вызовом.
- [x] Rust Box с MaybeUninit payload и явными flags; partial failure очищает
  завершённые поля в declaration order без destructor незавершённого объекта.
- [x] Move передаёт существующую identity; Copy рекурсивно создаёт независимые
  объекты, включая поля/arrays/tuples/enum payload. Copy + destructor запрещён.
- [x] Identity equality (`==`, `!=`, `same_identity`) заимствует operands без Copy.
- [x] Readonly/mutable methods, class views, rebind и запрет owning slot replacement
  через class view. Внутренний captured assignment address хранится отдельно от view identity.
- [x] Differential HIR/MIR и native O0/O3 cases: создание, возврат, replacement,
  recursive chain, partial construction, first/double panic, Copy, generic instances.
  Native instrumentation проверяет live allocation set, отсутствие double free,
  ровно одну allocation при move через функцию и вторую identity при Copy.

Views пока консервативно зависят от исходного owner: перемещение owner при
последующем использовании view отвергается. Inheritance/upcast/dyn относятся к C4.
OOM и fallible storage allocation остаются отдельной приёмкой C5; Box allocation
пока использует стандартную политику Rust. Полный memory acceptance — C9.


Результат C3.1 / C3.3 / C3.5:

- [x] Статический result contract содержит capability и пути к исходным аргументам.
  Lifetime сохраняется через несколько calls, generics, imports/reexports,
  if/match, recursive и mutually recursive entries.
- [x] Возвращаются readonly/mutable views non-Copy inline/class значений и их полей;
  Copy-result остаётся значением. Непересекающиеся поля сохраняют независимость.
- [x] HIR/MIR и Rust передают descriptor; возврат view не добавляет allocation,
  clone, ownership flag или destructor чужого target. Panic cleanup учитывает
  возвращаемый descriptor и consuming arguments независимо.
- [x] Entry result/effect summaries достигают fixed point; returned aliases
  сохраняют finalizing restrictions через recursive calls. Динамический index
  обобщается до containing place, глубокие recursive paths — консервативно до root.
- [x] Escaped local/owned parameter, mixed owned/view return, readonly promotion,
  invalidation и передача view в owning storage отвергаются. Ошибки конфликтов
  показывают origin, creation и reachable later use с исходными source positions.
- [x] Исправлена зависимость recursive class/enum layout от порядка объявлений,
  включая generic instances; inline recursion и бесконечное расширение типов запрещены.
- [x] Non-Copy owned/borrowed function values, unbound receivers, generic
  higher-order calls и cleanup при indirect call. Static metadata сохраняет ABI,
  result/component origins и may-write effects; значение содержит один code pointer.
  Несовместимые owned/borrowed result ABI при объединении pointers отвергаются.

View от temporary receiver, уничтожаемого внутри receiver scope, не выпускается
наружу. Loop-carried cyclic reborrowing пока диагностируется.
Пример: `examples/borrowed_results/main.nr`.

Результат C3.2:

- [x] Readonly поля struct/tuple/enum и элементы fixed array без скрытого clone.
  У массива единый static mode; у полей независимые modes.
- [x] Move/copy descriptors, rebind поля mutable оболочки, nested provenance и
  транзитивные зависимости view на всю оболочку. Target остаётся readonly.
- [x] Typed HIR/MIR и Rust representation, cleanup только owning полей; partial
  construction и panic при возврате не уничтожают чужой target.
- [x] Argument/result specialization по physical storage types через functions,
  методы, generics, imports/reexports и рекурсию. Result component contracts
  сохраняют несколько независимых origins; view может пережить wrapper при
  сохранении внешнего target. Типы и dependencies выводятся до fixed point.
- [x] Escaped local/owned target, invalidation и readonly promotion отвергаются;
  constructor initialization учитывает aliases, сохранённые в аргументах оболочек.
- [x] Class fields и специальные lifecycle entries для borrowed representations.
  Constructor выбирает payload до allocation; методы и destructor получают тот же
  layout. Implicit destructor reads продлевают loans до cleanup; may-before анализ
  проверяет фактический порядок root initialization, aliases и scope boundaries.
  Циклы внутри owning group и изменение порядка cleanup остаются отдельным C7.
- [x] Mutable borrowed aggregate arguments: статические may-write contracts для
  сохраняемых после вызова views, включая вложенные вызовы, generics, методы,
  imports/reexports, рекурсию и panic. Callee-local/owned targets не могут попасть
  в чужое storage. Dynamic indices расширяются до возможных fixed-array slots.
  Прежние зависимости сохраняются консервативно; гарантированный overwrite через
  вызов пока не сокращает lifetime старого target.
- [x] Refinement известных локальных enum variants: создание, move, root replacement,
  совпадающие ветви и loops. Доказанно недостижимые переходы исключаются из MIR,
  provenance, liveness, result/effect inference, initialization и generated Rust.
  Calls и projected/aliased writes сбрасывают знание variant.
- [x] Локальный вывод modes по последующим прямым присваиваниям: struct/tuple/array,
  nested fields, whole wrapper replacement и propagation через локальные wrapper
  bindings. Initializers перепроверяются до fixed point; provisional specializations
  откатываются до выбора ABI. Array mode объединяет требования всех элементов,
  сохраняя порядок вычисления и числовой contextual typing.
- [x] Вывод modes через параметры, mutable aliases, constructors, returned values
  и сложные initializer contexts (if/match, nested aggregates, function values).
  Parameter/result requirements достигают fixed point; временный owning target
  нельзя превратить в escaping view. Refinement произвольных enum places/calls
  остаётся консервативной точностью анализа, а не runtime изменением storage mode.

C3.2 закрыт. Примеры: `examples/borrowed_storage/main.nr` (24) и
`examples/borrowed_storage_write/main.nr` (10, переназначение поля через generic helper).
`examples/enum_refinement/main.nr` даёт 3 после замены borrowed enum на пустой variant
и разрешённой замены прежнего target. `examples/storage_modes/main.nr` даёт 11:
режим поля выводится из позднего присваивания, режим массива — из всех элементов.

Проверки C3 охватывают workspace Debug/Release, native O0/O3, совпадение HIR/MIR
results/drop traces, positive/negative imported/generic cases и Clippy без warnings.
Подробный итог проверки — в [status](status.md). Новые примеры
`examples/borrowed_classes/main.nr` и `examples/borrowed_callables/main.nr` дают 14.


Результат C4.1:

- [x] Номинальные interface implementations через `struct/class S: I, J`;
  matching methods без явного implementation не удовлетворяют constraint.
- [x] Type/const generic interfaces, defaults и conjunction `T: I & J & Copy`;
  зависимые constraints разрешаются до fixed point, включая method-level generics.
- [x] Проверка public methods, receiver/mutability, parameter/result signatures,
  alpha-equivalent method generics, constraints и defaults. Некорректные interface
  signatures/defaults отвергаются даже без использования объявления.
- [x] Abstract generic audit при специализации: доступны только объявленные
  constraints; concrete extra methods и неуказанный Copy не просачиваются в body.
- [x] Static/instance calls и function values используют concrete entries и C3
  borrowed/lifetime/effect contracts без runtime interface representation.
- [x] HIR/MIR/native differential cases, imported/reexported implementations,
  положительные и отрицательные diagnostics. Пример —
  [interfaces](../../examples/interfaces/main.nr), результат 8.

Method-level const signatures сравниваются символически после переименования
параметров; отдельного доказательства алгебраической эквивалентности нет.
Class inheritance, static upcast и dyn storage этот чекпоинт не закрывает.

Приёмка C4.1: 163 workspace tests в Debug/Release, 23 native interface-программы
при O0/O3, HIR/MIR/drop traces, Clippy без warnings. Подробности — в [status](status.md).


Результат C4.2a:

Private fields проверяются при чтении/записи через aliases, literal construction
и named patterns; private methods — при direct calls и создании code pointers.
Контекст задаёт declaration, а не caller/module/type arguments или receiver value.
Обычный private top-level function сохраняет module visibility. Diagnostic E0352
показывает запрещённый доступ и место private declaration.

Явно специализированные method values `Box<i32>.method<bool>` проходят общий
путь проверки и specialization. Публичный метод может вернуть code pointer
на private implementation: indirect call использует полученную capability.
Проверка callers ожидает instantiation returned callable producers; порядок
размещения HIR functions сохраняет function IDs при отложенной проверке.

C4.2 остаётся открытым: hierarchy, embedded base, super и virtual/override
ещё не реализованы. Пример — [private_members](../../examples/private_members/main.nr),
результат 7.

Приёмка C4.2a: 167 workspace tests в Debug/Release, 13 новых native-программ
при O0/O3, HIR/MIR/drop traces, Clippy без warnings. Подробности — в [status](status.md).


Declaration layer C4.2b/c реализован. Он проверяет иерархию, сохраняет declaring
owner, сравнивает override signatures после подстановки parent arguments и
переименования method binders. Закрытые const expressions вычисляются; символические
выражения должны совпадать структурно после переименования. Неявного member hiding
нет; private base member не является доступной целью override.

Root-class virtual methods поддерживают generic/callable/borrowed contracts C3
и static calls без vtable. Пример — [virtual_methods](../../examples/virtual_methods/main.nr),
результат 8. Valid concrete inheritance по-прежнему останавливается на E0900:
embedded base, super, inherited body lowering и derived/base cleanup ещё не готовы.
C4.2b/c и весь C4.2 остаются открытыми.
Сравнение declaration type expressions консервативно: typed normalization
вложенных nominal defaults и base entry/storage/effect contracts относятся
к следующему lowering этапу.

Проверки declaration layer: 174 workspace tests в Debug/Release, 54 hierarchy
cases, семь новых native-программ при O0/O3, HIR/MIR/drop traces и Clippy
без warnings. Подробности — в [status](status.md).
