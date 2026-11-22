# Ошибки сборки

## Версия Rust

`error: package ... requires rustc 1.95` или ошибки let-chains: обновитесь (`rustup update stable`). Нужен Rust 1.95+ (eframe 0.36, egui_dock 0.21); edition 2024.

## Не хватает системных библиотек

| Сообщение | Пакет (Arch) |
|---|---|
| `alsa` / `alsa-sys`, `pkg-config` | `alsa-lib`, `pkgconf` |
| `libudev` | `systemd-libs` |
| `xkbcommon`, `wayland-client` | `libxkbcommon`, `wayland` |
| `cc` / линкер | `base-devel` |

```sh
sudo pacman -S --needed base-devel pkgconf alsa-lib systemd-libs libxkbcommon wayland
```

## Прочее

- **Долго собирается редактор.** Первая сборка несколько минут (wgpu, egui, Rapier, kira). Обновления быстрее; кэш в `~/.local/share/olvin-hub/editor/target`.
- **`/tmp` переполнен.** На Arch `/tmp` это tmpfs в RAM: тяжёлые сборки держите в `CARGO_TARGET_DIR` на диске.
- **Несовместимые версии `olvin_*` и `olvin`.** Все крейты движка из одного checkout'а (`path` или один `git rev`): не смешивайте.
- **Lua: ошибки сборки C.** Lua 5.4 собирается из исходников (`vendored`), нужен C-компилятор (`base-devel`).
- **`cargo clippy -D warnings` в CI падает локально проходящее.** CI ставит текущий stable; обновите rust.
- **Хаб: «name is reserved».** Имя проекта не может быть `olvin`, `log`, `std` и ключевым словом.
- **Хаб не находит проект.** Он ищет `Cargo.toml` с зависимостью `olvin` вверх по папкам; используйте `-p` или `olvin-hub register <путь>`.
- **`olvin-hub build` не нашёл exe.** Смотрите вывод cargo выше: хаб берёт путь из `compiler-artifact`.

## См. также

- [Установка](../start/installation.md)
- [FAQ](faq.md)
- [Руководство контрибьютора](../dev/contributing.md)
- [CLI olvin-hub](../reference/hub-cli.md)
