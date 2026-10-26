# Проверки синхронного ядра NMM 0.1

Матрица относится к реализованному языку. NMM §25 также задаёт требования
к ещё не реализованным weak, closures, channels, async и изменяемым buffers;
их приёмка не закрывается тестами текущего компилятора.

## Граница MIR

`compiler/core/src/mir/verify.rs` вызывается после frontend/checker и при
каждом публичном `rust::emit_mir`. Ошибка возвращается как E0401 до генерации
Rust. Проверяются:

- индексы CFG/locals/callees, проекции places, arity и ownership/borrow ABI;
- невозможность Copy non-Copy значения и повторной передачи O одной операцией;
- initialization, use-after-transfer, loans и readonly/mutable access;
- повторно выведенное происхождение результатов и storage-write effects;
- сохранение provenance в direct и indirect calls;
- типы scalar raw/storage operations, writable admission и запрет readonly → writable casts;
- структура cleanup scopes и достижение cleanup либо return каждым живым
  owner-root на normal/unwind выходе, включая branches и loops.

O соответствует owning local/place, R/W — view/access и StoredView.
Отдельное runtime представление токенов не вводится. Verifier использует
существующие анализы provenance/loans/initialization, поэтому это отдельная
граница проверки, а не независимое математическое доказательство их алгоритмов.
Он не является полным type verifier произвольного внешнего MIR; публичного
формата импорта MIR пока нет. Перенос cleanup между корректными scopes,
все варианты неверной регистрации destructor и эквивалентность оптимизаций
требуют отдельных доказательств/тестов при изменении lowering.

`compiler/core/tests/mir_verify.rs` повреждает уже проверенный MIR: duplicate O,
non-Copy Copy, R → W, move из borrow, потеря result provenance, uninitialized
return, неверные CFG/cleanup/projections и потерянный cleanup normal/panic exit.
Каждый отрицательный случай проверяет также отказ Rust backend.

## Проверенная граница backend/runtime

Generated Rust хранит значения в MaybeUninit и управляет их состоянием явно;
не создаёт второй Rust Drop поверх Nether cleanup. Copy class создаёт новую
identity, transfer переносит handle. StoredView не владеет target.
Проверяемые точки реализации:

| Инвариант | Реализация | Исполняемые проверки |
| --- | --- | --- |
| Инициализация, partial move, replacement | `mir/drop_state.rs`, `rust/lifecycle.rs` | `drop_state.rs`, `initialization.rs`, lifecycle unit tests |
| Первичная panic продолжает cleanup; повторная abort | `mir/drop_plan.rs`, `rust/lifecycle.rs`, runtime | `drop_plan.rs`, lifecycle native tests, runtime tests |
| View не уничтожает target; destructor продлевает loan | `mir/borrow_check.rs`, `mir/drop_borrows.rs` | `borrowed_storage.rs`, `borrowed_destructors.rs` HIR/MIR/native O0/O3 |
| Scalar storage allocation/init/take/drop/release | `check/intrinsics.rs`, `reference_memory.rs`, `rust/operations.rs` | `raw_storage.rs`: normal/panic/OOM native counters, ZST, protocol failures, MIR mutation |
| Allocation/deallocation и class identity | `rust/classes.rs`, `runtime/src/lib.rs` | lifecycle native allocation counters, `classes.rs`, diagnostic runtime |
| Порядок roots и rebind | `mir/cleanup.rs`, runtime intrusive roots | `cleanup.rs`, `nmm.rs` N12/N13, lifecycle native tests |

Пути без префикса в таблице относятся к `compiler/core/src` для реализации
и `compiler/core/tests` для integration tests. Lifecycle unit tests находятся
в `compiler/core/src/check/lifecycle.rs`.

Runtime требует стабильных адресов linked records, одинаковых layout при
allocate/deallocate и отсутствия доступа после освобождения. ZST sentinel
не освобождается; invalid layout/OOM дают null. Эти условия сохраняются как
обязательства trusted backend; тесты не доказывают отсутствие любого UB.
Miri/sanitizers, non-scalar/borrowed storage и FFI не входят в эту приёмку.
Scalar raw operations имеют отдельные reference/native O0/O3 и MIR mutation
проверки. MIR не доказывает raw lifetime/bounds/aliasing: они остаются unsafe
обязательствами автора. В reference interpreters raw addresses используют Weak
storage, но это не реализация source weak handles и не полный UB detector.

## Результат прогона 2026-10-25

`cargo test --workspace --offline` и тот же прогон с `--release`: по 210
passed, 0 failed, 1 ignored. Native suites включают O0/O3, exact drop traces
и allocation checks. `cargo test -p nether-runtime --features diagnostic
--offline`: 6 passed. Clippy all-targets/all-features, fmt и локальные ссылки
документации прошли. Ignored performance probe не засчитывается как проверка.
