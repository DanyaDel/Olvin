# Windows и macOS

Основная платформа разработки Arch Linux; Windows и macOS поддерживаются кодом, но проверены хуже.

> **Статус:** в `.github/workflows/ci.yml` есть задача `desktop` (тесты, релизные сборки шести бинарников, zip-артефакты) для Windows и macOS, но на настоящих раннерах её ещё не запускали: [#50](https://github.com/DanyaDel/Olvin/issues/50). Всё ниже получено из кода и документации библиотек, а не из проверенных запусков.

## Windows

- Рендер: DX12 или Vulkan (wgpu выбирает сам; Full). `WGPU_BACKEND=gl` даст Compat.
- Rust через rustup, MSVC toolchain (Visual Studio Build Tools).
- Иконка `.exe` для хаба и редакторов: `build.rs` + `winresource` (`assets/logo/olvin-icon.ico`), см. [#55](https://github.com/DanyaDel/Olvin/issues/55).
- Пути хаба: конфиг в `%APPDATA%\olvin-hub`, редактор в каталоге данных пользователя.
- `olvin-hub build` создаёт `.zip` для `--archive zip`.

## macOS

- Рендер: Metal (Full).
- Xcode Command Line Tools.
- Сочетания `Ctrl+…` редактора на macOS это `Cmd+…` (egui `COMMAND`).
- Подпись и нотаризация не делаются хабом.

## Web и Android

Не поддерживаются: [#51](https://github.com/DanyaDel/Olvin/issues/51), [#52](https://github.com/DanyaDel/Olvin/issues/52). Архитектура готова (минимум WebGL2/GLES3, `SurfaceTarget`), сборки нет.

## См. также

- [Установка](../start/installation.md)
- [Ошибки сборки](build-errors.md)
- [Хаб](../start/hub.md)
- [FAQ](faq.md)
