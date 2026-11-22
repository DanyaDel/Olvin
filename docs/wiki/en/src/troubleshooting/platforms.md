# Windows and macOS

Arch Linux is the main development platform; Windows and macOS are supported by the code but checked less.

> **Status:** `.github/workflows/ci.yml` has a `desktop` job (tests, release builds of six binaries, zip artifacts) for Windows and macOS, but it has not run on real runners yet: [#50](https://github.com/DanyaDel/Olvin/issues/50). Everything below comes from the code and library docs, not from verified runs.

## Windows

- Rendering: DX12 or Vulkan (wgpu picks; Full). `WGPU_BACKEND=gl` gives Compat.
- Rust through rustup, the MSVC toolchain (Visual Studio Build Tools).
- The `.exe` icon for the hub and editors: `build.rs` + `winresource` (`assets/logo/olvin-icon.ico`), see [#55](https://github.com/DanyaDel/Olvin/issues/55).
- Hub paths: config in `%APPDATA%\olvin-hub`, the editor in the user's data directory.
- `olvin-hub build` creates a `.zip` for `--archive zip`.

## macOS

- Rendering: Metal (Full).
- Xcode Command Line Tools.
- The editor's `Ctrl+…` shortcuts are `Cmd+…` on macOS (egui `COMMAND`).
- Signing and notarization are not done by the hub.

## Web and Android

Not supported: [#51](https://github.com/DanyaDel/Olvin/issues/51), [#52](https://github.com/DanyaDel/Olvin/issues/52). The architecture is ready (WebGL2/GLES3 minimum, `SurfaceTarget`), there is no build.

## See also

- [Installation](../start/installation.md)
- [Build errors](build-errors.md)
- [Hub](../start/hub.md)
- [FAQ](faq.md)
