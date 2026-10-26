# Nether 0.1 — приёмка NMM и unsafe

Редакция 2026-10-25. Единственный oracle — [NMM](../core-memory-managment.md).
Старые M01/M05 с move по последнему use и R → W больше не применяются.
Полная нормативная матрица — NMM §25; ниже привязка к исполняемым проверкам.

| Контракт | Проверки |
| --- | --- |
| N01–N02: aliases, sequential reborrow, восстановление parent | `compiler/core/tests/nmm.rs`, `loans.rs`, `views.rs` |
| N03–N04: R → W запрещено, Copy независим | `nmm.rs`, `ownership.rs`, `classes.rs` |
| N05–N07: owner result / borrowed result / escape | `nmm.rs`, `borrowed_returns.rs`, `function_values.rs` |
| N08–N09: consuming O, use-after-transfer, `var` не consuming | `nmm.rs`, `entries.rs` |
| Borrowed storage не поставляет O consuming API | `nmm.rs`, `unsafe.rs`, `borrowed_storage.rs` |
| Cleanup/replacement/partial initialization/panic | `drop_state.rs`, `cleanup.rs`, `drop_plan.rs`, `native.rs`, runtime tests |
| Generics/modules/indirect calls сохраняют contracts | `generics.rs`, `modules.rs`, `function_values.rs`, `nmm.rs` |
| U01: unsafe call требует лексический блок | `unsafe.rs` direct/generic/method и вложенные вызовы |
| U02: unsafe fn не разрешает unsafe операции своим заголовком | `unsafe.rs` |
| U03: scope unsafe заканчивается на закрывающей скобке | `unsafe.rs` |
| U04: unsafe не отменяет NMM readonly/loans/O | `unsafe.rs` |
| U05: unsafe function value не выдаётся за safe | `unsafe.rs`: type mismatch; indirect call без блока E0500 |
| U06: return/break/continue и cleanup в unsafe | `unsafe.rs`, native example |
| U07: safety interface/override/import contracts | `unsafe.rs`, parser/modules tests |
| U08: raw read/write/offset/cast требует unsafe | `raw_pointers.rs`, E0500 |
| U09: raw address требует scalar place; writable address требует W и отсутствие конфликтующего loan | `raw_pointers.rs`, E0502/E0317/E0381 |
| U10: pointer type/cast не повышает readonly до writable | `raw_pointers.rs`, E0503; `mir_verify.rs` forged capabilities |
| U11: raw pointers не удерживают target; bounds и one-past | `raw_pointers.rs` reference diagnostics; валидные случаи `native.rs` O0/O3 |
| U12: raw operations совместимы с safe views/generics/unsafe callable | `raw_pointers.rs`, `native.rs` |
| U13: scalar storage symbol identity, private fields, consuming release, W mutation | `raw_storage.rs` checker/module/reexport tests |
| U14: scalar/Unit init/take/replace/drop state и invalid unsafe protocols | `raw_storage.rs` HIR/MIR; invalid programs не исполняются native |
| U15: allocation normal/panic cleanup, zero capacity/ZST и OOM | `raw_storage.rs` native O0/O3 со счётчиком allocations и failure injection |

## Полная привязка нормативной матрицы

| Cases | Реализованная проверка / граница |
| --- | --- |
| N01–N09 | `nmm.rs`, `loans.rs`, `borrowed_returns.rs`, `ownership.rs`, `entries.rs`; нормативный Counter из §25.1 исполняется также native O0/O3 |
| N10–N11 | `borrowed_storage.rs`: homogeneous owned/view storage, cleanup, запрет несовместимых modes; buffer pop ещё не реализован |
| N12–N13 | `nmm.rs`: явные oracle traces `[1,3,2]` и rebind `[3,1]`; lifecycle unit/native и `cleanup.rs` |
| N14–N15 | `classes.rs`, `views.rs`, `loans.rs`: whole class replacement, disjoint fields, dynamic index conflicts |
| N16–N17 | lifecycle unit/native, `drop_plan.rs`, `drop_state.rs`, `borrowed_storage.rs`: partial construction/transfer |
| N18–N20 | `borrowed_destructors.rs`, lifecycle unit/native, `effects.rs`, `drop_plan.rs`: lifetime, transitive replacement, panic/abort |
| N21–N25 | Owning-cycle/weak graph API не реализован; нормативная приёмка открыта |
| N26–N28 | Channels/cancellation/await не реализованы; приёмка открыта |
| N29 | `modules.rs`, `function_values.rs`, `nmm.rs`: imports, indirect calls, recursion |
| N30 | `nmm.rs`: join/continue без reinitialization отклоняются; `native.rs`, `drop_state.rs`, `cleanup.rs`: loops/drop flags/bounded state |
| N31 | Grow/remove API ещё не реализован; приёмка открыта |
| N32 | `borrowed_storage.rs`, `unsafe.rs`: mutable wrapper не повышает readonly target |
| N33 | Weak metadata не реализованы; приёмка открыта |
| N34 | `classes.rs`, lifecycle native: Copy class identity против transfer |
| N35–N42 | Capturing closures не реализованы; приёмка открыта |

Verifier проверяется отдельно намеренно повреждённым MIR в `mir_verify.rs`.
[Граница backend/runtime, гарантии и ограничения](nmm-verification.md).
Наличие регрессионного покрытия не означает исчерпывающего перебора всех
программ. Для новых primitives нужны собственные positive/negative/runtime cases.

## Различающие примеры

```nether
struct Cell {value:i32}
struct Hold {cell:Cell}
fn store(x:Cell):Hold {return Hold {cell:x}}
fn main():i32 {
    let owner = Cell {value:7}
    let alias = owner
    let h = store(alias) // NMM_NO_OWNER_TOKEN, даже без последующего use owner
    return h.cell.value
}
```

Непотребляющий API явно создаёт alias параметра перед сохранением descriptor:

```nether
fn borrow(x:Cell):Hold {
    let alias = x
    return Hold {cell:alias}
}
```

Его caller сохраняет O; Holder не переживает цель и не уничтожает её.
Прямое `store(owner)` передаёт O, а последующий use owner отклоняется.
`unsafe { let h = store(alias) }` тоже отклоняется.
