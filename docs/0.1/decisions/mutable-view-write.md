# Запись через safe inline view

Редакция 2026-10-25. Частное правило единственной [NMM](../../core-memory-managment.md)
для Nether 0.1; альтернативных вариантов здесь нет.

`*view` для safe inline view обозначает target с исходными O/R/W и provenance.
`*view = value` и compound assignment требуют W. Простое присваивание borrowed
binding перепривязывает descriptor. Readonly view не повышается в mutable.
Raw pointer dereference — отдельная unsafe operation, не safe view.
