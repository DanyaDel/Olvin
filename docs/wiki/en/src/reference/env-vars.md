# Environment variables

| Variable | Read by | Value |
|---|---|---|
| `WGPU_BACKEND` | wgpu | `gl` selects GL (Compat tier); unset: Vulkan / Metal / DX12 (Full). `gl` does not work on proprietary NVIDIA under Wayland |
| `__EGL_VENDOR_LIBRARY_FILENAMES` | glvnd | `/usr/share/glvnd/egl_vendor.d/50_mesa.json`: use Mesa (llvmpipe) for GL |
| `OLVIN_LOG`, `RUST_LOG` | `olvin_core::logging` | log filter: `debug`, `info,olvin_render=trace` (the longest module prefix wins); `OLVIN_LOG` takes precedence |
| `OLVIN_PROJECT_DIR` | `ProjectPaths::detect` | the project root |
| `OLVIN_ASSETS_DIR` | `ProjectPaths::detect` | the assets folder (changes only `assets`) |
| `OLVIN_PROFILE` | `olvin_core::profile` | `1`: record the profile (needs the `profile` feature) |
| `OLVIN_PROFILE_HTTP` | `profile-http` | `1` or `address:port` (default `127.0.0.1:8585`): a server for `puffin_viewer` |
| `OLVIN_CUBES` | `sandbox` | number of cubes (default 40000) |
| `OLVIN_SANDBOX_AUTOQUIT`, `OLVIN_SLICE_AUTOQUIT` | `sandbox`, `vertical_slice` | quit after N frames (same as `--frames N`) |
| `XDG_CONFIG_HOME` | hub | where to look for `olvin-hub/config.toml` (Linux) |
| `NO_COLOR` | hub CLI | turn off the logo colour |

## For tests and screenshots

| Variable | Test |
|---|---|
| `OLVIN_EDITOR_SHOTS` | `editor_dev --test screenshots`: where to write `editor-{dark,light,modeling}.png` (default `docs/screenshots/`) |
| `OLVIN_EDITOR_REVIEW`, `OLVIN_EDITOR_REVIEW_ONLY` | the `review` test: all workspaces in both themes into a folder; `_ONLY` filters by a substring of the file name |
| `OLVIN_EDITOR_PREFAB_SHOTS` | the `prefab_ui` test |
| `OLVIN_HUB_SHOTS`, `OLVIN_HUB_SHOTS_ROOT` | hub screenshots (`olvin_hub_gui -- --ignored screenshots`) |
| `OLVIN_SHOTS_DIR` | PNGs of `olvin_render` tests |
| `OLVIN_ANIM_SHOTS`, `OLVIN_IDE_SHOTS`, `OLVIN_POST_SHOTS`, `OLVIN_UI_RENDER_DIR` | PNGs of the animator, IDE, post-processing and `olvin_ui` tests |
| `OLVIN_BENCH_FRAMES` | the number of frames in benchmarks |

## See also

- [Project layout](../engine/project-layout.md)
- [Profiler](../engine/profiler.md)
- [Black window and GL](../troubleshooting/black-window.md)
- [Contributing guide](../dev/contributing.md)
