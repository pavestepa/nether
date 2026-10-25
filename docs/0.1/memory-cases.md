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
| U05: unsafe function value не выдаётся за safe | `unsafe.rs` E0501 |
| U06: return/break/continue и cleanup в unsafe | `unsafe.rs`, native example |
| U07: safety interface/override/import contracts | `unsafe.rs`, parser/modules tests |

Наличие файла в таблице не означает исчерпывающее покрытие всех сочетаний.
Weak/closures/channels/async, raw storage и полный MIR verifier остаются
незакрытой приёмкой. Для каждого нового primitive нужны положительные,
отрицательные и runtime cases; неподдержанные операции отклоняются явно.

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
