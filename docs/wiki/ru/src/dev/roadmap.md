# План (ROADMAP)

Полный план: [ROADMAP.md](https://github.com/DanyaDel/Olvin/blob/main/ROADMAP.md) ([English](https://github.com/DanyaDel/Olvin/blob/main/ROADMAP.en.md)). Задачи живут в [issues](https://github.com/DanyaDel/Olvin/issues); галочки в ROADMAP ставятся вместе с кодом.

## Принцип

Движок растёт из игры, а не из списка фич. Вертикальный срез (`apps/vertical_slice`) сделан; его [GAPS.md](https://github.com/DanyaDel/Olvin/blob/main/apps/vertical_slice/GAPS.md) показал, чего не хватало, и все пробелы закрыты.

## Фазы

| Фаза | Тема | Состояние |
|---|---|---|
| 0 | CI, надёжность, документация не врёт | в основном сделано |
| 1 | ядро: ассеты, префабы, версии сцены, состояния, время, события, профилировщик, сохранения | сделано; локализация и параллельный планировщик в плане |
| 2 | рендер: тени, постобработка, прозрачность, частицы, 2D, UI | частично (GPU-скиннинг, IBL, кластеры, LOD, terrain впереди) |
| 3 | геймплей: физика, звук, Lua, ввод | частично (joints, navmesh, сеть, таймеры впереди) |
| 4 | сборка и платформы | `olvin-hub build` есть; Windows/macOS CI, веб, Android впереди |
| 5 | редактор | в основном сделано (настраиваемые клавиши, multi-scene, редактор материалов с превью впереди) |
| 6 | экосистема и документация | эта вики, примеры, CHANGELOG, публикация крейтов |

## Что в работе

> **Статус:** планируется или в разработке (страницы появятся вместе с кодом): библиотека пресетов и панель пресетов, частицы в формате сцены и редакторе ([#11](https://github.com/DanyaDel/Olvin/issues/11)), GPU-скиннинг ([#8](https://github.com/DanyaDel/Olvin/issues/8)), пост-обработка и шейдеры ([#15](https://github.com/DanyaDel/Olvin/issues/15), [#22](https://github.com/DanyaDel/Olvin/issues/22)).

## Открытые темы по разделам

- Рендер: [#13](https://github.com/DanyaDel/Olvin/issues/13) тени, [#15](https://github.com/DanyaDel/Olvin/issues/15) пост-обработка, [#16](https://github.com/DanyaDel/Olvin/issues/16) LOD, [#18](https://github.com/DanyaDel/Olvin/issues/18) кластеры, [#19](https://github.com/DanyaDel/Olvin/issues/19) IBL.
- Геймплей: [#2](https://github.com/DanyaDel/Olvin/issues/2) физика, [#34](https://github.com/DanyaDel/Olvin/issues/34) navmesh, [#33](https://github.com/DanyaDel/Olvin/issues/33) сеть.
- Платформы: [#50](https://github.com/DanyaDel/Olvin/issues/50), [#51](https://github.com/DanyaDel/Olvin/issues/51), [#52](https://github.com/DanyaDel/Olvin/issues/52).
- Инструменты: [#70](https://github.com/DanyaDel/Olvin/issues/70), [#45](https://github.com/DanyaDel/Olvin/issues/45), [#44](https://github.com/DanyaDel/Olvin/issues/44), [#41](https://github.com/DanyaDel/Olvin/issues/41).
- Релиз: [#65](https://github.com/DanyaDel/Olvin/issues/65) примеры, [#66](https://github.com/DanyaDel/Olvin/issues/66) CHANGELOG, [#67](https://github.com/DanyaDel/Olvin/issues/67) crates.io.

## См. также

- [Архитектура](architecture.md)
- [Руководство контрибьютора](contributing.md)
- [Что такое Olvin](../start/what-is-olvin.md)
- [Языки документации](language-policy.md)
