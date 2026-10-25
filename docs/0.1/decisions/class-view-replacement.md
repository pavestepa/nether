# Class view и owning slot

Редакция 2026-10-25. Частное правило единственной [NMM](../../core-memory-managment.md)
для Nether 0.1; альтернативных вариантов здесь нет.

Class view обозначает identity существующего объекта. W разрешает его поля
и mutable methods, но не замену owning slot через `*view = new C()`.
Замена целого class выполняется владельцем slot или проверенным container API;
identity/cleanup/invalidation остаются частью NMM.
