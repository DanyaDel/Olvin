# Время и события

Основа `olvin_core`: приложение, расписание, время и типизированные события.

## App и расписание

`App` хранит `Resources`, `Schedule`, плагины и runner. `add_plugin` пропускает повторы того же типа. Стадии: `Startup, First, PreUpdate, Update, PostUpdate, Render, Last`; система это `FnMut(&mut Resources)`, пока всё однопоточное (планировщик на rayon в плане). Без runner'а крутится headless-цикл: `startup()` и `update()`.

## Resources

Синглтоны по `TypeId`. `get/get_mut` возвращают Option, `fetch/fetch_mut` паникуют с именем типа. Разные ресурсы можно брать на запись одновременно, один и тот же нельзя.

## Time

Обновляется в `First`.

| Метод | Смысл |
|---|---|
| `delta()`, `elapsed()` | время симуляции: умножается на `time_scale`, замирает на паузе |
| `unscaled_delta()`, `unscaled_elapsed()` | реальное время (UI) |
| `pause()`, `resume()`, `step()` | пауза и покадровый шаг |
| `set_time_scale(x)` | масштаб (отрицательное = 0) |
| `frame()` | счётчик кадров, считает и кадры паузы |
| `Time::with_fixed_delta(dt)`, `advance(dt)` | детерминированные тесты |

Реальный кадр ограничен 0,25 с. Физика идёт фиксированными шагами 1/60 с.

## Events

`Events<T>` очередь-ресурс: `send`, `send_batch`; читатель держит `EventCursor` и вызывает `events.read(&mut cursor)`, каждое событие отдаётся читателю один раз. Два буфера: событие живёт кадр отправки и весь следующий, поэтому читатель раньше отправителя в кадре тоже его видит. `App::add_event::<T>()` создаёт ресурс и меняет буферы в `First`, можно звать из нескольких плагинов.

```rust
app.add_event::<Hit>();
res.fetch_mut::<Events<Hit>>().send(Hit { damage: 5 });
// ...
let events = res.fetch::<Events<Hit>>();
for hit in events.read(&mut cursor) { /* ... */ }
```

## Таймеры

`TimerPlugin` (его сам добавляет `ScriptPlugin`) даёт ресурс `Timers`: задержки и повторы без ручного счёта кадров.

```rust
let mut timers = app.resources.fetch_mut::<Timers>();
let id = timers.after(2.0, "respawn");   // один раз через 2 с игрового времени
timers.every(0.5, "spawn_wave");         // каждые 0,5 с
timers.cancel(id);                       // или cancel_tag("spawn_wave")
```

Когда таймер истёк, в `PreUpdate` отправляется событие `TimerFired { id, tag }`; читать его можно через `EventCursor`, как любое событие. Таймеры идут по игровому времени: в паузе стоят и подчиняются `time_scale`; `after_unscaled` и `every_unscaled` не зависят ни от того, ни от другого (меню). Повторяющийся таймер не накапливает дрейф; после очень долгого кадра он срабатывает не больше 8 раз, остальной хвост отбрасывается. В скриптах то же самое: `ctx:after`, `ctx:every`, `ctx:cancel_timer` (см. [справочник Lua](../reference/lua-api.md)): объект получает `on_message(name)`.

## Лог

`Logging::new("info,wgpu_core=warn").file("logs/game.log").init()`; фильтр переопределяется `OLVIN_LOG` / `RUST_LOG`. Паника пишет `logs/crash-<время>.txt` (сообщение, место, поток, backtrace, 64 последние строки лога).

> **Статус:** таймеры, корутины и async-задачи не сделаны ([#37](https://github.com/DanyaDel/Olvin/issues/37)).

## См. также

- [Профилировщик](profiler.md)
- [Состояния и сохранения](game-states-saves.md)
- [Переменные окружения](../reference/env-vars.md)
- [Физика](physics.md)
