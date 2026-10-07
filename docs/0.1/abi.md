# ABI и panic protocol baseline 0.1

Контракт этапа 0 для `x86_64-unknown-linux-gnu`; object representation —
[layout.md](layout.md). ABI version 1 привязан к toolchain и metadata version.
Внутренние вызовы, borrowed аргументы, C main wrapper, allocator и panic reporting
реализованы для текущего subset. Resource cleanup, borrowed returns, C boundary,
syscall и сериализация metadata ещё не реализованы. Пока CLI компилирует встроенный
C runtime при linking; отдельный runtime archive остаётся целевым форматом.

## Внутренний вызов

Baseline использует явные success/panic edges. LLVM signature entry:

```text
i1 symbol(ptr panic_context, ptr result_slot, ptr arg0_slot, ptr arg1_slot, ...)
```

True означает normal return с initialized result_slot (unit не требует записи).
False означает panic с uninitialized result_slot. Все slots имеют target layout.
Owned argument consumed при входе: его cleanup на обоих выходах делает callee;
borrowed остаётся у caller. Receivers всегда borrowed. Arguments вычисляются
слева направо; failure до входа очищает только уже созданные owned temporaries.
Result slot не alias inputs; noalias LLVM attributes не выводятся из одного
наличия pointers. Function pointer фиксирует entry и эту signature; runtime
выбора owned/borrowed entry нет. Статические storage shapes входят в entry key.

Symbol: `_N1_` + hex UTF-8 package/module/name с `_` separators + deterministic
specialization/entry ID из metadata. Host randomized hash не является ABI ID.
Collisions диагностируются. Cross-toolchain binary stability не обещается.

## Panic и unwind

Panic context передаётся из startup. C-compatible fields в порядке объявления:
`active: u32`, `kind: u32`, `file: *u8`, `line: u32`, `column: u32`,
`message_len: usize`, `message: {u8; 192}`. File — static UTF-8 path, message
копируется без allocation и обрезается по UTF-8 границе. Kind: user=1,
bounds=2, overflow=3, division=4, shift=5, allocation=6.

Panic origin устанавливает context и переходит к generated cleanup blocks.
Caller, получив false, очищает свои initialized roots/temporaries и возвращает
false. Это stack unwinding через обычные returns, без native exception
personality. Panic не пользовательский Result и не может быть проигнорирована.

Порядок §69: roots обратно initialization, destructor body до owned полей
по объявлению, arrays/tuples по индексу, enum по active payload, group обратно
принятию ownership. Drop flag сбрасывается **до** вызова drop. Moved/неинициализированные
части пропускаются. Panic из destructor при обычном cleanup продолжает cleanup
его оставшихся полей и живых roots. Повторная panic при active=1 — немедленный
abort. Failure edge внутри destructor ведёт к remaining-field cleanup, а не
просто возвращает false, пропуская поля. Active не сбрасывается при propagation.
Пользовательского catch-panic в 0.1 нет; ожидаемые ошибки — enum Result.

OOM использует тот же allocation-free panic context и cleanup; failing allocation
не выдаёт capability. Непоправимая ошибка runtime/double panic — abort. Эта схема
имеет status checks на calls; оптимизация к native unwind/register arguments
может менять ABI version, но не наблюдаемую semantics.

## Entry и linking

Source entry: public `fn main(): ()` либо `fn main(): i32` в entry module,
ровно один, без arguments/generics. Generated C main создаёт context и вызывает
Nether main. Normal exit возвращает 0/unit либо i32 status; panic печатает
сообщение в stderr и завершает status 101 после cleanup.

Build: LLVM IR → target object → target C linker driver + runtime archive.
Linker path/sysroot настраиваются явно; аргументы передаются argv, не shell
code. Unsupported target, несовместимая metadata и отсутствующий toolchain
диагностируются. На macOS Linux ELF не запускается: acceptance в Linux CI/VM.

Runtime C symbols prefix `__nether_v1_`:

```text
allocate(size: u64, align: u64) -> ptr    // null при OOM, не throws
deallocate(base: ptr, size: u64, align: u64) -> void
abort_double_panic(context: ptr) -> never
report_panic(context: ptr) -> void       // без heap allocation
```

Zero-size storage использует aligned sentinel и не вызывает ordinary free.
Compiler проверяет size/alignment до входа. Runtime не обходит graph и не
считает owners. Диагностический allocation log не часть обычного view access.

## Очередь cleanup roots

Принято [обновление порядка при замене root](decisions/root-cleanup-order.md).
Runtime предоставляет ограниченную числом активных roots intrusive очередь.
Scope содержит `last: ptr`; root содержит пять pointers в порядке
`scope, previous, next, drop, value` (8 и 40 bytes, alignment 8). Scope и links
root обнуляются до первого использования. Records принадлежат compiler frame;
они не выделяются в heap и не продлевают lifetime values.

`drop` имеет C-compatible signature `bool drop(Panic*, void*)`; generated adapter
вызывает внутренний entry destructor и cleanup owned полей. Поля и их drop flags
остаются ответственностью generated drop glue, runtime их не обходит.

```text
cleanup_push(scope, root) -> void       // только после успешной инициализации
cleanup_forget(root) -> void            // move без destruction; unlinked = no-op
cleanup_root(panic, root) -> bool       // unlink до drop; unlinked = success
cleanup_scope(panic, scope) -> bool     // последние инициализации первыми
```

Все symbols имеют prefix `__nether_v1_`. Перед заменой occupied root его старое
значение уничтожается; после успешной записи root снова добавляется в свой
owning scope. Запись поля не вызывает push whole root. Нельзя повторно push
linked root: это нарушение internal ABI. Перенос между владельцами делает
forget source и push destination, не переносит запись вместе с value.

Scope cleanup продолжает оставшиеся roots после первой panic. Drop, вернувший
false, обязан установить panic context. False при уже активной panic вызывает
abort. Успешный cleanup не сбрасывает active panic. False от cleanup сообщает
о новой failure во время этого cleanup; propagation исходной panic остаётся
за generated caller.

Runtime queue реализована и отдельно тестируется. Подключение generated drop
glue к source destructors и partial initialization ещё не завершено.

## C boundary и syscall

Plain C functions используют target C ABI. Первый subset: i8–i64/u8–u64,
f32/f64, matching usize/isize, raw/function pointers, unit return,
`#[repr(C)]` POD structs из этих полей. i128/u128, bool и char требуют явного
преобразования через поддержанные типы. Classes, обычные tuples/enums/str не
передаются. Aggregate argument/return classification следует
[x86-64 psABI](https://gitlab.com/x86-psABIs/x86-64-ABI), не одному размеру.

Export syntax: `extern "C" fn name(args): R { ... }`. C adapter вызывает
internal Nether entry, при panic abort до возврата в C. Link name — export name,
duplicates запрещены. Module re-export не меняет ABI. Foreign exceptions/longjmp
через живые Nether frames запрещены. Retention/callbacks после возврата требуют
явного unsafe resource protocol.

Unsafe syscall intrinsic: `syscall(number: usize, args: {usize; 6}): isize`.
Lowering — системная инструкция target с правильными registers/clobbers/memory
effects; libc syscall не заменяет этот критерий. Platform wrappers учитывают
negative error codes, partial results, bounds и lifetime buffers.
