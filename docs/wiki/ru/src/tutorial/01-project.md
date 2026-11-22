# Шаг 1. Проект

Создаём проект в хабе, ставим нужные модули и открываем редактор.

## В хабе

1. Запустите `olvin-hub-gui`, на странице Projects нажмите `Ctrl+N`.
2. Шаблон **3D + Physics**, имя `first_game`, папка на ваш выбор.
3. Включите модули **Audio**, **Lua scripts**, **Game runtime** (карточки-переключатели; Lua сам потянет Scripts). Create.

То же в консоли:

```sh
olvin-hub new first_game && cd first_game
olvin-hub add physics audio lua game
```

## Фича gui

Игровой UI ([шаг 7](07-ui.md)) в каталоге хаба не представлен, добавьте фичу руками в `Cargo.toml`:

```toml
olvin = { path = "…", default-features = false, features = ["window", "render", "scene", "physics", "script", "input", "audio", "lua", "game", "gui"] }
```

(путь и список возьмите у себя: хаб уже записал туда `olvin`, вам нужно только дописать `"gui"`). Зависимости `serde` и `ron` для сохранений в шаге 8 не нужны: достаточно фасада.

## Редактор

```sh
olvin-hub editor install      # если ещё не ставили
olvin-hub editor add animator # не обязательно для туториала
olvin-hub editor open
```

Откроется рабочая область Layout: Scene, Game, Hierarchy, Inspector, Assets, см. [Первый запуск редактора](../start/editor-first-run.md).

Положите ассеты в папку `first_game/assets/`: `sfx/door.wav`, `sfx/thud.wav`, `music/theme.wav` (панель Assets увидит их и создаст `.meta` с GUID).

## Проверка

`olvin-hub run` собирает игру и показывает вращающийся куб: сцены ещё нет, шаблон рисует запасной куб. Выход по `Esc`.

## См. также

- [Хаб](../start/hub.md)
- [Фичи olvin](../reference/olvin-features.md)
- [Раскладка проекта](../engine/project-layout.md)
- [Дальше: шаг 2, сцена](02-scene.md)
