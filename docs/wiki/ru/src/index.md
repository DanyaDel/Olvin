# Olvin Wiki

Модульный игровой движок на Rust от [NLPA Collab](https://nlpacollab.neocities.org/). Эта вики описывает движок, хаб и редактор: по странице на тему, факты вместо рассказов, ссылки между страницами.

![Редактор Olvin](images/editor-dark.png)

## С чего начать

| Хочу | Страница |
|---|---|
| Понять, что это такое | [Что такое Olvin](start/what-is-olvin.md) |
| Поставить | [Установка](start/installation.md) |
| Сделать игру за час | [Туториал: первая игра](tutorial/index.md) |
| Разобраться в редакторе | [Редактор: обзор](editor/overview.md), [горячие клавиши](editor/hotkeys.md) |
| Написать скрипт | [Скрипты](engine/scripting.md), [справочник Lua API](reference/lua-api.md) |
| Что-то сломалось | [FAQ и решение проблем](troubleshooting/faq.md) |
| Внести вклад | [Руководство контрибьютора](dev/contributing.md) |

## Разделы

- **Начало**: [что такое Olvin](start/what-is-olvin.md), [установка](start/installation.md), [хаб](start/hub.md), [первый запуск редактора](start/editor-first-run.md).
- **Туториал**: [первая игра за час](tutorial/index.md) по вертикальному срезу `apps/vertical_slice`.
- **Редактор**: вкладки, иерархия, инспектор, вьюпорт, ассеты, материалы, префабы, анимация, IDE, CSG, моделирование, UV, настройки.
- **Движок**: раскладка проекта, формат сцены, ассеты, префабы, рендер, физика, звук, ввод, анимация, игровой UI, скрипты, состояния и сохранения, время и события, профилировщик.
- **Справочник**: [Lua API](reference/lua-api.md), [настройки рендера](reference/render-settings.md), [фичи `olvin`](reference/olvin-features.md), [CLI `olvin-hub`](reference/hub-cli.md), [переменные окружения](reference/env-vars.md), [глоссарий](reference/glossary.md).
- **Решение проблем**: [FAQ](troubleshooting/faq.md), чёрное окно, звук, горячая перезагрузка, медленные кадры, ошибки сборки, Windows и macOS.
- **Разработка**: [вклад](dev/contributing.md), [архитектура](dev/architecture.md), [план](dev/roadmap.md), [языки документации](dev/language-policy.md).

Поиск по вики: значок лупы в верхней панели или клавиша `S`. Язык переключается кнопкой в верхней панели справа. Состояние кода: коммит, на котором написана вики, см. [архитектура](dev/architecture.md).

> **Статус:** вики пишется под состояние `main` на 2026-11-22. Страницы про возможности, которые ещё в работе (пресеты и библиотека, частицы в сцене, GPU-скиннинг), содержат пометку «планируется».

## См. также

- [README репозитория](https://github.com/DanyaDel/Olvin/blob/main/README.md)
- [ARCHITECTURE.md](https://github.com/DanyaDel/Olvin/blob/main/ARCHITECTURE.md): источник правды о фактах
- [Issue #63: книга и туториал](https://github.com/DanyaDel/Olvin/issues/63)
