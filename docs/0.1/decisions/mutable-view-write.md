# Запись в цель mutable view

Статус: принят вариант A пользователем (ответ «1»). Предыдущий выбор owned/readonly-view специализаций
контейнеров сохраняется и этим вопросом не пересматривается.

## Место, требующее решения

`main.md` §24 требует: «Переприсваивание borrowed `var` binding меняет только
его локальную привязку. Запись в поле через mutable view изменяет исходный объект».
§88 разрешает `ref var` для scalar payload, а roadmap требует `Vec.get_mut`.
В profile §4 unary `*` пока относится только к raw pointers и требует unsafe.

Для struct/class можно записать `view.field = value`. Для borrowed `i32` нет
поля, а безопасная операция замены самой цели отдельно не определена. Это
влияет на смысл destination place, reborrow, invalidation и library API.
Нельзя незаметно реализовать перепривязку как изменение цели либо наоборот.

## A. Явная безопасная запись через `*` (принято)

```nether
var numbers = {1, 2}
let {ref var item, ..} = numbers
*item = 7
*item += 1
```

Для safe view `*item` обозначает целевое место и сохраняет его provenance и
capability. Readonly view не разрешает запись. Для raw pointer `*pointer`
по-прежнему требует unsafe. Простое `item = other` следует правилам перепривязки
borrowed binding. Правило закреплено в §24/§88 и таблице unary operators.

## B. Запись через явно объявленный `ref var` (альтернатива)

```nether
var numbers = {1, 2}
let {ref var item, ..} = numbers
item = 7
item += 1
```

Явный `ref var` обозначает целевое место; присваивание и compound assignment
изменяют его. Обычный borrowed `var` продолжает перепривязываться по §24.
Это отдельное правило для ref-pattern bindings, которое необходимо закрепить
в спецификации и сохранить в HIR/MIR и diagnostics.

Оба варианта требуют обычной проверки lifetime и уникальности mutable access.
Source checker отклоняет формы, для которых ещё нет полного lowering и проверки
lifetime/уникальности; принятие решения само по себе не означает готовности реализации.
