# Build errors

## Rust version

`error: package ... requires rustc 1.95` or let-chain errors: update (`rustup update stable`). Rust 1.95+ is required (eframe 0.36, egui_dock 0.21); edition 2024.

## Missing system libraries

| Message | Package (Arch) |
|---|---|
| `alsa` / `alsa-sys`, `pkg-config` | `alsa-lib`, `pkgconf` |
| `libudev` | `systemd-libs` |
| `xkbcommon`, `wayland-client` | `libxkbcommon`, `wayland` |
| `cc` / linker | `base-devel` |

```sh
sudo pacman -S --needed base-devel pkgconf alsa-lib systemd-libs libxkbcommon wayland
```

## Other

- **The editor builds for a long time.** The first build takes a few minutes (wgpu, egui, Rapier, kira). Updates are faster; the cache is in `~/.local/share/olvin-hub/editor/target`.
- **`/tmp` is full.** On Arch `/tmp` is a tmpfs in RAM: keep heavy builds in a `CARGO_TARGET_DIR` on disk.
- **Incompatible `olvin_*` and `olvin` versions.** Use all engine crates from one checkout (`path` or one `git rev`): do not mix them.
- **Lua: C build errors.** Lua 5.4 is built from source (`vendored`), a C compiler is needed (`base-devel`).
- **`cargo clippy -D warnings` fails in CI but passes locally.** CI uses the current stable; update Rust.
- **Hub: "name is reserved".** A project name cannot be `olvin`, `log`, `std` or a keyword.
- **The hub does not find the project.** It looks upward for a `Cargo.toml` with an `olvin` dependency; use `-p` or `olvin-hub register <path>`.
- **`olvin-hub build` did not find the exe.** Look at cargo's output above: the hub takes the path from `compiler-artifact`.

## See also

- [Installation](../start/installation.md)
- [FAQ](faq.md)
- [Contributing guide](../dev/contributing.md)
- [olvin-hub CLI](../reference/hub-cli.md)
