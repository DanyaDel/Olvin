# Профилировщик

Встроенный профилировщик на puffin 0.20 (чистый Rust). Включается фичей `profile` фасада; без неё метки раскрываются в ничто.

## Включение

```toml
olvin = { path = "...", features = ["profile"] }      # или "profile-http"
```

```sh
OLVIN_PROFILE=1 cargo run --release                    # запись включена
OLVIN_PROFILE_HTTP=1 cargo run --release               # + сервер для puffin_viewer (127.0.0.1:8585)
OLVIN_PROFILE_HTTP=0.0.0.0:9000 cargo run --release    # другой адрес и порт
```

Запись идёт только когда включена (`profile::set_enabled(true)`), иначе стоит одна атомарная проверка.

## Что размечено

`Schedule::add` регистрирует scope каждой системы под её путём (`olvin_core::time::update_time`) и scope стадии (`stage Update`). Свои scopes: рендер (`render frame`, `shadow pass`, `scene passes`, `submit and present`…), физика (`physics step`, `physics fixed step`), скрипты (`script update pass`, `script fixed_update pass`). Свой: `profile_scope!("имя")` (только литерал).

## Где смотреть

- Панель **Profiler** в редакторе: график кадра, счётчики, таблица систем ([Консоль и профилировщик](../editor/console-profiler.md)).
- `puffin_viewer` по сети с `profile-http` (`cargo install puffin_viewer`).
- Программно: `Collector::latest()` / `recent(n)` отдают `FrameProfile { scopes }`.

`RenderStats` (draw calls, instances, culled, triangles) доступны и без фичи.

> **Статус:** бенчмарки с эталонными сценами и цена кадра в CI не сделаны ([#68](https://github.com/DanyaDel/Olvin/issues/68)).

## См. также

- [Консоль и профилировщик (редактор)](../editor/console-profiler.md)
- [Медленные кадры](../troubleshooting/performance.md)
- [Время и события](time-events.md)
- [Фичи olvin](../reference/olvin-features.md)
