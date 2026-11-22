# Шаг 5. Плита и дверь на Lua

Нажимная плита открывает дверь. Логика на Lua: две короткие таблицы с колбэками. Нужен модуль `lua`.

## Объекты в сцене

| Имя | Что | Position | Scale | Компоненты |
|---|---|---|---|---|
| Door | куб | 0, 1.5, 0 | 2.4, 3, 0.3 | Box Collider, Rigidbody **Kinematic**, Scripts `door` |
| Plate | куб-метка | 4, 0.03, 6 | 2, 0.06, 2 | только Mesh Renderer |
| Plate Trigger | куб (меш можно убрать) | 4, 1, 6 | 2, 2, 2 | Box Collider с **Is Trigger**, Scripts `plate` |

Дверь кинематическая: её коллайдер следует за `Transform`, который двигает скрипт. Параметры скриптов задаются в Inspector, секция Scripts (ключ и значение строкой):

- `door`: `lift = 3.1`, `speed = 1.5`
- `plate`: `door = Door`, `message = Дверь открыта: зайди к роботу`

## door.lua

Файл `assets/scripts/door.lua` (имя файла это имя скрипта в сцене). Двигает дверь вверх на `lift`, пока общая переменная `door_open` истинна:

```lua
-- Sliding door: moves up by `lift` while the shared variable `door_open` is
-- true, back down when it is not. Simulation time, so it stops on pause.
-- The door is a kinematic body: its collider follows the Transform.
return {
    params = {
        { name = "lift", kind = "float", default = 3.1, doc = "how far up it slides, metres" },
        { name = "speed", kind = "float", default = 1.5, doc = "metres per second" },
    },

    start = function(self, ctx)
        local _, y = ctx:position_xyz()
        self.closed_y = y
    end,

    update = function(self, ctx)
        local x, y, z = ctx:position_xyz()
        local target = self.closed_y + (ctx:var("door_open") and self.params.lift or 0)
        local step = self.params.speed * ctx:dt()
        local dy = math.max(-step, math.min(step, target - y))
        if dy ~= 0 then
            ctx:set_position(x, y + dy, z)
        end
    end,
}
```

## plate.lua

Файл `assets/scripts/plate.lua`. Колбэк `on_trigger_enter` срабатывает, когда в триггер входит другой объект (`other`: его id):

```lua
-- Pressure plate (on its trigger volume): the player stepping in opens the
-- door named by `door` once, plays the door sound at the door and shows
-- `message` on the HUD.
--
-- The door's state is the shared variable `door_open` (ScriptVars): door.lua
-- slides the door by it, the game mirrors it into its saved `Progress`.
return {
    params = {
        { name = "door", kind = "string", default = "Door", doc = "name of the door entity" },
        { name = "message", kind = "string", default = "The door is open", doc = "shown when it opens" },
    },

    on_trigger_enter = function(self, ctx, other)
        if other ~= ctx:find_by_name("Player") or ctx:var("door_open") then
            return
        end
        local door = ctx:find_by_name(self.params.door)
        if not door then
            ctx:warn("no door named `" .. self.params.door .. "`")
            return
        end
        ctx:set_var("door_open", true)
        ctx:play_sound_at("sfx/door.wav", door, { volume = 0.9, max_distance = 30 })
        ctx:send("hud", self.params.message)
    end,
}
```

Что здесь происходит:

- `params` объявляет параметры и их типы (`float`, `string`); значения приходят как `self.params.*`.
- `ctx:position_xyz()` и `ctx:set_position(x, y, z)`: быстрый путь без таблиц.
- `ctx:var` / `ctx:set_var`: общие переменные между скриптами (`ScriptVars`). Дверь не знает про плиту, они связаны только именем `door_open`.
- `ctx:find_by_name`, `ctx:play_sound_at`, `ctx:send`: поиск объекта, звук ([шаг 6](06-sound.md)) и сообщение игре ([шаг 7](07-ui.md)).
- Время: `ctx:dt()` это время симуляции, на паузе дверь замирает.

Скрипты читаются из `assets/scripts/` (`LuaPlugin { dir, .. }` из шага 4). С `hot_reload` изменённый файл подхватывается на лету: `self` каждого объекта сохраняется, `start` заново не вызывается. Упавший колбэк отключает скрипт до перезагрузки файла, кадр не падает, ошибка идёт в лог.

## Проверка

`olvin-hub run`: наступите на жёлтую плиту, дверь поедет вверх. Сообщение пока уходит «в никуда»: HUD сделаем в шаге 7. Звука тоже нет, пока нет шага 6 (вызов `play_sound_at` с несуществующим файлом пишет ошибку в лог, а не падает).

## См. также

- [Скрипты](../engine/scripting.md)
- [Справочник Lua API](../reference/lua-api.md)
- [Физика: триггеры](../engine/physics.md#триггеры-и-события)
- [Дальше: шаг 6, звук](06-sound.md)
