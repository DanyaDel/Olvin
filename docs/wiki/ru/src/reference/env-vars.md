# Переменные окружения

| Переменная | Где читается | Значение |
|---|---|---|
| `WGPU_BACKEND` | wgpu | `gl` включает GL (тир Compat); не задан: Vulkan / Metal / DX12 (Full). На проприетарной NVIDIA под Wayland `gl` не работает |
| `__EGL_VENDOR_LIBRARY_FILENAMES` | glvnd | `/usr/share/glvnd/egl_vendor.d/50_mesa.json`: использовать Mesa (llvmpipe) для GL |
| `OLVIN_LOG`, `RUST_LOG` | `olvin_core::logging` | фильтр лога: `debug`, `info,olvin_render=trace` (самый длинный префикс модуля побеждает); `OLVIN_LOG` главнее |
| `OLVIN_PROJECT_DIR` | `ProjectPaths::detect` | корень проекта |
| `OLVIN_ASSETS_DIR` | `ProjectPaths::detect` | папка ассетов (меняет только `assets`) |
| `OLVIN_PROFILE` | `olvin_core::profile` | `1`: писать профиль (нужна фича `profile`) |
| `OLVIN_PROFILE_HTTP` | `profile-http` | `1` или `адрес:порт` (по умолчанию `127.0.0.1:8585`): сервер для `puffin_viewer` |
| `OLVIN_CUBES` | `sandbox` | число кубов (по умолчанию 40000) |
| `OLVIN_SANDBOX_AUTOQUIT`, `OLVIN_SLICE_AUTOQUIT` | `sandbox`, `vertical_slice` | выйти через N кадров (то же, что `--frames N`) |
| `XDG_CONFIG_HOME` | хаб | где искать `olvin-hub/config.toml` (Linux) |
| `NO_COLOR` | хаб CLI | отключить цвет логотипа |

## Для тестов и скриншотов

| Переменная | Тест |
|---|---|
| `OLVIN_EDITOR_SHOTS` | `editor_dev --test screenshots`: куда писать `editor-{dark,light,modeling}.png` (по умолчанию `docs/screenshots/`) |
| `OLVIN_EDITOR_REVIEW`, `OLVIN_EDITOR_REVIEW_ONLY` | тест `review`: все рабочие области в обеих темах в папку; `_ONLY` фильтр по подстроке имени файла |
| `OLVIN_EDITOR_PREFAB_SHOTS` | тест `prefab_ui` |
| `OLVIN_HUB_SHOTS`, `OLVIN_HUB_SHOTS_ROOT` | скриншоты хаба (`olvin_hub_gui -- --ignored screenshots`) |
| `OLVIN_SHOTS_DIR` | PNG тестов `olvin_render` |
| `OLVIN_ANIM_SHOTS`, `OLVIN_IDE_SHOTS`, `OLVIN_POST_SHOTS`, `OLVIN_UI_RENDER_DIR` | PNG тестов аниматора, IDE, постобработки, `olvin_ui` |
| `OLVIN_BENCH_FRAMES` | число кадров в бенчмарках |

## См. также

- [Раскладка проекта](../engine/project-layout.md)
- [Профилировщик](../engine/profiler.md)
- [Чёрное окно и GL](../troubleshooting/black-window.md)
- [Руководство контрибьютора](../dev/contributing.md)
