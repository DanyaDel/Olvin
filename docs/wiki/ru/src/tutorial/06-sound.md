# Шаг 6. Звук

Нужен модуль `audio`: `DefaultPlugins` ставит `AudioPlugin`, который берёт звуки из `assets/`. Без звукового устройства все вызовы тихо ничего не делают.

## Эффекты из Lua

Вызов уже есть в `plate.lua` ([шаг 5](05-door.md)): позиционный звук в точке двери.

```lua
ctx:play_sound_at("sfx/door.wav", door, { volume = 0.9, max_distance = 30 })
```

Путь относительно `assets/`, `..` и абсолютные пути запрещены. Второй аргумент: вектор или id объекта. Опции: `volume`, `pitch`, `loop`, `bus`, `min_distance`, `max_distance`, `priority`, `stream`, `occludable`. Остановить: `ctx:stop_sound(handle, fade)`, спросить: `ctx:sound_playing(handle)`.

Громкость падает с расстоянием, панорама считается от компонента **Audio Listener** (он у камеры).

## Звук объекта из редактора

Маяк среза играет гул постоянно: **Add Component → Audio → Audio Source**, Clip `sfx/hum.wav`, включить Looping и Spatial, задать Max Distance. Play On Awake по умолчанию включён: источник начинает играть при появлении сцены и следует за объектом. Из Lua его можно запускать и останавливать: `ctx:play_source()` и `ctx:stop_source()`.

## Музыка

Музыка играет из Rust (ресурс `Audio`, шина `music`, стриминг). В `setup` проекта:

```rust
res.fetch_mut::<Audio>().play_music("music/theme.wav", 0.4, true); // путь, громкость, цикл
```

Шина `music` не встаёт на паузу вместе с игрой, остальные (`sfx`, `ambient`, `voice`) встают.

## Проверка

`olvin-hub run`: шаг на плиту проигрывает хлопок двери. Без слышимого результата смотрите [Нет звука](../troubleshooting/sound.md).

## См. также

- [Звук](../engine/audio.md)
- [Нет звука](../troubleshooting/sound.md)
- [Lua API: звук](../reference/lua-api.md#звук)
- [Дальше: шаг 7, текст на экране](07-ui.md)
