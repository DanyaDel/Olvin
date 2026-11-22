# Profiler

A built-in profiler on puffin 0.20 (pure Rust). Enabled by the facade's `profile` feature; without it the marks expand to nothing.

## Enabling

```toml
olvin = { path = "...", features = ["profile"] }      # or "profile-http"
```

```sh
OLVIN_PROFILE=1 cargo run --release                    # recording on
OLVIN_PROFILE_HTTP=1 cargo run --release               # + a server for puffin_viewer (127.0.0.1:8585)
OLVIN_PROFILE_HTTP=0.0.0.0:9000 cargo run --release    # another address and port
```

Recording happens only when enabled (`profile::set_enabled(true)`), otherwise there is one atomic check.

## What is instrumented

`Schedule::add` registers a scope for each system under its path (`olvin_core::time::update_time`) and a scope for the stage (`stage Update`). Own scopes: rendering (`render frame`, `shadow pass`, `scene passes`, `submit and present`…), physics (`physics step`, `physics fixed step`), scripts (`script update pass`, `script fixed_update pass`). Your own: `profile_scope!("name")` (a literal only).

## Where to look

- The **Profiler** panel in the editor: frame graph, counters, a table of systems ([Console and profiler](../editor/console-profiler.md)).
- `puffin_viewer` over the network with `profile-http` (`cargo install puffin_viewer`).
- Programmatically: `Collector::latest()` / `recent(n)` return `FrameProfile { scopes }`.

`RenderStats` (draw calls, instances, culled, triangles) is available without the feature too.

> **Status:** benchmarks with reference scenes and frame cost in CI are not done ([#68](https://github.com/DanyaDel/Olvin/issues/68)).

## See also

- [Console and profiler (editor)](../editor/console-profiler.md)
- [Slow frames](../troubleshooting/performance.md)
- [Time and events](time-events.md)
- [olvin features](../reference/olvin-features.md)
