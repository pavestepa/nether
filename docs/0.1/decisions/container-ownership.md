# Статические режимы storage

Редакция 2026-10-25. Частное правило единственной [NMM](../../core-memory-managment.md)
для Nether 0.1; альтернативных вариантов здесь нет.

Storage имеет один статически выведенный режим компонентов: owned либо readonly
borrowed descriptor. Owned insertion требует O, borrowed insertion сохраняет
R/origin/lifetime без Consume. Нельзя принимать alias в consuming API через
альтернативный entry. Непотребляющий API может явно сохранить локальный alias
параметра; это reflected в storage/result contracts.

Pop/remove возвращает O только из owned slot; borrowed результат не уничтожает
внешнюю цель. Мутация buffer не повышает R элемента в W. Смешивание
несовместимых modes отвергается; runtime ownership tags не добавляются.
