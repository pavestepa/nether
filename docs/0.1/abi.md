# Rust code generation и границы ABI

Актуальная архитектура: [Nether → Rust](rust-backend.md). Прямой LLVM backend и
старый C linking protocol удалены из основного pipeline. Внутренний ABI не является
стабильным Rust или C binary ABI; совместимость требует одного toolchain/profile.

## Внутренние вызовы

Текущий backend генерирует типизированные Rust функции:

```rust
unsafe fn entry(owned: Payload, borrowed: *mut Payload) -> Result<Output, Fault>
```

Это схема, а не требование, чтобы каждый entry имел оба параметра. Режимы
параметров выбираются статически. Result — внутренний канал panic propagation,
не пользовательский enum Result. Receivers borrowed. Readonly capability
проверяется Nether checker; raw pointer не даёт пользователю права на запись.
Function value указывает на фиксированный entry с известной signature.

Для программ с ресурсами функции дополнительно получают внутренний Context
(panic state и последовательность успешных initialization). Resource view хранит
типизированные raw pointers на payload и lifecycle flags; pointer на root stamp
позволяет whole replacement обновлять порядок в исходном owning scope. Для field
view этот pointer отсутствует: запись поля не переставляет root. Lifecycle flags
фиксируют initialization, а не выбирают ownership динамически. Resource-free
программы сохраняют простую схему выше без Context. Этот внутренний ABI пока
не заявляется как оптимальный по размеру views; performance acceptance — C9.

Локальные места используют MaybeUninit; payload types не получают произвольный
Rust Drop. Нельзя читать неинициализированные или moved части; перед emission
проверяются initialization и loans. Адрес поля формируется через raw place,
без создания ссылки на целиком неинициализированный aggregate. Физический move
не заменяет логические ownership/drop flags.

## Panic и cleanup

Fallible операция возвращает Fault в MIR unwind edge. Сохраняется исходная
позиция Nether source. Main печатает location и завершает процесс с кодом 101.
Обычное завершение возвращает 0 для unit либо код i32. Арифметика генерирует явные
checked/wrapping операции независимо от Rust debug/release overflow defaults.
Bounds проверяются до доступа. Checked shift проверяет count, wrapping shift
маскирует его по разрядности; division overflow (включая MIN % -1) вызывает panic
в обоих режимах.

Для ресурсных типов обязательны cleanup flags и отдельный explicit cleanup:
roots обратно последней успешной инициализации, body destructor до owned полей,
поля по объявлению, элементы по индексу, enum по активному payload. Первый panic
продолжает cleanup; повторный panic вызывает abort. Return staging очищается при
failure cleanup, а на успешном выходе передаётся caller.

Finalizing slot нельзя повторно инициализировать, перенести или уничтожить.
Запись его живых полей разрешена. Effects проверяются через aliases и calls.
Source destructors inline aggregates, включая generic instances, подключены к
HIR/MIR и generated Rust. Class allocation и class cleanup используют тот же проверяемый protocol.

## Class representation

Concrete owned class использует Box<ConcreteData>. Inheritance реализуется
base payload внутри derived struct и trait implementations. Известный concrete
тип специализируется статически. Owned runtime polymorphism использует
Box<dyn BaseTrait>, borrowed — trait view с исходным lifetime/capability.
Upcast сохраняет identity/allocation и не делает clone или дополнительный Box.
Rust Drop конкретного Box сам по себе не заменяет Nether cleanup protocol.

## Сборка

`emit-rust` выдаёт самостоятельный Rust source текущего subset. `build` передаёт
его rustc и атомарно публикует output только после успешной сборки. Аргументы
передаются argv. `NETHER_RUSTC` выбирает executable; `--emit=object` выдаёт object
для host, не универсальный переносимый binary. Профиль пока требует 64-bit target.
Object хранит внутренние Rust symbols и может требовать Rust runtime при linking;
для приложения следует использовать обычный `build`.

## Foreign boundary и platform

Будущие FFI wrappers используют extern "C", ABI-safe scalars, raw pointers и
repr(C) POD. Обычные Rust tuples, trait objects, enums, classes и safe views не
являются C ABI. Panic не пересекает foreign boundary. Direct syscalls реализуются
через target-specific Rust asm adapters с явными registers/clobbers; первым
остаётся Linux x86-64. Это требование roadmap, пока не реализованное API.

Allocation использует target Layout и allocator Rust; zero-size storage требует
aligned sentinel. Нужны проверяемые deallocation layout, exactly-once cleanup,
OOM и diagnostic allocation accounting. Shared owners/refcounts не вводятся.


## Borrowed aggregate ABI: реализованный C3.2

Readonly descriptors могут находиться внутри struct/tuple/array/enum и class payload.
Concrete entry получает и возвращает выбранное статически представление. Owned
wrapper может содержать borrowed поля, которые не становятся владельцами target.
Result metadata внутри compiler хранит отдельные origins каждого descriptor и
различает wrapper slot и внешний target; импорты проходят ту же проверку.
Mutable borrowed aggregate entries дополнительно имеют may-write metadata:
parameter/component paths и input origins для возможных новых descriptors. На
normal и panic edges caller объединяет эти зависимости с прежними. Контракт
специализируется вместе с ABI, не добавляя runtime ownership tags. Публиковать
view callee-local/owned target запрещено. Экспорт metadata для отдельной компиляции
остаётся C6.3/C6.4. User destructors имеют отдельный entry на каждый physical layout;
их implicit reads участвуют в lifetime и cleanup-order checks.

Function pointer остаётся одним адресом кода. Статическая signature фиксирует
owned/borrowed параметры и результат; component origins и may-write effects
применяются к indirect call так же, как к direct call. Ни environment, ни runtime
ownership tag, ни ABI adapter не добавляются.
