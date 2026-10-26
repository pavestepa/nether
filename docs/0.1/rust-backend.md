# Nether → Rust: утверждённая концепция

Редакция 2026-10-25. Единственная модель памяти — [NMM](../core-memory-managment.md)
(`nmm-0.1`); unsafe boundary — [unsafe.md](../unsafe.md). Поддержка — [status](status.md).

Решение от 2026-10-13. Nether — язык со своей проверкой типов, ownership, views,
identity и deterministic cleanup, компилируемый в Rust source. Единственный
основной pipeline: Nether → typed HIR → checked MIR → Rust → rustc → executable.
Прямой LLVM backend удалён. Обычный rustc разрешён, включая его внутренний LLVM;
Nether не генерирует LLVM IR и не требует отдельной установки Clang.

## Производительность

Цель — производительность сопоставимого вручную написанного Rust при одинаковых
allocations, dispatch, арифметике, bounds checks и cleanup semantics. Это критерий
приёмки, а не обещание одинакового машинного кода для любой программы. Сравниваются
время, allocations и размер кода; проверяются debug и optimized builds.
Недопустимы обязательные GC, Rc/Arc, RefCell, boxing inline значений, скрытые clone
или динамический ownership tag для обхода статической проверки.

## Модель памяти

Backend сохраняет только действующий контракт NMM `nmm-0.1`. Rust borrow
checker не заменяет Nether ownership inference. Backend может генерировать проверенный unsafe,
raw pointers, MaybeUninit и ManuallyDrop. Для каждого такого применения нужны
доказательства initialization, alignment, provenance, lifetime и uniqueness.
Нельзя создавать &mut для более широкого/долгого доступа, чем доказывает Nether.

Порядок уничтожения не передаётся обычному Rust Drop без доказанной эквивалентности:
roots уничтожаются обратно последней успешной инициализации; root replacement
обновляет его позицию в исходном scope; поля — в объявленном порядке. Panic и
partial moves сохраняют drop flags. Повторная panic при cleanup вызывает abort.
Finalizing target нельзя целиком заменить даже через aliases и helper calls.

## Class и наследование

Для concrete class A генерируются payload struct, trait представления A и owned
storage на основе Box<AData>. B : A содержит base payload inline внутри BData и
реализует trait A; отдельный Box для базовой части не создаётся. Upcast не копирует
payload, не создаёт новую identity и не выделяет память. Struct остаётся inline
и не получает concrete inheritance. Поддерживается single class inheritance.

Там, где ожидается A, допустим B с сохранением access/ownership contracts.
Если concrete type известен, вызов специализируется статически (trait bounds,
monomorphization), включая транзитивное наследование. Когда тип выбирается во
время выполнения или нужна смешанная коллекция, используются dyn trait views и
Box<dyn ATrait> для owned polymorphic storage. Одна class identity — один owned
allocation; boxing существующего Box повторно недопустим.

Borrowed upcast передаёт view на тот же объект; readonly не становится mutable.
Cleanup вызывает destructor concrete типа и затем base cleanup по спецификации.
Traits включают доступ к base state через проверенные accessors: Rust trait сам
по себе не содержит полей. Private поля остаются private для Nether checker.
Receiver modes сохраняются в override. Generic методы без фиксированной
специализации не включаются в dyn API. Class view не позволяет заменять owning slot.

Текущий source checker ещё отклоняет class, constructors, inheritance и dyn;
описанное представление — обязательная цель обновлённого roadmap, а не заявление
о готовой реализации. Миграция backend не делает эти формы молча принимаемыми.

## Toolchain и проверка

`emit-rust` выдаёт самостоятельный .rs для поддержанного subset. `build` вызывает
rustc; `--emit=object` выдаёт host object. `NETHER_RUSTC` выбирает compiler executable.
Первый профиль — 64-bit targets; portable subset выполняется на host. Linux
x86-64 остаётся первым platform adapter для прямых syscalls, не ограничением
обычной генерации Rust. Cross-target flags и platform APIs требуют отдельного
проверенного target profile.

Generated Rust и metadata привязаны к совместимому toolchain. Стабильного Rust
binary ABI не обещается. Foreign boundary использует extern "C" и repr(C).

Основания: [Rust: trait objects](https://doc.rust-lang.org/reference/types/trait-object.html),
[Rust: monomorphization](https://doc.rust-lang.org/book/ch10-01-syntax.html),
[Rust: codegen options](https://doc.rust-lang.org/rustc/codegen-options/index.html).

Для backend storage адреса locals захватываются raw pointer один раз; последующие
field projections не создают новый &mut на весь aggregate. Это сохраняет
совместимость одновременно живых disjoint field views с aliasing rules Rust.

## Реализованный class lowering

Concrete class имеет pointer handle и один Box с MaybeUninit payload,
initialization flags для ресурсных полей и признаком завершения constructor.
Borrowed class descriptor хранит pointer на эту identity. Внутренний descriptor
адреса assignment дополнительно хранит owning slot/drop flags; он не является
пользовательским class view. Это позволяет инициализировать class-поле, не читая
ещё не существующий handle, и не даёт view заменять владельца.

Copy class выделяет отдельный Box и копирует поля по правилам их типов;
Move передаёт существующий pointer. Cleanup запускает class destructor только
после успешного завершения constructor, затем поля и освобождение Box.
Base payload и upcast ещё не реализованы в source execution.


## Borrowed results и callable entries

Function result contract хранится в HIR/MIR отдельно от логического типа.
`CallView` возвращает typed descriptor; inline target использует raw place,
resource/class target — descriptor с существующим lifecycle/identity состоянием.
Копирования payload, нового Box и runtime выбора owned/borrowed режима нет.
Обычные Copy-results и owned results используют существующее value lowering.

Result provenance выводится до fixed point вместе с выбранными entry
specializations. Caller подставляет свои origins в пути к параметрам callee;
last-use loan checker запрещает invalidation и escape. Conditional return
передаёт descriptor выбранной ветви, сохраняя объединённый статический lifetime
contract. Effect summaries учитывают returned aliases, включая finalizing target.

Function pointers поддерживают non-Copy owned/borrowed arguments/results и unbound
receivers. Статическая metadata задаёт выбранный ABI, origins и effects. Indirect
call использует обычный code pointer без environment или runtime ownership tags.

Backend использует отдельный внутренний `StoredView<T>` для readonly полей
struct/tuple/array/enum. В Rust он становится descriptor target либо raw pointer
для resource-free target; у class descriptor сохраняется heap identity. Тип
оболочки статически отличается от owned representation. Store/load descriptor
не копирует payload, не создаёт allocation и не запускает target destructor.
Внутренний layout descriptor зависит от backend и lifecycle target и пока не
экспортируется как source layout primitive .

MIR provenance привязана к projected storage paths. Copy/move агрегата переносит
зависимости его borrowed компонентов; rebind меняет descriptor. Liveness view
на оболочку транзитивно удерживает зависимости её компонентов. Cleanup обходит
только owning поля. Argument/result ABI специализируется по physical storage
shapes; возвращаемый owned aggregate переносит descriptors и их независимые
статические зависимости. Symbolic dereference отличает внешний target параметра
от slot внутри переданного по значению wrapper. При подстановке контракта caller
разрешает эти пути в собственные origins. Динамические result indices расширяются
на возможные slots и не сохраняют callee-local IDs в контракте.

Типы результатов и component provenance выводятся до стабильного состояния до
окончательной проверки loans. Это работает через generic и recursive entries и
imports. Mutable borrowed aggregate writes описываются отдельными may-write
contracts выбранного entry. MIR подставляет caller origins для каждого потенциально
изменяемого descriptor slot на normal и unwind edges. Прежние зависимости остаются
живыми консервативно. Публикация view callee-local/owned target отвергается; source
paths через class payload и dynamic array indices сохраняют lifetime владельца.
Контракты существуют только в compiler: Rust ABI и runtime representation не
получают новых tags, allocations или проверок ownership. Пользовательские
destructors оболочек учитывают implicit reads и согласованный порядок cleanup;
borrowed class payload имеет отдельный physical layout для constructor/method/drop.


## Уточнение достижимости локальных enum

После lowering отдельный finite dataflow pass переносит known local discriminants
и bool/integer constants. Join сохраняет только совпадающие факты; loops доходят
до fixed point. Calls и projected/aliased writes сбрасывают знание. Неизвестное
условие сохраняет обе ветви, panic edges не исключаются по предположению.
Доказанно недостижимые блоки получают `Unreachable`, а доказанный branch — `Goto`.
Проверки provenance/liveness, initialization, returns/effects и Rust emission
используют один и тот же MIR. Поэтому невозможный owned return не попадает в
borrowed Rust ABI, а недостижимое чтение пустого payload не требует loan origin.
Произвольные nested enum places и callee result discriminants пока не выводятся.
Пример: [enum_refinement](../../examples/enum_refinement/main.nr).


## Локальный вывод storage modes

Checker повторяет проверку тела при росте readonly требований к named local
storage. Ключ hint — source span binding, а не номер HIR local: rechecks и
shadowing не смешивают разные переменные. Последующие прямые field/index/root
writes уточняют initializer representation; hints распространяются через local
wrapper bindings. Provisional monomorphization/entries откатываются при retry,
так что в final HIR/MIR остаются согласованные signatures и physical types.
Требования проходят также через mutable aliases, параметры, returned values,
constructors и if/match. Межпроцедурный fixed point хранит ключ requested signature
отдельно от inferred physical signature; provisional entries откатываются при retry.

Array checking сохраняет numeric contextual typing, объединяет physical modes
всех элементов и перепроверяет initializers в итоговом mode. Это compile-time
повтор, runtime evaluation не дублируется. Ни owner allocation, ни runtime tags,
ни hidden clone не добавляются; owning temporary, несовместимый с readonly mode,
отвергается. Explicit generic record/enum constructors сохраняют physical context.
Пример: [storage_modes](../../examples/storage_modes/main.nr).


## Destructor dependencies и borrowed class layout

Class handle сохраняет nominal identity и содержит выбранный статически payload
layout. Canonical recursive class edge остаётся непрозрачным; physical borrowed
поля не требуют бесконечно разворачивать recursive Type. Registry и drop entries
различают layouts. Интерпретаторы и Rust используют один и тот же physical key.

Liveness включает implicit user-destructor reads. Loan holders отслеживаются
транзитивно через stored descriptors. Cleanup допускается, если holder завершается
раньше target: по scope, construction field order или доказанному root initialization
order. May-before pairs объединяются на joins/loops; aliased root replacement
учитывает provenance. Owning-cycle запрещён NMM; невладеющие циклы требуют доказанных lifetime/weak.


## Статические interfaces

Interface declarations, явные реализации class/struct и constraints существуют
в checker. Abstract generic audit использует только объявленные методы constraints;
при concrete specialization вызовы направляются в обычные method entries.
Conjunction, generic methods, type/const arguments, static methods и function
values сохраняют существующие physical storage/callable contracts NMM.
Rust source получает конкретные специализации: interface сам по себе не добавляет
trait object, vtable, Box, refcount, runtime tag или копирование payload.
Runtime Rust traits/dyn dispatch, class inheritance и upcast ещё не завершены.


## Private member access

Checker проверяет access в lexical declaring type до выбора concrete entries.
Generic/borrowed specialization и emitted code не получают runtime access tags.
Function pointer, полученный через public factory, сохраняет доступ к выбранному
code entry; для indirect call повторного разрешения private имени не требуется.
Caller проверяется после instantiation producer возвращаемого generic callable;
порядок проверки не меняет function IDs и соответствие ABI/result/effect contracts.


## Hierarchy declarations и static virtual entries (пока только declarations)

Declaration checker проверяет class graph и override signatures до выбора entries.
При подстановке parent arguments method binders переименовываются, module constants
разрешаются в исходном lexical context. Root virtual methods используют существующий
concrete entry lowering без vtable или новых allocations. Derived payload lowering,
embedded base construction/super и cleanup ещё требуют реализации; правильная
declaration hierarchy сама по себе не разрешает генерацию derived object.

## Source unsafe boundary

Source `unsafe fn` safety проверяется до HIR/MIR execution и codegen.
Явный unsafe block сохраняется как обычный checked block; ordinary NMM checks
не отключаются. Generated Rust может использовать внутренние unsafe функции
и raw pointers независимо от source safety; это trusted backend implementation,
не разрешение caller вызывать source unsafe fn без блока.

## Консервативные оптимизации 2026-10-25

При `overflow-checks=off` после полной проверки MIR выделяется замкнутое
множество infallible scalar functions. Для него codegen использует structured
HIR: обычные Rust locals, if/while и прямые value-returning вызовы. Множество
вычисляется до fixed point, включая рекурсию. Поддержаны Unit/Bool/Integer,
wrapping arithmetic и деление/остаток на положительный literal. Dynamic divisor,
MIN / -1, views, indirect calls, сложные value blocks и прочие неподдержанные
формы оставляют функцию на проверенном MIR fallback. Checked arithmetic
сохраняет прежний путь. Result ABI wrappers остаются для внешних/indirect callers;
внутри scalar subset вызовы не передают Fault. Public MIR-only emission сохраняет
MIR dispatcher: structured HIR доступен через обычный source pipeline.

Для closed programs с простыми non-Copy классами допускается frame allocation.
Условия консервативны для всей программы: только scalar fields (не более 16),
нет destructors, class results, owned class parameters, aggregate storage,
indirect calls или identity observations. May-live анализ CFG разрешает
allocation site лишь при отсутствии живых class owners на входе; это исключает
перезапись места ещё живого объекта при следующей итерации/replacement. На функцию
не более 16 frame slots; остальные sites используют heap. Существующие NMM
loans/initialization/cleanup checks исполняются до этого выбора.

Heap и frame handles имеют одинаковые operations/cleanup; внутренний `heap_owned`
бит определяет только освобождение backing storage, не право владения или
borrow mode. Frame storage живёт до выхода из функции, а логическая жизнь owner
по-прежнему заканчивается на исходной cleanup boundary. Copy classes, escaping
objects, destructors и наблюдаемая identity оставлены на прежнем heap пути.
Обе оптимизации сейчас включаются только в wrapping arithmetic mode.

`compiler/core/tests/optimizations.rs` проверяет eligibility/fallback и нулевое
число heap allocations через counting allocator при O0/O3. Native differential
suite сравнивает с HIR/MIR рекурсию, unsigned limits, overflow, panic конструктора,
loop-carried owner, break и continue. Это ограниченный subset, не общий escape
analysis или устранение всех Result/allocations.
