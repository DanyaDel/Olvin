# CLI olvin-hub

`olvin-hub [команда]`. Без аргументов печатает справку с текстовым логотипом. Опция `-p, --project <путь>` указывает проект; по умолчанию проект ищется вверх по папкам от текущей. Сообщения на английском.

| Команда | Что делает |
|---|---|
| `new <имя> [--dir <папка>]` | создать проект игры (по умолчанию в текущей папке) |
| `list [-p]` | каталог модулей, `[x]` отмечает установленные |
| `add <модуль>…` | установить модули вместе с тем, что они требуют |
| `remove <модуль>… [--force]` | убрать модули; без `--force` нельзя убрать то, от чего зависят другие |
| `run [--release]` | собрать и запустить игру |
| `build [--release] [-o <папка>] [-a tar.gz\|zip]` | собрать игру и разложить папку для игроков |
| `build --release [--keep-symbols] [--no-strip] [--no-remap] [--sign-key FILE\|--no-sign] [--locked] [--trust]` | strip, символы рядом с папкой, ремап путей, подписанный `manifest.json`; см. [Безопасность сборок](../dev/build-security.md) |
| `keygen [-o FILE] [--force]` / `verify FOLDER [--pubkey KEY]` | создать ключ подписи релизов / проверить собранную папку по манифесту |
| `trust [PATH]` / `untrust [PATH]` | разрешить / перестать разрешать хабу собирать и запускать проект; иначе `run` и `build` требуют `--trust` или `OLVIN_TRUST=1` |
| `projects` | проекты, известные хабу |
| `register <путь>` / `forget <путь>` | добавить в список / убрать из списка (файлы остаются) |
| `engine [--path <папка> \| --git <url>]` | показать или сменить источник движка для новых проектов |
| `editor install` | собрать редактор |
| `editor list` | элементы, `[x]` установленные |
| `editor add <элемент>…` / `editor remove <элемент>…` | поставить / убрать элементы и пересобрать |
| `editor open [проект]` | открыть проект в редакторе (по умолчанию из текущей папки); устаревший редактор пересоберётся |

## build

```sh
olvin-hub build --release                       # dist/<имя>/
olvin-hub build --release -o ~/out/mygame       # своя папка
olvin-hub build --release -a tar.gz             # + <папка>.tar.gz
```

Запускает `cargo build --message-format=json-render-diagnostics` (вывод cargo идёт в терминал, путь к exe берётся из `compiler-artifact`), кладёт exe и копию `assets/` (без скрытых файлов и `*~`) и `licenses/` в `<проект>/dist/<имя>`. Папку пересобирает, только если её создал хаб (маркер `.olvin-build`). Если собирали в папку по умолчанию, `/dist` дописывается в существующий `.gitignore`. Для проектов-крейтов внутри workspace работает так же (`--project apps/vertical_slice`).

Не сделано: `--target` (кросс-компиляция), стрипинг и сжатие ассетов, инсталляторы.

## Модули и элементы

Каталог модулей: [Хаб](../start/hub.md#модули). Элементы редактора: `console`, `profiler`, `animator`, `csg`, `modeling`, `ide`, `uv`; по умолчанию ставятся `console`, `csg`, `modeling`, `uv`.

## Файлы и окружение

`~/.config/olvin-hub/config.toml` (`XDG_CONFIG_HOME`), `~/.local/share/olvin-hub/editor`. `NO_COLOR` убирает цвет логотипа.

## См. также

- [Хаб](../start/hub.md)
- [Установка](../start/installation.md)
- [Фичи olvin](olvin-features.md)
- [Переменные окружения](env-vars.md)
