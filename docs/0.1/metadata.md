# Metadata и вывод контрактов 0.1

Контракт этапа 0. Schema/algorithm определены здесь; serializer и checker пока
не реализованы. Library artifact — native object/archive плюс UTF-8 JSON `.nmeta`.

## Schema version 1

```text
format_version, compiler_contract, runtime_contract, target, abi_version,
package {name, source_id, content_id}, dependencies[], types[], functions[],
constants[], imports[], generic_bodies[]
```

До imports/link проверяются version/target/ABI, limits размера и nesting,
duplicates/dangling references и согласованность schemas. Неизвестные semantic
fields/version отвергаются. Package IDs проверяются по lock; package cycles
запрещены. IDs declarations — package/module/local ID; порядок encoding
deterministic. Metadata не должна молча ослабляться ради успешного link.

Type records: kind, generics/constraints/defaults, members/visibility,
receiver kinds, fields/variants, Copy validity, destructor symbol/effects,
baseline layouts и representation shape. Shape отдельно описывает owned/view
компоненты, storage mode, provenance и weak domain; source type T их не стирает.

Origins: symbolic root `arg[n]`, receiver, static, storage slot, weak domain
и projection path (field ID, variant/payload, const index, unknown index,
deref). Unknown index включает все возможные slots этого storage. Unknown weak
provenance включает весь weak-addressable domain. Во внешних contracts нет
ссылок на частные caller-local IDs.

Function record: signature/access modes, available entry keys, reasons для
недоступных entries, symbol, effects, result contract и generic body ID.
Entry key — modes ordinary args + storage shapes. Receiver остаётся borrowed,
Copy args материализуются owned. Function pointer фиксирует конкретный entry.

Result: `owned(shape)` или `view(access, origins, invalidations)`; component
dependencies сохраняются и внутри owned оболочки. Разные ветви должны иметь
единый доказанный result mode; runtime owner/view union не вводится.
Effects: read/write/consume/initialize/replace/drop/free, escape в result/field/
storage, invalidation, group membership, weak lookup/register, panic и
destructor accesses. При вызове symbolic paths подставляются actual places.
Generic typed body поставляется для specialization; ordinary caller проверяется
по metadata без чтения private body. Module imports не запускают runtime init.

## Рекурсия

1. Собрать signatures всего пакета до bodies. Разрешить cycles declarations,
   отдельно отвергнуть cycles constant evaluation и бесконечного inline layout.
2. Построить call graph и SCC для candidate entries. Начальное состояние —
   unknown contract, а не вымышленный безопасный borrowed entry.
3. Локальные операции дают constraints прав, moves, result origins/effects.
   Recursive call ссылается на unknown summary; это не доказательство safety.
4. Решить mode/shape equalities по всему SCC. Defaults owned применяются после
   всех constraints, не по первому посещённому call. Конфликт исключает entry
   с diagnostics обеих причин.
5. Origins/effects — конечный домен symbolic paths specialization. Recursive
   `arg.next.next...` расширяется до wildcard root; precision теряется, safety
   сохраняется. Propagation монотонна (union), недоступные entries исключаются.
6. Повторять propagation и проверку bodies до fixed point. После convergence
   повторно проверить оставшиеся bodies/call sites против окончательных
   summaries, включая cleanup exits. Непроверенные предпосылки о callee не
   становятся автоматически достоверными contracts.
7. Result mode доказывается на всех reachable returns; неизвестный mode/provenance
   даёт diagnostic. Бесконечная recursion без return не создаёт фиктивный owned
   результат. Недоказуемый recursive storage shape отклоняется.

Specializations memoize stable IDs. Resource limit на разрастание compiler
выдаёт явную ошибку, не успешную частичную metadata. Нет runtime выбора entry,
refcount для обхода recursion или обращения к неинициализированному result
на nonreturning path.

## Граница доверия

Native object и metadata должны быть из согласованной trusted build. Signature
validation не доказывает безопасность враждебного native object. Unsafe wrapper
отвечает за raw resource protocol; importer проверяет доступные ему ordinary
ownership/lifetime contracts. Изменение consuming/cleanup/return effects может
ломать API при прежней source signature и требует пересборки dependents.
