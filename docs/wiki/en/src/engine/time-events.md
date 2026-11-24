# Time and events

The base of `olvin_core`: the application, the schedule, time and typed events.

## App and the schedule

`App` holds `Resources`, a `Schedule`, plugins and a runner. `add_plugin` skips repeats of the same type. Stages: `Startup, First, PreUpdate, Update, PostUpdate, Render, Last`; a system is `FnMut(&mut Resources)`, everything is single-threaded for now (a rayon scheduler is planned). Without a runner a headless loop runs: `startup()` and `update()`.

## Resources

Singletons by `TypeId`. `get/get_mut` return an Option, `fetch/fetch_mut` panic with the type name. Different resources can be borrowed mutably at once, the same one cannot.

## Time

Updated in `First`.

| Method | Meaning |
|---|---|
| `delta()`, `elapsed()` | simulation time: multiplied by `time_scale`, frozen on pause |
| `unscaled_delta()`, `unscaled_elapsed()` | real time (UI) |
| `pause()`, `resume()`, `step()` | pause and frame stepping |
| `set_time_scale(x)` | the scale (negative = 0) |
| `frame()` | the frame counter, it counts pause frames too |
| `Time::with_fixed_delta(dt)`, `advance(dt)` | deterministic tests |

A real frame is capped at 0.25 s. Physics runs in fixed steps of 1/60 s.

## Events

`Events<T>` is a queue resource: `send`, `send_batch`; a reader holds an `EventCursor` and calls `events.read(&mut cursor)`, each event is delivered to a reader once. Two buffers: an event lives the frame it was sent and the whole next one, so a reader earlier in the frame than the sender sees it too. `App::add_event::<T>()` creates the resource and swaps buffers in `First`, it can be called from several plugins.

```rust
app.add_event::<Hit>();
res.fetch_mut::<Events<Hit>>().send(Hit { damage: 5 });
// ...
let events = res.fetch::<Events<Hit>>();
for hit in events.read(&mut cursor) { /* ... */ }
```

## Timers

`TimerPlugin` (added by `ScriptPlugin` on its own) gives the resource `Timers`: delays and repeats without counting frames.

```rust
let mut timers = app.resources.fetch_mut::<Timers>();
let id = timers.after(2.0, "respawn");   // once, in 2 s of game time
timers.every(0.5, "spawn_wave");         // every 0.5 s
timers.cancel(id);                       // or cancel_tag("spawn_wave")
```

When one runs out, the event `TimerFired { id, tag }` is sent in `PreUpdate`; read it with an `EventCursor` like any event. Timers follow simulation time, so they stand still in a pause and obey `time_scale`; `after_unscaled` and `every_unscaled` ignore both (menus). A repeating timer keeps its rhythm without drift; after a very long frame it fires at most 8 times and drops the rest of the backlog. Scripts have the same in `ctx:after`, `ctx:every` and `ctx:cancel_timer` (see the [Lua reference](../reference/lua-api.md)): the object gets `on_message(name)`.

## Log

`Logging::new("info,wgpu_core=warn").file("logs/game.log").init()`; the filter is overridden by `OLVIN_LOG` / `RUST_LOG`. A panic writes `logs/crash-<time>.txt` (message, location, thread, backtrace, the last 64 log lines).

> **Status:** timers, coroutines and async tasks are not done ([#37](https://github.com/DanyaDel/Olvin/issues/37)).

## See also

- [Profiler](profiler.md)
- [States and saves](game-states-saves.md)
- [Environment variables](../reference/env-vars.md)
- [Physics](physics.md)
