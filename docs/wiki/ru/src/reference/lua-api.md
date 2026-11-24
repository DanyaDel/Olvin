# Lua API

Справочник по скриптам на Lua 5.4 (`olvin_script`, фича `lua`). Сверяется с кодом скриптом `docs/wiki/tools/check_lua_reference.py`: каждая зарегистрированная функция `ctx:` и каждый колбэк должны быть здесь. Введение: [Скрипты](../engine/scripting.md).

## Скрипт

Файл `assets/scripts/<имя>.lua` возвращает таблицу. Имя файла это имя скрипта в сцене (`scripts: [(name: "door", params: {..})]`).

```lua
return {
    params = { { name = "speed", kind = "float", default = 90, doc = "degrees per second" } },
    start = function(self, ctx) end,
    update = function(self, ctx) end,
}
```

`params`: список `{ name, kind, default, doc }`; `kind`: `float` (`number`), `int` (`integer`), `bool` (`boolean`), `vec3`, `string` (по умолчанию), `entity` (имя другого объекта); тип как у `ParamKind`. Значения из сцены (строки) приводятся к типу и доступны как `self.params.<имя>`. `self` своя таблица у каждого объекта, сохраняется при горячей перезагрузке.

## Колбэки

| Колбэк | Аргументы | Когда |
|---|---|---|
| `start` | `(self, ctx)` | один раз перед первым `update` |
| `update` | `(self, ctx)` | каждый кадр |
| `fixed_update` | `(self, ctx)` | каждый фиксированный шаг физики (1/60 с) |
| `on_collision_enter` | `(self, ctx, other)` | начало контакта; `other`: id объекта |
| `on_collision_exit` | `(self, ctx, other)` | конец контакта |
| `on_trigger_enter` | `(self, ctx, other)` | вход в триггер |
| `on_trigger_exit` | `(self, ctx, other)` | выход из триггера |
| `on_destroy` | `(self, ctx)` | объект удаляется |
| `on_message` | `(self, ctx, name, value, from)` | пришло сообщение `ctx:send` |
| `on_net_message` | `(self, ctx, name, value, from)` | пришло сетевое сообщение (фича `net`) |

Колбэки читаются из таблицы при загрузке: присвоение `self.update = …` в рантайме их не заменяет. Объекты в Lua это числа-id (integer); `nil` значит «нет такого».

## Векторы

`vec3(x, y, z)`: таблица `{x, y, z}` с арифметикой (`a + b`, `a - b`, `-a`, `v * 2`, `a / 2`, покомпонентное `a * b`, `==`) и методами `v:dot(b)`, `v:length()`, `v:distance(b)`, `v:normalized()`, `v:cross(b)`, `v:lerp(b, t)`, `v:unpack()`. Везде, где функция принимает вектор, годятся `vec3(..)`, `{x=, y=, z=}`, `{x, y, z}` или три числа; недостающие оси 0.

**Быстрый путь `*_xyz`:** у каждого вектор-геттера есть двойник `имя_xyz()`, возвращающий три числа без таблицы; вектор-сеттеры принимают три числа (`set_position(x, y, z)`) или у `*_xyz` ровно три числа. Для горячего кода используйте их. `axis2d_xy` возвращает два числа.

## Время и объект

| Функция | Результат |
|---|---|
| `ctx:entity()` | id этого объекта |
| `ctx:name()` | имя объекта |
| `ctx:dt()` | шаг кадра, время симуляции (замирает на паузе) |
| `ctx:fixed_dt()` | шаг физики |
| `ctx:elapsed()` | время симуляции с начала |
| `ctx:frame()` | номер кадра |
| `ctx:unscaled_dt()` | шаг кадра в реальном времени |
| `ctx:time_scale()` | множитель времени |
| `ctx:paused()` | `true` на паузе |
| `ctx:log(msg)` | сообщение в лог (info) |
| `ctx:warn(msg)` | предупреждение в лог |

## Transform этого объекта

Углы в градусах, поворот Эйлер YXZ.

| Функция | Быстрый путь | Что |
|---|---|---|
| `ctx:position()` | `ctx:position_xyz()` | локальная позиция |
| `ctx:world_position()` | `ctx:world_position_xyz()` | мировая позиция |
| `ctx:set_position(v)` / `(x, y, z)` | `ctx:set_position_xyz(x, y, z)` | задать позицию |
| `ctx:translate(v)` | `ctx:translate_xyz(x, y, z)` | сдвиг в осях родителя |
| `ctx:translate_local(v)` | `ctx:translate_local_xyz(x, y, z)` | сдвиг в своих осях |
| `ctx:rotate_euler_deg(v)` | `ctx:rotate_euler_deg_xyz(x, y, z)` | довернуть |
| `ctx:rotation_euler_deg()` | `ctx:rotation_euler_deg_xyz()` | текущие углы |
| `ctx:set_rotation_euler_deg(v)` | `ctx:set_rotation_euler_deg_xyz(x, y, z)` | задать углы |
| `ctx:scale()` | `ctx:scale_xyz()` | масштаб |
| `ctx:set_scale(v)` | `ctx:set_scale_xyz(x, y, z)` | задать масштаб |
| `ctx:look_at(target[, up])` / `(x, y, z)` | `ctx:look_at_xyz(x, y, z[, ux, uy, uz])` | повернуться к точке (up по умолчанию +Y) |
| `ctx:forward()` | `ctx:forward_xyz()` | направление вперёд |
| `ctx:right()` | `ctx:right_xyz()` | вправо |
| `ctx:up()` | `ctx:up_xyz()` | вверх |

## Другие объекты

Id получают из поиска, из `other` колбэков и из `ctx:raycast`.

| Функция | Результат |
|---|---|
| `ctx:find_by_name(name)` | id первого объекта с таким именем или `nil` |
| `ctx:find_by_tag(tag)` | id первого объекта с тегом или `nil` |
| `ctx:find_all_by_tag(tag)` | массив id |
| `ctx:find_child(name)` | id прямого потомка этого объекта с таким именем или `nil` |
| `ctx:alive(id)` | `true`, если объект существует |
| `ctx:name_of(id)` | имя или `nil` |
| `ctx:has_tag(id, tag)` | `true`, если у объекта такой тег |
| `ctx:position_of(id)` / `ctx:position_of_xyz(id)` | локальная позиция или `nil` |
| `ctx:world_position_of(id)` / `ctx:world_position_of_xyz(id)` | мировая позиция или `nil` |
| `ctx:rotation_euler_deg_of(id)` / `ctx:rotation_euler_deg_of_xyz(id)` | углы или `nil` |
| `ctx:scale_of(id)` / `ctx:scale_of_xyz(id)` | масштаб или `nil` |
| `ctx:set_position_of(id, v)` / `ctx:set_position_of_xyz(id, x, y, z)` | задать позицию; `true`, если объект найден |
| `ctx:translate_of(id, v)` / `ctx:translate_of_xyz(id, x, y, z)` | сдвинуть чужой объект |
| `ctx:set_rotation_euler_deg_of(id, v)` / `ctx:set_rotation_euler_deg_of_xyz(id, x, y, z)` | задать углы |
| `ctx:set_scale_of(id, v)` / `ctx:set_scale_of_xyz(id, x, y, z)` | задать масштаб |
| `ctx:despawn(id)` | удалить объект с детьми (в конце прохода) |
| `ctx:spawn_prefab(path, x, y, z[, yaw_deg])` | поставить в очередь экземпляр префаба (`prefabs/...prefab.ron`, путь от `assets/`, без `..`); `false`, если за кадр уже 16 запросов или живёт 256 созданных объектов. Игра создаёт его после прохода и шлёт этому объекту сообщение `"spawned"`, значение — id нового корня |
| `ctx:destroy_self()` | удалить себя |

## Сообщения и переменные

| Функция | Что |
|---|---|
| `ctx:send(name, value[, target])` | отправить сообщение: `value` число, строка, bool, вектор или `nil`; без `target` это рассылка: `on_message` получает каждый объект со скриптом, а игра читает `Events<ScriptMessage>`; с `target` сообщение адресовано одному объекту; `from` это id отправителя |
| `ctx:var(name)` | общая переменная (`ScriptVars`) или `nil` |
| `ctx:set_var(name, value)` | записать общую переменную (`nil` удаляет) |

Имена и строки ограничены 16 КБ, очередь сообщений 4096, переменных 1024; превышение даёт ошибку скрипта.

## Сеть

Фича `net` у `olvin_script` и `LuaNetPlugin` после `NetPlugin` (см. [Сеть](../engine/networking.md)). Без сетевой сессии `is_server()` ложно, а `is_client()` истинно.

| Функция | Что |
|---|---|
| `ctx:is_server()` | этот процесс ведёт авторитетный мир (offline, хост или выделенный сервер) |
| `ctx:is_client()` | у процесса есть локальный игрок (offline, хост или удалённый клиент) |
| `ctx:local_client()` | наш id клиента (0 для offline и хоста) или `nil` до рукопожатия |
| `ctx:owner()` | id клиента, которому принадлежит объект (`Owner`), или `nil` |
| `ctx:is_mine()` | объект принадлежит локальному игроку |
| `ctx:net_send(target, name, value)` | отправить сообщение: `target` это `"server"`, `"all"`, `"owner"` (владелец этого объекта), id клиента или `{ except = id }`; клиент может слать только `"server"`; `value` число, строка, bool, вектор или `nil`; имя до 64 символов, текст до 256 байт, 64 отправки за кадр |
| `ctx:replicate_var(name)` | на сервере: общая переменная `name` (`ctx:set_var`) копируется всем клиентам при изменении и опоздавшим; клиенты читают её через `ctx:var` |

Колбэк `on_net_message(self, ctx, name, value, from)` вызывается для сообщений, пришедших по сети; `from` это id клиента-отправителя или `nil`, если писал сервер. На сервере проверяйте здесь, что просит клиент: id отправителя тот, который увидел сервер, а не написанный клиентом. Сообщения ограничены по частоте и размеру на каждого клиента, как и все сетевые сообщения.

## Физика

Фича `physics`; только для Dynamic-тела (иначе функции ничего не делают).

| Функция | Быстрый путь | Что |
|---|---|---|
| `ctx:apply_force(v)` | `ctx:apply_force_xyz(x, y, z)` | сила на следующий шаг (звать каждый шаг) |
| `ctx:apply_impulse(v)` | `ctx:apply_impulse_xyz(x, y, z)` | мгновенный импульс (Δv = импульс / масса) |
| `ctx:set_velocity(v)` | `ctx:set_velocity_xyz(x, y, z)` | задать скорость |
| `ctx:velocity()` | `ctx:velocity_xyz()` | скорость или `nil` |
| `ctx:motor_velocity()` | `ctx:motor_velocity_xyz()` | скорость мотора Character Controller этого объекта (персонаж движется по ней) или `nil` |
| `ctx:set_motor_velocity(v)` | `ctx:set_motor_velocity_xyz(x, y, z)` | задать её; `false` без Character Controller |
| `ctx:grounded()` | | персонаж стоит на земле |
| `ctx:gravity()` | | гравитация мира, м/с², положительная |
| `ctx:teleport(v)` / `ctx:teleport_of(id, v)` | `ctx:teleport_xyz(x, y, z)` / `ctx:teleport_of_xyz(id, x, y, z)` | поставить тело (любого вида, в том числе персонажа) в точку сразу и остановить; `false`, если объекта нет. Для респауна: обычный `set_position` тоже работает, но сохраняет скорость |
| `ctx:raycast(origin, direction, max)` | `ctx:raycast_xyz(ox, oy, oz, dx, dy, dz, max)` | луч: `nil` или `{ entity, point, normal, distance }`; быстрый путь возвращает `entity, px, py, pz, nx, ny, nz, distance` |

## Ввод

Фича `input`. `action` это имя действия из `InputMap` ([Ввод](../engine/input.md)).

| Функция | Результат |
|---|---|
| `ctx:pressed(action)` | зажато |
| `ctx:set_cursor_locked(locked)` | захватить (или отпустить) курсор мыши для обзора |
| `ctx:just_pressed(action)` | нажато в этом кадре |
| `ctx:just_released(action)` | отпущено в этом кадре |
| `ctx:value(action)` | число 0..1 (ось) |
| `ctx:axis2d(action)` | таблица `{ x, y }` |
| `ctx:axis2d_xy(action)` | два числа `x, y` |

## Звук

Фича `audio`. Пути относительно `assets/`, без `..` и абсолютных. Без звукового устройства функции ничего не делают, `play_*` возвращает `nil`. Одновременно отслеживается не более 1024 звуков.

| Функция | Что |
|---|---|
| `ctx:play_sound(path[, opts])` | 2D-звук, возвращает хэндл звука |
| `ctx:play_sound_at(path, where[, opts])` | звук в точке: `where` вектор или id объекта |
| `ctx:play_sound_at_xyz(path, x, y, z[, opts])` | то же числами |
| `ctx:stop_sound(sound[, fade])` | остановить (с затуханием в секундах) |
| `ctx:sound_playing(sound)` | играет ли |
| `ctx:set_sound_volume(sound, volume)` | громкость |
| `ctx:set_sound_pitch(sound, pitch)` | высота (минимум 0.01) |
| `ctx:play_source([id])` | запустить `AudioSource` из сцены (свой или объекта `id`) |
| `ctx:stop_source([id])` | остановить его |

`opts`: `{ volume, pitch, loop, bus, min_distance, max_distance, priority, stream, occludable }`; `bus` это имя шины (`"sfx"`, `"music"`, `"ambient"`, `"voice"`, своя).

## Песочница

Библиотеки: только `table`, `string`, `math`, `utf8`, `coroutine`. Нет `io`, `os`, `debug`, `package`, `dofile`, `loadfile`, `require`. `load` компилирует только текст (бинарные чанки отклоняются: исправленный вручную байт-код портит память виртуальной машины), `string.dump` удалён, `collectgarbage` понимает только `"collect"` и `"count"`. Лимиты по умолчанию (`LuaLimits`): память 64 МБ на состояние, 5 000 000 инструкций на колбэк. Ошибка колбэка отключает скрипт до перезагрузки файла и идёт в лог, кадр не падает. `ctx` действует только внутри колбэка: сохранять его нельзя.

## См. также

- [Скрипты](../engine/scripting.md)
- [Туториал: шаг 5](../tutorial/05-door.md)
- [Физика](../engine/physics.md)
- [Звук](../engine/audio.md)
