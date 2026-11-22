# Звук

`olvin_audio` на kira 0.12: 2D и 3D звук, музыка потоком, шины, реверб, EQ, occlusion. Модуль `audio`.

## Использование

`AudioPlugin { assets_dir, enabled }` (по умолчанию `ProjectPaths::assets`), ресурс `Audio`:

```rust
let mut audio = res.fetch_mut::<Audio>();
audio.play("sfx/click.wav");                       // 2D, один раз
audio.play_at("sfx/thud.wav", Vec3::new(1.0, 0.0, 2.0)); // 3D
audio.play_music("music/theme.wav", 0.4, true);    // поток, одна музыкальная дорожка
```

`play_with(path, PlayParams { volume, pitch, looping, position, min_distance, max_distance, bus, priority, stream, occludable })`. `AudioSourceDesc` из сцены играет при появлении и следует за позицией. 3D: затухание и панорама от `AudioListener` (у камеры). Без устройства все вызовы no-op.

## Шины и эффекты

`BusId::{SFX, MUSIC, AMBIENT, VOICE}` и `add_bus(name)`: громкость, mute, 3-полосный EQ (shelf 250 Гц, bell 1 кГц, shelf 4 кГц) и уровень отправки в общий реверб.

## Occlusion, приоритеты

`set_occluder(|listener, source| 0..1)` (обычно луч физики) даёт low-pass (20 кГц → 500 Гц) и падение громкости до 0,35. Голосов до 64 (`set_max_voices`), потоков до 8; лишние виртуализируются по (priority, громкость, новизна): зацикленные ждут, one-shot отбрасываются. `priority` 0 важнее всего.

## Пауза

Система плагина следует за `Time::paused()`: звуки на шинах `pausable` (все, кроме `music`) встают на паузу и продолжают с того же места; звуки, запущенные во время паузы (меню), играют. Отключить: `set_follow_time_pause(false)`.

## Lua

`ctx:play_sound`, `ctx:play_sound_at`, `ctx:stop_sound`, `ctx:play_source`: [справочник](../reference/lua-api.md#звук). Пути только внутри `assets/`.

## См. также

- [Нет звука](../troubleshooting/sound.md)
- [Туториал: шаг 6](../tutorial/06-sound.md)
- [Скрипты](scripting.md)
- [Время и события](time-events.md)
