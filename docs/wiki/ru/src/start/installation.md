# Установка

Olvin ставится из исходников: нужен Rust и системные библиотеки. Готовых бинарников и пакетов в AUR пока нет.

## Arch Linux

```sh
sudo pacman -S --needed base-devel git rustup alsa-lib libxkbcommon wayland systemd-libs vulkan-icd-loader
rustup default stable
```

- `rustup` ставит `rustc` и `cargo`. Нужен Rust 1.95 или новее (это `rust-version` у eframe 0.36 и egui_dock 0.21).
- `alsa-lib` нужна звуку (kira → cpal), `systemd-libs` даёт `libudev` для геймпадов (gilrs), `libxkbcommon` и `wayland` нужны окну (winit). С рабочим столом они обычно уже стоят.
- `vulkan-icd-loader` и драйвер: `vulkan-radeon`, `vulkan-intel` или `nvidia-utils`. Для Mesa llvmpipe (программный рендер) нужен пакет `mesa`.

## Хаб из git

```sh
cargo install --git https://github.com/DanyaDel/Olvin olvin_hub_cli   # olvin-hub
cargo install --git https://github.com/DanyaDel/Olvin olvin_hub_gui   # olvin-hub-gui
```

Из клона репозитория: `cargo install --path hub/olvin_hub_cli` и `--path hub/olvin_hub_gui`. Бинарники попадут в `~/.cargo/bin`: проверьте, что он в `PATH`.

Источник движка для новых проектов (`EngineSource`) по умолчанию папка, из которой собран хаб; если её нет, берётся GitHub. Сменить: `olvin-hub engine --path ~/Olvin` или `olvin-hub engine --git <url>`, см. [CLI хаба](../reference/hub-cli.md).

## Редактор

Редактор ставится отдельно, его элементы докачиваются по одному:

```sh
olvin-hub editor install          # база + console, csg, modeling, uv
olvin-hub editor add animator profiler
cd my_game && olvin-hub editor open
```

Сборка редактора занимает несколько минут: он компилируется в `~/.local/share/olvin-hub/editor`. Когда движок меняется, хаб пересоберёт устаревший редактор сам. Подробнее: [Хаб](hub.md), [Первый запуск редактора](editor-first-run.md).

## Попробовать без проекта

```sh
git clone https://github.com/DanyaDel/Olvin && cd Olvin
cargo run -p sandbox --release        # 40k кубов, WASD + ПКМ, Shift, Esc
cargo run -p vertical_slice --release # готовая игра-срез
```

## Видеокарта

- **Vulkan** (по умолчанию) работает на AMD, Intel и NVIDIA. `WGPU_BACKEND` задавать не нужно. Даёт тир `Full`.
- **GL** (`WGPU_BACKEND=gl`) даёт тир `Compat` (WebGL2/GLES 3). На Mesa работает.
- **Проприетарная NVIDIA под Wayland: GL не работает.** EGL драйвера не отдаёт native-renderable конфигурацию, и wgpu отказывается показывать кадр. Это ограничение wgpu. Если `WGPU_BACKEND` не задан, Olvin сам переключается на Vulkan; если задан `gl`, пишет предупреждение. Подробности: [Чёрное окно и GL](../troubleshooting/black-window.md).
- **llvmpipe** (CPU-растеризатор Mesa): для тестов и скриншотов без окна:

  ```sh
  __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json WGPU_BACKEND=gl cargo run -p sandbox --release
  ```

## Windows и macOS

Код кроссплатформенный (wgpu: DX12, Vulkan, Metal), в CI есть задача `desktop` для Windows и macOS, но на настоящих раннерах её ещё не запускали, см. [#50](https://github.com/DanyaDel/Olvin/issues/50). Заметки: [Windows и macOS](../troubleshooting/platforms.md).

## Проверка установки

```sh
olvin-hub --help
olvin-hub list        # каталог модулей
```

## См. также

- [Хаб](hub.md)
- [Первый запуск редактора](editor-first-run.md)
- [Ошибки сборки](../troubleshooting/build-errors.md)
- [Переменные окружения](../reference/env-vars.md)
