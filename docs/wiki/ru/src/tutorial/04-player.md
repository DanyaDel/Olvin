# Шаг 4. Персонаж

Персонаж от первого лица на встроенном поведении `SimplePlayerController` (WASD, мышь, Space, Shift). Нужны модули `input` и `physics`.

## В редакторе

1. **GameObject → Create Empty**, имя `Player`, Position `0, 1, 10`.
2. **Add Component → Physics → Character Controller** (капсула: Height 2, Radius 0.5, шаг и уклон настраиваются там же).
3. **Add Component → Scripts → New Script**, имя `SimplePlayerController`, параметры (ключ–значение):

   | Ключ | Значение | Смысл |
   |---|---|---|
   | `speed` | 4 | ходьба, м/с |
   | `sprint_speed` | 7 | с зажатым Shift |
   | `jump_height` | 1.2 | высота прыжка, м |
   | `sensitivity` | 0.12 | градусов на пиксель мыши |
   | `camera` | `Player Camera` | имя дочерней камеры (наклон вверх-вниз) |

4. Выделите `Player`, **GameObject → Camera**, переименуйте в `Player Camera`, перетащите на `Player` в Hierarchy (станет дочерним), Position `0, 0.7, 0`. Камера создаётся вместе с Audio Listener: звук будет слышен из неё.

Действия `Move`, `Look`, `Jump`, `Sprint` входят в карту действий по умолчанию (`InputMap`), см. [Ввод](../engine/input.md).

## В коде

Захватим курсор при старте и положим `LuaPlugin` (понадобится в следующем шаге). `src/main.rs` проекта:

```rust
use olvin::prelude::*;
use olvin::script::lua::{LuaLimits, LuaPlugin};

fn main() {
    olvin::core::Logging::new("info,wgpu_core=warn,wgpu_hal=warn,naga=warn")
        .file("logs/game.log")
        .init();

    let mut app = App::new();
    app.add_plugin(DefaultPlugins {
        window: WindowPlugin { title: "first_game".into(), ..Default::default() },
        ..Default::default()
    });
    let assets = app.resources.fetch::<ProjectPaths>().asset("scripts");
    app.add_plugin(LuaPlugin { dir: assets, hot_reload: cfg!(debug_assertions), limits: LuaLimits::default() })
        .add_system(Stage::Startup, |res| res.fetch_mut::<ActionState>().lock_cursor(true))
        .add_system(Stage::Startup, setup);   // setup из шаблона: загружает сцену
    app.run();
}
```

`setup`, `Spin` и `spin` остаются из шаблона (загрузка `scenes/main.scene.ron`), `Esc` в шаблоне закрывает игру.

Шаблон загружает сцену через `olvin::scene_file::load`; он включает физику, ввод и скрипты, потому что `DefaultPlugins` ставит всё, что включено фичами.

## Проверка

`olvin-hub run --release`: ходите WASD, смотрите мышью, прыгайте на Space. Персонаж толкает динамические тела (`push_mass` в Character Controller, по умолчанию 80 кг).

## См. также

- [Физика: контроллер персонажа](../engine/physics.md#контроллер-персонажа)
- [Скрипты](../engine/scripting.md)
- [Ввод](../engine/input.md)
- [Дальше: шаг 5, плита и дверь](05-door.md)
