# Шаг 8. Сохранение и загрузка

`F5` сохраняет состояние игры в слот, `F9` загружает. Нужен модуль `game` (`olvin_game::save`). Сохранения отдельны от сцен: сцена это уровень, как его сделал редактор, сохранение это состояние сессии.

## Как работает

- `SavePlugin::new(dir)` кладёт слоты в `<dir>/<слот>.save.ron` (запись через временный файл и rename) и по умолчанию регистрирует состояние движка: `Transform`, скорости динамических тел, `CharacterMotor`, состояние скриптов Rust (`Behaviour::save_state` / `load_state`).
- Сущности сцены сохраняются под своим `SceneId`, поэтому ничего регистрировать для ящиков и игрока не нужно. Свои компоненты и ресурсы регистрируются: `register_component::<T>(name)`, `register_resource::<T>(name)` (serde + RON).
- `restore` ставит тела на сохранённые позы и возвращает скорости. Всё разбирается до того, как тронут мир, поэтому ошибка ничего не меняет.

## Код

Плагин и действия (в `main`, после `DefaultPlugins`):

```rust
use olvin::game::{SaveMeta, SavePlugin, SaveRegistry, SaveSlots};

let saves = app.resources.fetch::<ProjectPaths>().resolve("saves");
let mut keys = InputMap::default();
keys.bind("QuickSave", [Binding::Key(KeyCode::F5)]);
keys.bind("QuickLoad", [Binding::Key(KeyCode::F9)]);
app.resources.fetch_mut::<InputMap>().fill_missing(&keys);
app.add_plugin(SavePlugin::new(saves))
    .add_system(Stage::Update, quick_slot);
```

Система (упрощённый `apps/vertical_slice/src/save.rs`):

```rust
fn quick_slot(res: &mut Resources) {
    let (save, load) = {
        let a = res.fetch::<ActionState>();
        (a.just_pressed("QuickSave"), a.just_pressed("QuickLoad"))
    };
    if save {
        let meta = SaveMeta {
            title: "Quick save".into(),
            scene: "scenes/main.scene.ron".into(),
            playtime_secs: res.fetch::<Time>().elapsed(),
            saved_at: 0,
        };
        let data = res.fetch::<SaveRegistry>().capture(res, meta).expect("capture");
        res.fetch::<SaveSlots>().save("quick", &data).expect("write slot");
        log::info!("saved: {} entities", data.entities.len());
    }
    if load {
        match res.fetch::<SaveSlots>().load("quick") {
            Ok(data) => res.fetch::<SaveRegistry>().restore(res, &data).expect("restore"),
            Err(e) => log::warn!("no quick save: {e}"),
        }
    }
}
```

В срезе после `restore` ещё синхронизируются общие переменные Lua: сохранённый `Progress { door_opened }` возвращается в `ScriptVars` (`door::restore_vars`), потому что состояние Lua-скриптов в сохранение пока не попадает.

> **Статус:** состояние `self`-таблиц Lua не сохраняется (хуки `save_state`/`load_state` для Lua не подключены); храните нужное в компонентах, ресурсах или `ScriptVars` и регистрируйте их.

## Проверка

Подвигайте ящик, `F5`, подвигайте ещё, `F9`: ящик и игрок вернутся. Файл: `saves/quick.save.ron` в папке проекта (`/saves` в `.gitignore` шаблона).

## См. также

- [Состояния игры и сохранения](../engine/game-states-saves.md)
- [Ввод](../engine/input.md)
- [Скрипты](../engine/scripting.md)
- [Дальше: шаг 9, сборка](09-build.md)
