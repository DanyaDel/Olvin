# Состояния игры и сохранения

`olvin_game` (модуль `game`): состояния с входом и выходом, фоновая загрузка сцен, слоты сохранений. Без GPU и окна.

## Состояния

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum GameState { Loading, Playing, Paused }

app.add_state(GameState::Loading)
    .on_enter(GameState::Paused, |res| res.fetch_mut::<Time>().pause())
    .on_enter(GameState::Playing, |res| res.fetch_mut::<Time>().resume())
    .add_system_in_state(Stage::Update, GameState::Playing, player_control);
```

`State<S>` (`get`, `is`, `set`). `set` только запрашивает переход: он применяется раз в кадр в начале `PreUpdate` (побеждает последний запрос): `on_exit(old)`, переключение, `on_enter(new)`, событие `StateChanged<S>`. Цепочки переходов обрываются на 8 шагах; `on_enter(initial)` выполняется в `Startup`.

## Фоновая загрузка сцен

`SceneLoaderPlugin` + `SceneLoader`: `request(path) -> LoadHandle` читает и разбирает сцену (со всеми миграциями) в рабочем потоке; `status` (`Pending`, `Ready`, `Failed`, `Gone`), `take(handle)`. Экран загрузки это состояние `Loading`, которое ждёт `Ready` и `olvin::models::pending_count(res) == 0`, затем ставит `Playing`; спавн: `olvin::scene_file::spawn_loaded(res, handle)`. Образец: `apps/vertical_slice/src/level.rs`.

## Сохранения

Сохранение это состояние сессии, а не сцена. `SaveRegistry`: `register_component::<T>(name)`, `register_resource::<T>(name)` (serde + RON), `register_external` для состояния вне ECS. Сохраняются сущности с `SaveId(u64)` и объекты сцены (по `SceneId`).

`SavePlugin::new(dir)` регистрирует состояние движка: `transform`, `motor`, `velocity` (физика), `scripts` (`Behaviour::save_state` / `load_state`). `without_engine()` отключает.

- `capture(res, meta) -> SaveData { version, meta, entities, resources }`, `restore(res, &data)`: сущности с `SaveId` обновляются или создаются, отсутствующие в сохранении удаляются, зарегистрированные компоненты, которых нет в сохранении, снимаются. Всё разбирается до того, как тронут мир. Ссылки между сущностями не переназначаются: храните `SaveId`.
- `SaveSlots`: `<dir>/<слот>.save.ron`, `save` (tmp + rename), `load`, `list` (новые первыми), `delete`, `exists`; имя слота не выходит из каталога.
- Версионирование: `SAVE_VERSION` + `SAVE_MIGRATIONS`, будущая версия это ошибка.

> **Статус:** состояние `self` Lua-скриптов не сохраняется (хуки подключены только у Rust `Behaviour`); используйте `ScriptVars` и компоненты.

## См. также

- [Туториал: шаг 8](../tutorial/08-saves.md)
- [Формат сцены](scene-format.md)
- [Время и события](time-events.md)
- [Скрипты](scripting.md)
