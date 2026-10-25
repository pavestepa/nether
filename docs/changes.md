# Nether 0.1 — изменения базовой линии

2026-10-25. Эта редакция заменяет прежний draft 0.1.

- NMM `nmm-0.1` — единственная модель памяти.
- Existing non-Copy binding создаёт alias, а не move по последнему use.
- O, R, W независимы; readonly не превращается в writable через ownership.
- Non-Copy `var` parameter — borrow; consuming ordinary parameter требует O.
- Убран обход consuming-effect через borrowed entry для alias.
- Добавлены unsafe fn и явные unsafe blocks; безопасные NMM проверки сохраняются.
- Unsafe callable пока нельзя превратить в safe function value.
- Старые альтернативные memory rules, планы и устаревшие acceptance counts
  заменены единым [планом](0.1/mvp_plan.md) и [статусом](0.1/status.md).

Raw operations/FFI, weak, closures/channels/async и полный MIR verifier
не объявляются готовыми. Контракт unsafe — [unsafe.md](unsafe.md).
История прежней редакции доступна в Git; она не является действующей спецификацией.
