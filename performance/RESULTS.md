# Производительность Nether и Rust — после оптимизаций 2026-10-25

Замер NMM 0.1 после structured scalar lowering и frame allocation. Полные данные: [до](results/2026-10-25.json), [после](results/2026-10-25-optimized.json); [предыдущий отчёт](results/2026-10-25-before-optimizations.md). Исходники бенчмарков не изменены. Измерено рабочее дерево, commit/dirty и SHA-256 компилятора записаны в JSON.

Среда: macOS 26.6.2 arm64, rustc 1.86.0. Модель CPU недоступна из sandbox. Одинаковая методика до/после: opt-level=3, overflow checks off, без LTO, прогрев и 7 повторов, чередование сторон, медиана wall-clock включая запуск процесса. Во время замеров тесты/сборки других задач не запускались.

## Исполнение

| Тест | Nether до, мс | Nether после, мс (min–max) | Rust сейчас, мс (min–max) | Nether / Rust сейчас |
| --- | ---: | ---: | ---: | ---: |
| array_sum | 143.69 | 142.50 (141.07–161.71) | 146.62 (145.69–153.91) | 0.972× |
| class_alloc | 357.41 | 8.03 (7.49–8.67) | 355.24 (352.85–375.61) | 0.023× |
| class_alloc_elidable | 358.14 | 8.63 (7.19–10.56) | 7.73 (6.94–10.72) | 1.117× |
| collatz | 400.69 | 216.51 (215.53–218.84) | 215.84 (215.43–218.45) | 1.003× |
| fib | 513.64 | 261.63 (257.20–272.67) | 262.18 (257.77–270.63) | 0.998× |
| lcg | 754.27 | 748.21 (745.00–753.73) | 748.51 (746.74–759.42) | 1.000× |
| struct_step | 104.60 | 109.96 (105.73–171.36) | 112.91 (106.93–135.52) | 0.974× |

Коды выхода совпали на каждом повторе. Бенчмарки проверяют 8-битный контрольный результат через exit code; полное сравнение семантики обеспечивается отдельными HIR/MIR/native tests, а не этим checksum.

## Что изменилось

- Collatz: Nether ускорился примерно в 1.85 раза; Fibonacci — в 1.96 раза. В этой серии их время практически совпадает с Rust. Для подходящих scalar functions генерируются обычные locals/if/while и прямые infallible value calls. Безопасный постоянный делитель не требует Fault path.
- `class_alloc_elidable`: Nether ускорился примерно в 41.6 раза. Неубегающие локальные классы без наблюдаемых destructors/identity получают frame storage; rustc затем может устранять ненужные объекты. Время обоих вариантов теперь мало и существенно зависит от process startup; 1.117× нельзя переносить на стоимость отдельного объекта.
- `class_alloc`: Rust по-прежнему имеет black_box, Nether теперь устраняет heap allocations. Значение 0.023× — асимметричный диагностический сценарий, **не утверждение, что Nether выделяет память в 44 раза быстрее Rust**. Для свободной оптимизации сравнивать нужно `class_alloc_elidable`.
- Array/LCG/struct остаются близки к Rust. Разницу в несколько процентов по одному хосту не считаем устойчивым преимуществом.

Оптимизации ограничены доказанным subset в wrapping mode. Checked overflow, dynamic division, сложные storage/views, destructors, escaping/Copy/identity-bearing наблюдаемые классы сохраняют fallback. Точные условия — [Rust backend](../docs/0.1/rust-backend.md#консервативные-оптимизации-2026-10-25).

## Компиляция и размер

Медиана 3 полных сборок исходника с прогретым filesystem cache. Nether включает frontend/checker/verifier, codegen, rustc и linker; сборка самого компилятора исключена. Размер — executable целиком, не только text section.

| Тест | Rust сборка, мс | Nether сборка, мс | Rust, KiB | Nether, KiB |
| --- | ---: | ---: | ---: | ---: |
| array_sum | 70.2 | 107.5 | 437.3 | 437.3 |
| class_alloc | 71.4 | 449.4 | 437.3 | 437.4 |
| class_alloc_elidable | 69.7 | 450.1 | 437.3 | 437.4 |
| collatz | 69.2 | 85.4 | 437.3 | 437.3 |
| fib | 66.9 | 78.6 | 437.3 | 437.4 |
| lcg | 66.9 | 78.8 | 437.3 | 437.3 |
| struct_step | 69.6 | 102.0 | 437.3 | 437.3 |

## Проверки и воспроизведение

Полный workspace Debug и Release: по **202 passed, 0 failed, 1 ignored**. Diagnostic runtime: 6 passed. Clippy all-targets/all-features, fmt и документационные ссылки прошли. Native tests сверяют HIR/MIR при O0/O3, включая recursion, arithmetic limits, panic конструктора и loop-carried owners. Отдельный counting allocator подтверждает ноль heap allocations у подходящего frame-класса при O0 и O3.

Дополнительный in-process kernel probe прошёл с полным совпадением u64 результатов в 9 раундах: Rust 3.337 мс, Nether 3.119 мс, отношение 0.935×. [Лог](results/2026-10-25-optimized-kernel.txt).

Повторить: `./performance/run.sh 7 --build-runs 3`. Новые данные: `performance/out/results.json`. Микробенчмарки на одном хосте без фиксации CPU/температуры и полной изоляции фоновой нагрузки. Peak RSS и performance checked-overflow mode не измерены. Старый прогон 2026-10-23 использовал другую методику; он сохранён [отдельно](results/2026-10-23.md).
