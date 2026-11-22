# Скрипты

Два способа писать логику объектов: поведения на Rust (`Behaviour`, модуль `script`) и на Lua 5.4 (`lua`). Оба подключаются к объекту через компонент `Scripts` (в сцене: `scripts: [(name, params)]`).

## Поведения на Rust

```rust
struct Spinner { speed: f32 }
impl FromParams for Spinner {
    fn from_params(p: &Params) -> Self { Self { speed: p.f32("speed", 90.0) } }
    fn schema() -> Vec<ParamSpec> { vec![ParamSpec::float("speed", 90.0, "degrees per second")] }
}
impl Behaviour for Spinner {
    fn update(&mut self, ctx: &mut ScriptCtx<'_>) {
        ctx.rotate_euler_deg(Vec3::new(0.0, self.speed * ctx.dt(), 0.0));
    }
}
// app.register_script::<Spinner>("Spinner");
```

Колбэки `Behaviour`: `start`, `update`, `fixed_update`, `on_collision_enter/exit`, `on_trigger_enter/exit`, `on_destroy`; хуки сохранения `save_state` / `load_state`. `ScriptCtx` даёт мир и компоненты, Transform-помощники, время, поиск (`find_by_name`, `find_by_tag`), ресурсы (`res`, `res_mut`), ввод, `raycast`, силы. Отложенные `spawn/despawn/insert/remove` выполняются в конце прохода. Паника в колбэке отключает только этот скрипт. Готовые: Rotator, Oscillator, Mover, FollowTarget, DestroyAfter, FreeFlyCamera, SimplePlayerController.

## Lua

Скрипт это `assets/scripts/<имя>.lua`, возвращающий таблицу с колбэками; имя файла это имя скрипта в сцене.

```lua
return {
    params = { { name = "speed", kind = "float", default = 90, doc = "degrees per second" } },
    start  = function(self, ctx) self.t = 0 end,
    update = function(self, ctx)
        ctx:rotate_euler_deg_xyz(0, self.params.speed * ctx:dt(), 0)
    end,
}
```

`LuaPlugin { dir, hot_reload, limits }` ставится после `DefaultPlugins`. Колбэки: `start`, `update`, `fixed_update`, `on_collision_enter/exit`, `on_trigger_enter/exit`, `on_destroy`, `on_message(self, ctx, name, value, from)`. `self` свой у каждого объекта, `self.params` заполнен из сцены по типам `params`. Полный список функций `ctx:` и колбэков: [справочник Lua API](../reference/lua-api.md).

**Горячая перезагрузка:** изменённый файл подхватывается, новые колбэки заменяют старые, `self` сохраняется, `start` заново не вызывается; файл с ошибкой оставляет работающую версию.

**Песочница:** загружены только `table`, `string`, `math`, `utf8`, `coroutine`; нет `io`, `os`, `debug`, `package`, `dofile`, `loadfile`, `require`. Лимит памяти 64 МБ на состояние, 5 000 000 инструкций на колбэк, ограничены сообщения (4096 в очереди), переменные (1024), строки (16 КБ), звуки (1024). Упавший колбэк отключает скрипт до перезагрузки файла, кадр не падает.

**Производительность:** векторы как таблицы `vec3` удобны, но для горячего кода есть `*_xyz`-двойники без таблиц (оценка «ходока» 0,42 мкс, бенчмарк `cargo bench -p olvin_script --features lua --bench lua_vs_rust`). Выбор Lua: `docs/adr/0001-lua-scripting.md`.

## Сообщения и переменные

`ctx:send(name, value[, target])` кладёт `ScriptMessage` в `Events<ScriptMessage>`; цель получает `on_message`, игра читает очередь сама (так срез показывает HUD). `ctx:var(name)` и `ctx:set_var(name, value)`: общие переменные `ScriptVars` между скриптами и игрой (`vars.flag(name)`, `vars.set(..)`).

> **Статус:** Lua в редакторе не запускается и его параметры не видны в инспекторе. Не сделано: нодовый редактор, `require` между файлами, таймеры и корутины-задержки ([#37](https://github.com/DanyaDel/Olvin/issues/37)), состояние `self` в сохранениях, веб. Горячая перезагрузка кода Rust: [#70](https://github.com/DanyaDel/Olvin/issues/70).

## См. также

- [Справочник Lua API](../reference/lua-api.md)
- [Туториал: шаг 5](../tutorial/05-door.md)
- [Физика](physics.md)
- [Состояния и сохранения](game-states-saves.md)
