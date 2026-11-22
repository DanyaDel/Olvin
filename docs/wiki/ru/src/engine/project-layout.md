# Раскладка проекта

Где лежат файлы игры и как движок их находит: ресурс `ProjectPaths` из `olvin_core`.

## Папки проекта

```
my_game/
  Cargo.toml          зависимость olvin с модулями-фичами, deps всегда -O3
  src/main.rs         точка входа (шаблон хаба)
  assets/             всё, что нужно игре
    scenes/           *.scene.ron (главная: scenes/main.scene.ron)
    prefabs/          *.prefab.ron
    materials/        *.mat.ron, *.wgsl
    models/           glTF, рядом <модель>_textures/
    scripts/          *.lua
    sfx/, music/      звуки (путь в коде относительно assets/)
    input.ron         сохранённая карта действий (если есть)
    window.ron        режим окна и разрешение (если сохранено)
  saves/              слоты сохранений *.save.ron (в .gitignore)
  logs/               game.log, crash-<время>.txt (в .gitignore)
  dist/               результат olvin-hub build (в .gitignore)
```

Рядом с каждым ассетом редактор держит `<имя>.meta` с GUID ([Ассеты](assets.md)).

## ProjectPaths

`ProjectPaths { root, assets, source }`: ресурс, который вставляет `App::new`. Его читают `AudioPlugin`, `InputPlugin`, `Logging` и шаблон хаба. Методы: `asset(p)` (путь внутри `assets/`) и `resolve(p)` (путь от корня проекта); абсолютные пути не меняются.

`ProjectPaths::detect()` считается один раз на процесс, порядок поиска корня:

| № | Источник | Когда |
|---|---|---|
| 1 | `OLVIN_PROJECT_DIR` | явно заданная папка |
| 2 | папка exe, если в ней есть `assets/` | раскладка `olvin-hub build` |
| 3 | `CARGO_MANIFEST_DIR` | `cargo run`, `cargo test` |
| 4 | рабочая папка | запасной вариант |

`OLVIN_ASSETS_DIR` меняет только `assets`. Что сработало, говорит `PathsSource`. Поэтому собранная папка запускается двойным кликом или из любой рабочей папки.

`LuaPlugin` и `SavePlugin` путь берут как есть: передавайте им путь от `ProjectPaths` (`paths.asset("scripts")`, `paths.resolve("saves")`).

## Файлы редактора и хаба

| Файл | Что |
|---|---|
| `~/.config/olvin-hub/config.toml` | список проектов, источник движка, тема хаба |
| `~/.local/share/olvin-hub/editor` | собранный редактор |
| `<config>/olvin-editor/{preferences,layout}.ron` | настройки редактора |

## См. также

- [Хаб](../start/hub.md)
- [Ассеты](assets.md)
- [Формат сцены](scene-format.md)
- [Переменные окружения](../reference/env-vars.md)
