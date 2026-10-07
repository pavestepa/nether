# Замена class через mutable view

Статус: принят вариант 1 пользователем (ответ «1»). Реализованное решение
[mutable-view-write](mutable-view-write.md) сохраняется: safe `*view` пишет в цель,
обычное присваивание перепривязывает binding. Для локальных inline значений это
уже представлено и проверяется в HIR, MIR и LLVM.

## Уточнённая граница

`main.md` §18/§21 определяет class view как ссылку на identity объекта,
а не на место, в котором хранится owning reference. `layout.md` задаёт class
reference одним pointer. `storage-contracts.md` §4 сохраняет storage dependency
у class view, но не говорит, что view даёт возможность заменить owning slot.

Для scalar/inline `*view = value` заменяет значение в исходном месте. Для class
есть два разных места: owning slot с reference и payload объекта с identity.

```nether
var owner = new Node()
let ref var current = owner
*current = new Node()
```

Замена reference в `owner` завершила бы старую identity и потребовала знания
owning slot. Запись payload по object pointer сохранила бы старую identity и
не является тем же действием. Подмена одного другим нарушает equality,
weak handles и destructor semantics. Обычный `current = other` остаётся
перепривязкой локального view при любом решении.

## 1. Object view не заменяет class целиком (принято)

`*view = value` разрешено для inline target places. Class view даёт доступ к
полям/методам существующей identity, но не к замене owning reference. Пример
выше диагностируется. Заменять class можно через владельца или явную операцию
контейнера `replace`; сохранённые views проверяются на invalidation. `get_mut`
для class возвращает mutable object view, не capability замены slot.

Это сохраняет единый контракт object view, включая `this`, обычные aliases и
views, возвращаемые из функций. Generic операция `*target = value` требует
inline-target capability либо использует явную операцию замены storage.

## 2. Различать object view и mutable slot view

Обычные class aliases/`this` ссылаются на identity. `ref var` существующего
owning place и `get_mut` owned storage дополнительно могут создавать slot view
на owning reference. `*slot_view = new Node()` заменяет slot с cleanup старого
значения; `*object_view = new Node()` отклоняется. Статический контракт различия
сохраняется через generics, параметры, returns и metadata без runtime тега.

Тогда необходимо определить forwarding и допустимость преобразований этих
capabilities; одна только логическая сигнатура `T` их не различает. Доступ к
полям через slot view сначала читает текущий object reference из slot.

## Реализация

Class/non-Copy resource lowering реализуется с указанным ограничением.
До готовности всех проверок неподдержанные формы диагностируются. Реализация
локальных inline views и их проверки не зависят от этого выбора.
