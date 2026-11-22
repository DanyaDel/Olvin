# Фичи фасада olvin

Крейт `olvin` (`crates/olvin`) реэкспортирует подсистемы; каждая включается фичей. По умолчанию включены `window` и `render`. Хаб при создании проекта ставит `default-features = false` и перечисляет фичи сам ([Хаб](../start/hub.md)).

| Фича | Что подключает | Включает | Модуль хаба |
|---|---|---|---|
| `window` | окно, `Input`, runner (winit) | | `window` |
| `render` | рендер wgpu; `GuiRenderPlugin`, если есть `gui` | | `render` |
| `scene` | загрузка `*.scene.ron`, модели, CSG, `RenderAssets` | `render`, `anim`, `assets` | `scenes` |
| `anim` | скелеты, клипы, `Animator`, `AnimPlugin` (без рендера) | | нет |
| `assets` | `.meta`, GUID, glTF, `AssetManager` | | нет |
| `physics` | Rapier: тела, коллайдеры, контроллер | | `physics` |
| `script` | поведения на Rust (`Behaviour`) | | `scripts` |
| `lua` | Lua 5.4 (vendored, без системной библиотеки) | `script` | `lua` |
| `input` | карты действий, геймпады | | `input` |
| `audio` | звук на kira | | `audio` |
| `game` | состояния, `SceneLoader`, сохранения | | `game` |
| `gui` | игровой UI и текст (`olvin::gui`, `olvin::text`) | `input` | нет |
| `profile` | puffin-метки систем, рендера, физики, скриптов | | нет |
| `profile-http` | сервер для `puffin_viewer` (`OLVIN_PROFILE_HTTP`) | | нет |

Фичи связываются: `physics` включает физику в `olvin_script` и `olvin_game`, `audio` и `input` включают одноимённые биндинги Lua.

## Наборы плагинов

- `DefaultPlugins { window, audio, .. }`: всё включённое (окно, рендер, физика, ввод, скрипты, звук, GUI, анимация).
- `HeadlessPlugins { physics, input, script, audio }`: то же без окна и GPU (звук по умолчанию выключен), для тестов игр и серверов. Вместе с `DefaultPlugins` не ставится.
- `prelude`: `App`, `Stage`, `Resources`, `Time`, `ProjectPaths`, `Transform`, `Camera`, `Input`, `Meshes`, … и типы включённых модулей.

## Проверка

Тест хаба сверяет модули каталога с фичами фасада; фичи `gui`, `anim`, `assets`, `profile*` пока не в каталоге.

## См. также

- [Хаб](../start/hub.md)
- [CLI olvin-hub](hub-cli.md)
- [Скрипты](../engine/scripting.md)
- [Профилировщик](../engine/profiler.md)
