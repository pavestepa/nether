# Базовое размещение типов Nether 0.1

Статус: исполнимый контракт расчёта размера и выравнивания. Реализация —
`compiler/semantics/src/layout.rs`. Это часть этапа 0, а не готовый ABI:
calling convention, runtime symbols, panic/unwind и link protocol ещё открыты.

## Target и ограничения

Начальный target — `x86_64-unknown-linux-gnu`, little-endian, указатель 64 bits.
Расчёт ведётся в target-размерах, не через host `size_of`. Размер одного объекта
ограничен `2^63 - 1` bytes, включая padding. Размеры и offsets не должны
переполняться. Alignment — ненулевая степень двойки, не больше этого предела.
Невозможный layout является compile error, а не wrapping или запросом allocator.

## Скалярные представления

| Значение | Size | Alignment |
| --- | --- | --- |
| `()` | 0 | 1 |
| `bool`, `i8`, `u8` | 1 | 1 |
| `i16`, `u16` | 2 | 2 |
| `i32`, `u32`, `f32`, `char` | 4 | 4 |
| `i64`, `u64`, `f64`, `usize`, `isize` | 8 | 8 |
| `i128`, `u128` | 16 | 16 |
| Raw pointer, class identity reference, function pointer | 8 | 8 |
| `str` | 16 | 8 |

`str` — data pointer и byte length в этом порядке. Descriptor не владеет
данными и не стирает provenance. `bool` имеет допустимые значения 0 и 1,
`char` — только Unicode scalars; подходящий размер не делает arbitrary bits
корректным значением. Plain C function pointer не взаимозаменяем с Nether
function pointer, хотя размер совпадает.

Class reference не содержит inline payload. Layout payload вычисляется отдельно;
owned class имеет отдельный allocation и стабильную identity. Weak/group
metadata и размер фактического allocation определяются runtime-контрактом,
который пока не завершён. Нельзя предполагать, что размер class reference равен
размеру объекта или что размер пустого payload позволяет объединять identities.

## Aggregates

Поля struct, tuple, class payload и одного enum variant размещаются в порядке
объявления. Перед каждым полем offset округляется вверх до alignment поля;
конечный размер — до максимального alignment всех полей. Размер включает
tail padding, поэтому он же является array stride. Packing и перестановки
полей в baseline отсутствуют.

Вход layout calculation — уже разрешённые **представления** полей. Он не
выводит owned/view mode и не заменяет representation analysis. В частности,
заимствованное inline значение может требовать descriptor, а не inline payload;
соответствующее представление должно быть передано анализом контрактов.
Контракт контейнерных режимов [утверждён](decisions/container-ownership.md):
owned элементы имеют своё представление, borrowed элементы — readonly descriptors.

Пустой aggregate имеет size 0, align 1. Zero-sized поле сохраняет alignment,
но не требует отдельного адреса. Совпадение адресов ZST не означает одинаковое
владение: initialization/drop state относится к логическим slots. Over-aligned
layout поддерживается расчётом; публичный attribute выравнивания пока не введён.

Array `{T; N}`: size = size(T) × N с checked arithmetic, align = align(T).
При N=0 alignment сохраняется. Array ZST имеет нулевой размер при любом
допустимом N: число логических элементов и количество cleanup операций всё
равно N. Нулевой размер не является разрешением пропустить destructor.

Пример: поля `u8, i64, i32, u8` дают offsets `0, 8, 16, 20`, size 24, align 8.
Массив из трёх таких значений занимает 72 bytes. Нужные правила `Copy` и move
проверяются независимо от этих чисел.

## Enum

Непустой enum использует отдельный `u32` tag в offset 0. Tags — индексы variants
в порядке объявления, начиная с нуля. После tag находится union payload,
выровненный по максимальному alignment всех variants. Размер payload —
максимальный размер variant aggregate; общий size округляется до общего align.
Niche optimization и переиспользование pointer bits отсутствуют.

Enum без variants не имеет значений: его storage layout — size 0, align 1,
без tag. Это не `()`; конструирование значения невозможно. Нижние уровни не
получают право считать произвольные bytes экземпляром такого типа.

В enum с variants `None` и payload `{u8, f64}` tag занимает bytes 0–3,
payload начинается с 8, общий size 24 и align 8. Cleanup касается только
инициализированных owned частей активного variant. Layout сам не добавляет
ни drop flags, ни runtime ownership tags.

## Граница ABI

Эта таблица задаёт собственное baseline object representation Nether.
Она не реализует классификацию C arguments/returns, регистры, `sret` или
call-frame alignment. `repr(C)` должен дополнительно пройти проверку C layout
и ABI-safe состава на target; совпадение размера нельзя считать достаточным
для foreign call. Для exports и metadata нужен отдельный ABI-контракт.

Тесты проверяют padding/stride, nested aggregates, enum payload, ZST,
over-alignment и ошибки размеров в debug/release. Перебор небольших комбинаций
полей дополнительно проверяет alignment, отсутствие перекрытий ненулевого
storage и отсутствие лишнего padding.
