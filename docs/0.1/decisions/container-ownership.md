# Решение: статические ownership-варианты generic storage

Статус: вариант A утверждён пользователем 2026-10-06. Варианты B и C
сохранены как отклонённые альтернативы. Реализация использует статические
owned/readonly-view специализации без смешивания режимов в одном storage.

## Основание

Основная спецификация §14 говорит, что borrowed элемент `Vec<T>` остаётся view
на внешнего владельца и удаление такого элемента не уничтожает цель.
§24 допускает собственный mutable контейнер с readonly views. §94–95 требуют
статического выбора ownership и сохранения прав/lifetime без скрытого clone.

Roadmap §4.2 одновременно требует: `push` статически принимает owned аргумент,
`pop` возвращает owned значение; borrowed **компоненты** T сохраняют lifetime.
В матрице памяти roadmap отдельно указан «контейнер с borrowed T».

Это допускает два разных прочтения: прямой borrowed User как элемент или
owned оболочка, у которой поле содержит borrowed User. Их storage, допустимые
вызовы push/pop/get_mut и экспортируемые contracts различаются. Объявить
pop всегда owned при хранении прямого non-Copy view было бы ошибкой soundness.

## Различающий пример

Ниже предполагается библиотечный `Vec.create()` и `Option.Some/None`.
Это acceptance scenario для будущей библиотеки, а не готовая программа.

```nether
class User {
    id: i32
    constructor(value: i32) { this.id = value }
    fn read(this): i32 { return this.id }
}

fn example(): i32 {
    let source = new User(7)
    var values = Vec<User>.create()
    values.push(source)
    let extracted = values.pop()
    let original_id = source.read() // будущий use: source не передан в push
    return match extracted {
        Option.Some(value) => value.read() + original_id,
        Option.None => 0,
    }
}
```

`source` non-Copy. Здесь нельзя одновременно считать push consuming,
разрешить последующий source.read() и объявить извлечённый User новым owner.
Clone/refcount не являются допустимым способом согласования.

## A. Статические варианты хранилища (принято)

Compiler выводит один режим **элементов** для конкретного storage: owned или
borrowed с известным access/lifetime contract. Source type остаётся `Vec<T>`,
пользователь не пишет owns/view. Разные варианты могут иметь разные внутренние
представления и специализации методов. Сам descriptor/buffer контейнера owned
независимо от режима элементов.

| Операция | Owned storage | Borrowed storage |
| --- | --- | --- |
| push | Требует owned T | Сохраняет допустимый view, связывает storage с целью |
| pop/remove с результатом | Передаёт owned T | Возвращает view с lifetime внешней цели |
| cleanup | Уничтожает owned T | Убирает view, не уничтожает внешнюю цель |
| get_mut | Mutable view на owned элемент при unique access | Не повышает readonly право внешней цели |

Для начала borrowed storage может хранить только readonly views; mutable view
storage потребует отдельных unique-access правил. Мутация состава readonly-view
контейнера не даёт mutable-доступ к целям. В примере выше результат — 14,
владелец User — source до конца функции; контейнер освобождает только свой buffer.

Смешивание owned/view элементов и присваивание несовместимых storage modes
запрещаются статически. Пустой контейнер получает режим из ограничений всех
использований, а не из первого runtime push. При недостаточных ограничениях
принимается owned; при противоречащих — diagnostic. Mode и borrowed component
provenance входят в metadata, generic specialization и function contracts.

Это уточняет §4.2 roadmap: его owned push/pop относятся к owned specialization.
Нет per-element runtime ownership tag. Для borrowed inline T storage хранит
view descriptor, а не копирует payload. Утверждение §14 об inline struct elements относится к owned specialization;
borrowed specialization хранит descriptors и не копирует payload.

## B. Только owned элементы в 0.1 (отклонено)

§4.2 применяется буквально: каждый slot владеет T. Внешний view можно хранить
внутри owned wrapper T, если проверены lifetime и права его полей.
Прямой borrowed User в примере отвергается на push, потому что source нужен ниже.
`pop` всегда передаёт owned T, включая его borrowed поля, которые не становятся
самостоятельными владельцами внешних целей.

Это наиболее простой контракт storage, но сужает §14 основной спецификации
для 0.1: прямые borrowed элементы нужно явно исключить из профиля и уточнить
acceptance matrix. User не должен получать сообщение, будто такие программы
приняты с другой семантикой.

## C. Смешанные owned/view элементы (отклонено)

Один buffer может содержать разные режимы элементов, в том числе выбранные
ветвлением и неизвестным index. Требуются per-slot cleanup state и контракт
для результата pop, режим которого неизвестен статически. Одних обычных
initialization flags недостаточно для объявления результата owned.

Нужно отдельно выбрать представление такого результата, права мутации,
возможность передачи в consuming function и правила экспорта metadata.
Runtime tags сами по себе не обеспечивают lifetime внешних целей.
Этот вариант меняет больше правил и не рекомендуется как неявное упрощение 0.1.

## Почему нельзя решить только внутри backend

Выбор меняет допустимые исходные программы, способность pop возвращать
ownership, layout borrowed inline elements и число destructors. Он определяет
контракты `initialize/read/drop/view` для произвольного пользовательского
контейнера, не только Vec. Поэтому это решение семантики этапа 0, а не способ
оптимизировать уже определённое поведение.
