# Benchmarks

`apps/bench` (`olvin-bench`) measures what the engine's hot paths cost on reference scenes, so a slowdown shows up as a number instead of a feeling.

```
cargo run --release -p olvin_bench -- [--list] [--only text] [--json out.json] [--compare benches/baseline.json] [--tolerance 3] [--scale 0.5]
```

Each case builds its scene outside the timed part, runs a few warm-up passes and prints the median and the best time in milliseconds. `--only` picks cases by a piece of the name, `--scale` shortens the runs (never below 3), `--json` writes the numbers, `--compare` reports every case slower than the baseline by more than `--tolerance` times and exits with 1.

| Case | What it measures |
|---|---|
| `ecs_iterate_100k` | moving 100 000 entities through a query |
| `transform_propagation_20k` | world matrices of 20 000 objects in a hierarchy |
| `physics_pile_500_60_steps` | 60 fixed steps of 500 boxes dropped on a floor |
| `physics_raycast_10k` | 10 000 rays against 500 boxes |
| `nav_bake_160k_cells` | baking a 100 × 100 m level with walls |
| `nav_find_path_12_maze` | 12 A* paths through it (a maze: the search covers most of the level) |
| `lod_simplify_100k_triangles` | simplifying a 100 000 triangle mesh to a quarter |
| `scene_parse_5000_objects` | parsing a scene file of 5 000 objects |
| `save_sign_check_1mb` | signing and verifying 1 MB of save text |
| `timers_advance_20k` | 20 000 repeating timers over 100 frames |
| `lua_update_5000` | one update of 5 000 Lua behaviours |
| `render_prepare_5k_200_lights` | the CPU side of a frame: 5 000 objects, 200 lights (needs a GPU) |

## In CI

The `bench` job runs the suite on a GitHub runner, compares with `benches/baseline.json` and keeps the numbers as the artifact `bench-result`. A runner is not the machine the baseline was taken on, so the tolerance is 6 times: the job catches slowdowns of an order of magnitude and is allowed to fail without failing the build. For anything finer, run the suite on one machine before and after a change. Refresh the baseline with `--json benches/baseline.json` after an intended change, and say on which machine in the commit.

Rendering the frame itself (drawing, not preparing it) has a separate measurement: `cargo test -p olvin_render perf -- --ignored --nocapture`. Lua against Rust: `cargo bench -p olvin_script --features lua --bench lua_vs_rust`.

## See also

- [Contributing](contributing.md)
- [Profiler](../engine/profiler.md)
