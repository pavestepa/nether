# Реализация оставшихся возможностей языка

Этот план фиксирует весь запрошенный объём. Отметка одного этапа не означает
готовности языка 0.1 целиком. Приоритет — NMM contracts и независимые
positive/negative/HIR/MIR/native проверки до расширения следующего слоя.

| Этап | Состояние | Условие приёмки |
| --- | --- | --- |
| Синхронный NMM, MIR boundary, backend/runtime regression | Реализован subset, полная нормативная матрица будущих primitives открыта | [Матрица](memory-cases.md), [границы verifier](nmm-verification.md) |
| Unsafe callable types | Реализованы | Safety через annotations/generics/storage/imports; indirect unsafe admission; нет safety erasure; native O0/O3 |
| Statement loop | Реализован | Вложенные break/continue/return, divergence, cleanup; value-result loop остаётся открытым |
| Raw pointers/address/deref/casts/arithmetic | Scalar subset реализован; non-scalar/reinterpret/integer casts открыты | Explicit unsafe, W admission, element bounds/provenance obligations, MIR verifier и reference/native O0/O3 |
| Allocation/storage initialize/take/drop | Owned scalar/Unit subset реализован; non-scalar и borrowed mode открыты | `raw_storage.rs`: initialized slots, consuming release, panic cleanup, ZST/OOM; [границы](storage-contracts.md#6-реализованный-owned-scalar-subset) |
| Vec/String/buffers и safe wrappers | Не реализован | Storage primitives приняты; grow/remove invalidation, bounds, drop order; UTF-8 contract для String |
| I/O/resource wrappers | Не реализован | Target-independent API, errors, owned resource cleanup и unwind |
| Derived construction/base/super | Declarations subset | Embedded base, inherited entry resolution, base/derived cleanup и panic construction |
| Upcast/dyn/heterogeneous storage | Не реализован | Identity/provenance preserved, vtable ABI, readonly/W/owned contracts |
| Weak | Не реализован | N21–N25/N33, generation reuse, guards, invalidation, OOM |
| Capturing closures | Не реализован | N35–N42, R/W/O capture inference, escaping, Once, cleanup |
| Channels/tasks/async | Не реализован | N26–N28/N40, MaySuspend, send commit/failure/cancellation, suspension loans |
| FFI | Не реализован | C ABI, unsafe callbacks, linking, unwind/resource contracts |
| Separate compilation/metadata | Контракт описан | Serializer/importer, version/ABI/safety/NMM compatibility checks |
| for/iterators/range values/?/variadic packs | Не реализованы | Правила element lifetime, iterator ownership, error propagation cleanup, evaluation order |
| General structured lowering/panic elimination/escape analysis | Ограниченный subset | [Backend](rust-backend.md), differential tests и повторные [замеры](../../performance/RESULTS.md) |

Safe и unsafe callable types пока требуют точного совпадения safety; implicit
safe-to-unsafe coercion не реализована. Statement loop не поддерживает break
со значением. Поддержанный raw subset и его ограничения перечислены в [unsafe](../unsafe.md).
Нереализованные raw/storage operations по-прежнему отклоняются даже в unsafe.
