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

## Лог

`Logging::new("info,wgpu_core=warn").file("logs/game.log").init()`; фильтр переопределяется `OLVIN_LOG` / `RUST_LOG`. Паника пишет `logs/crash-<время>.txt` (сообщение, место, поток, backtrace, 64 последние строки лога).

> **Статус:** таймеры, корутины и async-задачи не сделаны ([#37](https://github.com/DanyaDel/Olvin/issues/37)).

## См. также

- [Профилировщик](profiler.md)
- [Состояния и сохранения](game-states-saves.md)
- [Переменные окружения](../reference/env-vars.md)
- [Физика](physics.md)
