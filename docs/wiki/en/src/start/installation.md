# Installation

Olvin is installed from source: you need Rust and a few system libraries. There are no prebuilt binaries or AUR packages yet.

## Arch Linux

```sh
sudo pacman -S --needed base-devel git rustup alsa-lib libxkbcommon wayland systemd-libs vulkan-icd-loader
rustup default stable
```

- `rustup` installs `rustc` and `cargo`. Rust 1.95 or newer is required (it is the `rust-version` of eframe 0.36 and egui_dock 0.21).
- `alsa-lib` is for audio (kira → cpal), `systemd-libs` provides `libudev` for gamepads (gilrs), `libxkbcommon` and `wayland` are for the window (winit). With a desktop they are usually installed already.
- `vulkan-icd-loader` plus a driver: `vulkan-radeon`, `vulkan-intel` or `nvidia-utils`. Mesa llvmpipe (software rendering) comes with the `mesa` package.

## The hub from git

```sh
cargo install --git https://github.com/DanyaDel/Olvin olvin_hub_cli   # olvin-hub
cargo install --git https://github.com/DanyaDel/Olvin olvin_hub_gui   # olvin-hub-gui
```

From a clone: `cargo install --path hub/olvin_hub_cli` and `--path hub/olvin_hub_gui`. The binaries land in `~/.cargo/bin`: make sure it is in `PATH`.

The engine source for new projects (`EngineSource`) defaults to the folder the hub was built from; if it is gone, GitHub is used. Change it with `olvin-hub engine --path ~/Olvin` or `olvin-hub engine --git <url>`, see [Hub CLI](../reference/hub-cli.md).

## The editor

The editor is installed separately and its elements are fetched one at a time:

```sh
olvin-hub editor install          # base + console, csg, modeling, uv
olvin-hub editor add animator profiler
cd my_game && olvin-hub editor open
```

Building the editor takes a few minutes: it is compiled in `~/.local/share/olvin-hub/editor`. When the engine changes the hub rebuilds a stale editor itself. More: [Hub](hub.md), [First editor run](editor-first-run.md).

## Try it without a project

```sh
git clone https://github.com/DanyaDel/Olvin && cd Olvin
cargo run -p sandbox --release        # 40k cubes, WASD + RMB, Shift, Esc
cargo run -p vertical_slice --release # the finished sample game
```

## GPU

- **Vulkan** (default) works on AMD, Intel and NVIDIA. Do not set `WGPU_BACKEND`. Gives the `Full` tier.
- **GL** (`WGPU_BACKEND=gl`) gives the `Compat` tier (WebGL2/GLES 3). Works on Mesa.
- **Proprietary NVIDIA on Wayland: GL does not work.** The driver's EGL does not provide a native-renderable config and wgpu refuses to present a frame. This is a wgpu limitation. When `WGPU_BACKEND` is not set Olvin switches to Vulkan itself; when it is `gl` it logs a warning. Details: [Black window and GL](../troubleshooting/black-window.md).
- **llvmpipe** (Mesa's CPU rasterizer): for tests and windowless screenshots:

  ```sh
  __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json WGPU_BACKEND=gl cargo run -p sandbox --release
  ```

## Windows and macOS

The code is cross-platform (wgpu: DX12, Vulkan, Metal) and CI has a `desktop` job for Windows and macOS, but it has not run on real runners yet, see [#50](https://github.com/DanyaDel/Olvin/issues/50). Notes: [Windows and macOS](../troubleshooting/platforms.md).

## Checking the install

```sh
olvin-hub --help
olvin-hub list        # module catalog
```

## See also

- [Hub](hub.md)
- [First editor run](editor-first-run.md)
- [Build errors](../troubleshooting/build-errors.md)
- [Environment variables](../reference/env-vars.md)
