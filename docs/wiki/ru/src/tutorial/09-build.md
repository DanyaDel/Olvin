# Шаг 9. Сборка игры

Отдаём игру человеку без редактора и Cargo.

```sh
olvin-hub build --release
```

Хаб запускает `cargo build --release`, находит exe и раскладывает папку `dist/first_game/`:

```
dist/first_game/
  first_game          исполняемый файл
  assets/             копия assets/ проекта (без скрытых файлов и *~)
  licenses/           лицензии (Inter, если включена фича gui, и проекта)
```

Запускайте из любой папки: `ProjectPaths` находит `assets/` рядом с exe, а логи (`logs/`) и сохранения (`saves/`) пишутся рядом. Архив: `olvin-hub build --release --archive tar.gz` (или `zip`) кладёт рядом `first_game.tar.gz`.

Папку `dist/` хаб дописывает в `.gitignore`, если проект старый и её там нет (CLI сообщает). Выходную папку хаб пересобирает только если сам её создал (маркер `.olvin-build`), чужую непустую не трогает.

> **Статус:** не сделано: кросс-компиляция (`--target`), сжатие и стрипинг ассетов, кнопка в GUI, подпись и инсталляторы. Упаковка ассетов: [#26](https://github.com/DanyaDel/Olvin/issues/26).

## Что дальше

- Прочитайте [Редактор: обзор](../editor/overview.md) и [Движок](../engine/project-layout.md).
- Подмените звуки и текстуры, добавьте [материалы](../editor/materials.md) и [анимацию](../editor/animation.md).
- Если что-то пошло не так: [FAQ](../troubleshooting/faq.md).

## См. также

- [CLI olvin-hub: build](../reference/hub-cli.md#build)
- [Раскладка проекта](../engine/project-layout.md)
- [Хаб](../start/hub.md)
- [Windows и macOS](../troubleshooting/platforms.md)
