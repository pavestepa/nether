# Обновление концепции — 2026-10-13

Nether теперь генерирует Rust; прямой LLVM backend удалён из проекта. Модель памяти сохранена. Class lowering: payload struct + trait + Box, static specialization по возможности и dyn при необходимости. Внутренний unsafe разрешён; обычный rustc подходит. Цель скорости — сопоставимый handwritten Rust. Подробности: [контракт](0.1/rust-backend.md).

Ниже сохранена история предыдущих обсуждений; она не отменяет актуальную спецификацию.

Обновил [docs/main.md](main.md): внёс утверждённые правила, согласовал примеры, заменил void на (), убрал this. Проверил структуру документа, ссылки между разделами и блоки кода. Вопрос 9 оставлен открытым в [§93 (line 2455)](main.md).
Для освобождения памяти долгоживущих структур вижу следующие варианты. Это предложения, в документации они пока не утверждены.
Вариант	Когда освобождается память	Новое правило	Цена
Единственный владелец элемента	При удалении или замене owned элемента	Используемый view запрещает уничтожение владельца	Статические ограничения; работа allocator и destructors, без обязательных проверок обычных views
Отдельные регионы для групп объектов	При уничтожении всей группы	Связанные объекты имеют общий lifetime; внешние views не переживают группу	Metadata регионов и удержание удалённых объектов до освобождения группы
Поколения / snapshots	После завершения всех разрешённых обращений к старому поколению	Views ограничены известной фазой или scope	Временное хранение нескольких поколений, задержка освобождения, возможное перестроение данных
Удаляемые узлы с handles	При удалении конкретного узла владельцем хранилища	Долгоживущие связи допускают исчезновение цели	Metadata, косвенный доступ и проверка действительности handle


Я рекомендую начать с единственного владельца элемента, дополнив его регионами для связанных групп.
Для cache пользовательский код остаётся обычным:
let user = cache.get(id)
println("{}", user.name)
cache.remove(id)
Компилятор доказывает, что view больше не используется, и разрешает уничтожить элемент. Обратный порядок отклоняется:
let user = cache.get(id)
cache.remove(id)
println("{}", user.name) // ошибка: цель view уже удалена
Новое правило: owned элемент контейнера имеет самостоятельное владение, а удаление завершает его lifetime. Не нужно ждать завершения ApplicationRegion. Компилятор выбирает подходящее хранилище — например, pool с повторным использованием освобождённых мест.
Цена здесь — ограничения на сохранённые ссылки. Если другой объект или async operation продолжает использовать view, удаление нельзя разрешить до завершения этого доступа. Когда независимость элементов невозможно доказать, проверка может быть консервативной.
Для графов нужно отделить связи графа от владения памятью. Связи могут образовывать циклы, а владение остаётся однозначным. Например, весь граф принадлежит одному контексту: при уничтожении графа освобождаются все узлы независимо от циклов. Однако удаление одного узла ещё не освобождает его память, если используется общий регион.
Если нужна частая независимая ликвидация узлов, подходят handles и утверждённый weak: удаление делает ссылку недействительной, обращение возвращает Option. Аналогичный подход применяется в slotmap для графов и объектов со стабильными ключами. Это требует проверки доступа и не определяет автоматически, какие узлы стали недостижимыми — удаление остаётся операцией владельца хранилища. Документация slotmap.
Бесплатный способ есть для случаев, где lifetime можно доказать статически. Тогда не нужны счётчики ссылок, обход достижимости или проверки каждого обращения; остаётся необходимая работа освобождения памяти.
Для произвольного изменяемого графа одновременно сохранить неограниченные долгоживущие ссылки, автоматически освобождать недостижимые узлы и обойтись без runtime-учёта нельзя обещать. Придётся выбрать ограничение ссылок, освобождение группами или динамический механизм.
Для идеологии Nether наиболее подходящее правило: простые views проверяются статически; удаление owned элемента освобождает его после завершения разрешённых заимствований; динамически исчезающие цели доступны через явный weak.
## 2026-10-15 — C2: source classes и construction

Подключены class/new/constructors к HIR/MIR и Rust backend. Typed CFG проверяет
инициализацию полей через aliases и ветвления. Работают recursive и generic classes,
methods/views, identity-preserving move, независимый Copy и partial construction
cleanup. Native tests проверяют порядок destructor calls, allocation/free balance,
identity и panic при O0/O3. Borrowed returns и aggregate contracts — следующий C3.


## 2026-10-15 — C3: borrowed results и entry contracts

Реализованы статические borrowed return contracts, capability/provenance через
несколько вызовов, generics/imports, if/match и recursive groups. HIR/MIR и Rust
возвращают view без нового владения. Returned aliases сохраняют finalizing effects;
panic cleanup не уничтожает чужую цель. Lifetime diagnostics включают исходного
владельца, создание view и последующее использование. Non-Copy owned function
values работают с indirect argument/result cleanup; borrowed callable metadata
остаётся открытой. Исправлен recursive class/enum layout при обратном порядке
объявлений. C3.2/C3.4 и C4–C9 ещё не завершены.


## 2026-10-15 — C3.2: локальное readonly aggregate storage

Structs, tuples, fixed arrays и enum payload сохраняют readonly views non-Copy
значений без clone/ownership tag. MIR отслеживает provenance каждого компонента,
включая вложенные оболочки, перемещения и перепривязку поля. HIR/MIR/native cleanup
не уничтожает borrowed target; owned соседние поля и partial failure сохраняют
обычный порядок уничтожения. Добавлены native O0/O3 проверки результата,
destructor trace и allocation/free balance. Исправлены проверки constructor
initialization через сохранённые aliases и diverging return expressions.
Argument/result contracts aggregate проходят через functions, methods, generics,
imports/reexports и рекурсию. Для каждого borrowed компонента выводятся отдельные
origins; descriptor slot и внешний target имеют разные lifetime. Class fields,
mutable storage write contracts и пользовательские destructors оболочек остаются
открытой частью C3.2. Пример: `examples/borrowed_storage/main.nr`.


## 2026-10-18 — C3.2: mutable borrowed storage contracts

Mutable borrowed aggregate parameters и receivers могут переназначать readonly
поля. Для selected entry выводятся may-write contracts с target/component paths
и origins параметров. Их подстановка работает через nested calls, generics,
imports/reexports и рекурсию, на normal и panic edges. Callee-local/owned targets
не могут сохраниться в caller storage. Dynamic indices не экспортируют local IDs;
пути через class payload сохраняют зависимость от владельца. Ранее извлечённые
views продолжают указывать на прежние targets. Runtime ABI не получает tags,
allocations или hidden clones.

Проверка консервативна: прежние зависимости поля после вызова не снимаются даже
при гарантированном overwrite. Class fields, destructors borrowed оболочек и
полный вывод modes/refinement остаются открытой частью C3.2. Пример:
[borrowed_storage_write](../examples/borrowed_storage_write/main.nr), результат 10.

Проверка: workspace Debug/Release — по 153 tests, 0 failures, один opt-in probe
ignored; 71 borrowed-storage program при O0/O3 (19 новых сценариев), с точными
cleanup traces и allocation accounting. Clippy all-targets/all-features без
warnings, formatting, whitespace и ссылки документации проверены.


## 2026-10-20 — C3.2: уточнение локальных enum variants

Known enum discriminants и scalar constants переносятся по MIR до fixed point.
Construction/move/root replacement и совпадающие joins сохраняют знание; calls и
projected/aliased writes его сбрасывают. Доказанно недостижимые переходы удаляются
до lifetime/initialization/result/effect checks и Rust emission. Замена на пустой
variant больше не требует origin для payload в невозможной match-ветви. Разные
возможные variants и loops остаются безопасно консервативными; source type errors
проверяются во всех ветвях. Покрыты imports/storage specialization, guards,
неисполняемые writes/returns и сохранение lifetime проверок после alias/call writes.
Пример: [enum_refinement](../examples/enum_refinement/main.nr), результат 3.

Проверка: workspace Debug/Release — по 154 tests, 0 failures, один performance
probe opt-in/ignored; 83 borrowed-storage programs × O0/O3 (12 новых сценариев).
Clippy all-targets/all-features без warnings, formatting, whitespace и ссылки
документации проверены. Пример даёт 3 в eval и native release.


## 2026-10-20 — C3.2: локальный вывод static storage modes

Прямые поздние присваивания named locals уточняют owned/readonly representation
их initializers до fixed point. Поддержаны nested struct/tuple/array fields,
whole wrapper replacement, связанные wrapper bindings, shadowing и последующие
generic/imported calls. Provisional entries откатываются при повторной проверке.
Массив согласует mode всех элементов, сохраняя порядок, однократное runtime
вычисление и numeric contextual typing. Перепроверка не превращает owning
temporaries в views и не добавляет hidden allocations/clones/tags.

Полный вывод через параметры/arbitrary aliases и сложные initializer contexts
остаётся открытым; borrowed class fields и destructors оболочек также ещё не готовы.
Пример: [storage_modes](../examples/storage_modes/main.nr), результат 11.

Проверка: workspace Debug/Release — по 155 tests, 0 failures, один performance
probe opt-in/ignored. 103 borrowed-storage programs × O0/O3 (20 новых сценариев),
точные cleanup traces и allocation accounting; Clippy all-targets/all-features
без warnings, formatting, whitespace и ссылки документации. Пример даёт 11
в eval и native release.


## 2026-10-20 — завершение C3: borrowed class и callable contracts

- Borrowed class payload выбирается до allocation; constructor, методы и destructor
  специализируются по одному physical layout. Canonical recursive class edge
  остаётся непрозрачным и не разворачивает Type бесконечно.
- User destructors borrowed оболочек участвуют в implicit liveness. Cleanup-order
  analysis учитывает scopes, construction order, root initialization/replacement,
  projected writes, may-write calls, joins/loops и panic. Внутренние группы — C7.
- Storage modes выводятся через параметры, mutable aliases, returned values,
  if/match и function values; requested signatures отделены от inferred ABI.
- Plain function pointers сохраняют owned/borrowed ABI, result/component origins и
  may-write effects. Работают unbound receivers, generics/higher-order functions и
  returned/selected pointers. Metadata существует только в compiler; runtime value
  содержит один адрес кода. Несовместимые result ABI отвергаются без hidden adapter.
- Добавлены native O0/O3 проверки, imported/reexported positive/negative cases и
  примеры `borrowed_classes` / `borrowed_callables` (оба дают 14).

C0–C3 закрыты; следующий этап — C4. Остальной MVP C4–C9 ещё не завершён.


## 2026-10-20 — C4.1: статические interfaces

Добавлены explicit class/struct implementations, type/const generic interfaces,
defaults, conjunction constraints и проверка generic bodies по abstract method
surfaces. Проверяются public access, readonly/mutable/static receivers,
parameter/result types, alpha-equivalent method generics, defaults и constraints.
Зависимые bounds разрешаются до fixed point; symbolic const signatures не
считаются равными только по совпадению одной числовой specialization.

Static methods, function values, imports/reexports и borrowed method results
используют concrete entries, сохраняя C3 lifetime/storage/effect contracts.
Runtime interface representation, vtable и дополнительные allocations не нужны.
Добавлен [пример interfaces](../examples/interfaces/main.nr), результат 8.
C4.2–C4.4 (class inheritance, upcast, dyn dispatch) ещё не реализованы.


Проверка C4.1: workspace Debug/Release — по 163 tests, 0 failures,
один performance probe opt-in/ignored. Все 23 interface-программы прошли
HIR/MIR comparison и native O0/O3 с проверкой результатов и порядка destructor
calls; отдельно проверены imports/reexports и отрицательные constraints.
Clippy all-targets/all-features с `-D warnings`, formatting, whitespace и
локальные Markdown links проверены. Пример interfaces даёт 8 в eval и native release.


## 2026-10-20 — C4.2a: private member access и generic method values

Private fields/methods теперь доступны только из объявляющего class/struct,
включая constructors/destructors и generic/borrowed entries. Проверяются direct
calls, receiver aliases, method pointers, literals и patterns. Same-module free
functions и другие types больше не имеют доступа к private members. Diagnostics
показывают место доступа и declaration; private top-level functions сохраняют
module visibility.

Добавлены явные type arguments у method function values и корректная проверка
returned generic callable: caller ожидает готовности producer specialization,
при этом function IDs/ABI/contracts не зависят от порядка проверки.
[Пример private_members](../examples/private_members/main.nr) возвращает 7.
Это часть C4.2; embedded base, super и virtual/override ещё не реализованы.


Проверка C4.2a: workspace Debug/Release — по 167 tests, 0 failures,
один performance probe opt-in/ignored. 13 новых программ прошли HIR/MIR
comparison и native O0/O3 с точным порядком destructor calls; отдельно проверены
отрицательные access cases и imports/reexports с declaration notes.
Clippy all-targets/all-features с `-D warnings`, formatting, whitespace и локальные
Markdown links проверены. Пример private_members возвращает 7 в eval и native release.


## 2026-10-20 — declaration layer C4.2b/c

Добавлены AST/parser virtual/override и declaration hierarchy checker: один class
parent первым в списке, запрет cycles, invalid parents и неявного member hiding.
Проверяются override chain, public instance receiver, parameter/result signatures,
method generics/constraints/defaults. Подстановка parent arguments/defaults не
захватывает local method binders или lexical module constants; проверены
многоуровневые generic и const inheritance declarations. Diagnostics сохраняют
source note на базовом методе через imports/reexports.

Root-class virtual methods используют обычные concrete entries; новые native
cases покрывают borrowed results, mutable receivers, interfaces, generic/unbound
pointers и destructor calls. [Пример virtual_methods](../examples/virtual_methods/main.nr)
возвращает 8. Embedded base, super, inherited body lowering и derived/base cleanup
ещё не готовы; C4.2 не закрыт.

Проверка declaration layer C4.2b/c: workspace Debug/Release — по 174 tests,
0 failures; один performance probe opt-in/ignored. Проверены 19 положительных
и 35 отрицательных hierarchy cases, imports/reexports с source notes и семь
новых root-virtual программ в HIR/MIR и native O0/O3 с точным cleanup trace.
Clippy all-targets/all-features с `-D warnings`, formatting, whitespace и
локальные Markdown links проверены. Пример virtual_methods возвращает 8
в eval и native release. Derived-object runtime acceptance остаётся открытым.
