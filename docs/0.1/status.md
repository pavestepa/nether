# Состояние реализации 0.1

## Завершённый первый шаг этапа 0

- Составлен [синтаксический и числовой профиль](profile.md): type/grammar table,
  разделение blocks/arrays, newline и return, operators, casts и failure rules.
- Подготовлены [ожидаемые parser/typecheck cases](syntax-cases.md). Это спецификация
  тестов: они пока не компилируются и не считаются пройденными.
- Написан `compiler/semantics` — независимый от host эталон integer semantics.
  Есть все 12 integer types, checked/wrapping operations, const checks, shifts,
  comparisons, integer casts и saturating float-to-integer casts.
- Создан Rust workspace без сторонних зависимостей. Его сборка не требует LLVM.
- Реализован [baseline layout](layout.md): размеры примитивов, arrays,
  aggregates и tagged enums, padding, ZST и проверка переполнений. Calling ABI
  этим расчётом не реализован.

Эталон проверяется исчерпывающим перебором пар i8/u8 и проверками границ всех
разрядностей, включая u128/i128, negative shifts, NaN и infinity. Тесты запускаются
в debug и release: поведение Nether не зависит от overflow mode самого Rust.
Это ещё не испытания исполняемых Nether-программ и не проверка модели памяти.

## Этап 0: контракты зафиксированы

Утверждены [статические owned/readonly-view варианты storage](decisions/container-ownership.md).
Их mode constraints реализованы отдельным solver, включая conflicts, branch/call
unification и recursive equality cycles. Runtime tags не вводятся.

- [Storage/provenance/invalidation](storage-contracts.md).
- [Baseline layout](layout.md) и [внутренний ABI/panic protocol](abi.md).
- [Metadata schema и recursive contract inference](metadata.md).
- [Positive/negative acceptance contracts памяти](memory-cases.md).

Это завершение **спецификации этапа 0**, не обещание soundness готового языка:
intrinsics, metadata serializer и полный cleanup ресурсов ещё не реализованы.
Memory cases задают ожидаемый результат; пройденными compiler tests не считаются.

## Текущая работа: этап 1

Идёт реализация исполняемого subset; актуальное состояние перечислено ниже.

## Обновление: исполняемый subset этапа 1

Реализованы и проверяются автоматически:

- UTF-8 lexer/parser со spans и диагностикой исключённого синтаксиса.
- Загрузка `.nr` модулей, imports/re-exports, private declarations, циклические
  ссылки declarations, проверка выхода пути за package root. Пока root — каталог
  entry source; manifest/dependency manager не реализован.
- Типизированный HIR, сигнатуры функций, scopes, forward/recursive calls,
  let/var, return, if, while, break/continue, tuple и fixed arrays.
- Числовые операции и casts, bounds checks, checked/wrapping режимы, pure
  constants с forward references, циклами, overflow и вычисляемыми array lengths.
- Именованные Copy и non-Copy structs и enums без resource cleanup, вложенные layouts,
  поля, constructors enum payload, проверка private struct fields.
- Match со scalar/tuple/array/struct/enum patterns, guards и проверкой полноты.
  Payload читается только после проверки tag. Guards допускают проверенные Copy-value calls; изменение binding в guard
  отклоняется. Полные call effects появятся вместе с ownership/FFI. Для integer/char нужен fallback; для составных
  patterns пока нет полного matrix coverage алгоритма.
- LLVM 18 lowering и ELF object emission, C-driver linking на Linux, main wrapper.
  Внутренний success/panic protocol и panic reporting есть; resource cleanup
  ещё не реализован. Non-Copy inline values пока не имеют destructors/heap ресурсов.
- CLI, reference execution, настоящая проверка IR Clang при `-O0`/`-O2` и
  сравнение завершения backend-программ с интерпретатором на Linux x86-64.

Этап 1 целиком не закрыт: остаётся дальнейшая интеграция с этапом владения.
Classes, destructors, function pointers, FFI/unsafe и storage intrinsics пока
отклоняются. Instance methods structs и borrowed function entries реализованы
в пределах текущего inline subset.
Constants пока не поддерживают nominal aggregates. Reference execution имеет
fuel/call-depth limits; parser и type resolver имеют защитные ограничения глубины.
Эти ограничения не являются обещанием окончательного профиля 0.1.

Этапы 2–8 целиком не считаются реализованными. Memory acceptance contracts пока не
пройдены compiler/runtime тестами. Нативные сравнения проверяют результат/код
ошибки завершения, а не журнал allocation/drop будущего runtime.


## MIR и специализации

- LLVM теперь получает код через CFG-based MIR. В нём есть places с field,
  variant payload, constant/dynamic index projections и явные success/panic edges.
- Отдельный MIR interpreter сверяется с HIR interpreter и native execution.
  Реализованы backward liveness и definite initialization на CFG, включая loops
  и различие успешной записи и unwind path.
- Loan conflict domain проверяет пересечение нормализованных origins, unique
  mutable access, readonly capability, reborrow и invalidation владельца.
  Для локальных inline views подключён CFG-анализ origins;
  source moves non-Copy inline values подключены; class ownership ещё не реализован. Эти tests не закрывают
  memory acceptance matrix.
- Generic functions и inline aggregate types специализируются с type/const
  arguments, defaults и inference через nominal arguments. Поддерживается Copy
  constraint; interface constraints ещё отклоняются. Статические методы structs
  используют тот же путь, включая generic arguments и private visibility.
- Для использованных generic функций дополнительно проверяется тело с непрозрачными
  типами параметров, чтобы конкретная числовая подстановка не разрешала неоговорённые
  операции. Const parameters при этой проверке уже конкретизированы. Полная проверка
  абстрактных ownership/effects и интерфейсов остаётся частью следующих этапов.
- Есть защитные лимиты специализаций и сложности типов; бесконечно растущая
  generic recursion диагностируется. Generic type constructors пока требуют
  явные arguments или готовый expected type; общего field-driven inference ещё нет.


## Текущая граница реализации

Явный MIR Move подключён к path-sensitive initialization, MIR interpreter и
проверке перед LLVM. Проверяются partial moves, повторная инициализация,
branch joins и unknown indices. Source ownership inference выдаёт
эти операции для non-Copy inline значений; class resources пока отклоняются.

Pattern extraction не вставляет Copy при необходимости view: tuple/array/struct
bindings с дальнейшим использованием источника и в loops получают локальные views.
Явные `ref` / `ref var` bindings поддержаны для локальных inline places, в том
числе field/index; индекс фиксируется при заимствовании. Borrow и dereference
представлены в HIR/MIR и LLVM; native view хранит pointer без счётчика ссылок.
Проверки используют CFG liveness и provenance, сохраняют readonly capability,
различают disjoint fields/constant indices, проверяют reborrow и границы scopes
включая return/break/continue. Перепривязка view сохраняет старую цель.

Принят и подключён [способ записи в цель view](decisions/mutable-view-write.md):
безопасные `*view = value` и compound assignment; обычное присваивание перепривязывает
view. Поддержаны views в match/enum extraction: readonly guard bindings проверяются
до создания mutable body bindings. Borrowed function entries реализованы;
возвращаемые/хранимые в агрегатах views и source moves для heap ресурсов ещё нет. Смешанные
consuming/ref declaration patterns и циклическая перепривязка reborrow в loops консервативно
отклоняются. Генератор проверяет 4096 последовательностей field move/reinitialization
отдельно от native resource tests. Остальные незавершённые пункты roadmap сохраняются.

## Вызовы, методы и runtime

- Вызовы non-Copy аргументов создают статические owned/readonly/mutable entries.
  Borrowed entry сохраняет ownership у caller; mutable запись меняет исходные
  данные. Проверяются конфликтующие аргументы, readonly capability и consuming-only
  тела. Recursive entries переиспользуются. Выбор по будущим использованиям пока
  консервативен, особенно при повторной инициализации и в loops.
- Struct instance methods используют borrowed receiver даже для Copy типа.
  Поддержаны generic receivers, краткие имена полей/методов, shadowing и временный
  receiver. Возврат non-Copy borrowed результата пока не реализован.
- HIR и MIR interpreters передают views между frames через общие host slots;
  завершение storage явно инвалидирует значение. Это модель эталонного выполнения,
  не runtime refcount скомпилированной программы. Native views — обычные pointers.
- C runtime реализует aligned allocation/deallocation, zero-size sentinel,
  allocation-free panic reporting и abort при повторной panic. Diagnostic mode
  проверяет allocation identity/size/alignment и считает живые allocations.
  Source panic locations пока не заполняются; user panic intrinsic ещё не подключён.
- CLI связывает встроенный runtime с executable, публикуя output только после
  успешного linking. Diagnostic allocator тестируется на Linux, включая double free.

Debug/Release workspace tests, formatting и Clippy без warnings прошли.
Все 276 сравнений HIR/MIR/Linux прошли: 69 программ в checked/wrapping
режимах при `-O0`/`-O2`. Diagnostic allocator test также прошёл. Resource drop
matrix в это число не входит.

Принято [решение о замене class через view](decisions/class-view-replacement.md):
object view изменяет поля существующей identity, а замена class целиком проходит
через владельца или явный `replace`. Отдельный slot view в 0.1 не вводится.

Принято [правило порядка замены roots](decisions/root-cleanup-order.md): успешная
замена и повторная инициализация обновляют позицию root в его owning scope.
Запись отдельного поля не переставляет aggregate root. Resource cleanup реализуется
с этим правилом; решение само по себе не означает готовность destructors.
