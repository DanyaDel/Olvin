# Руководство контрибьютора

Как собрать, проверить и отправить изменение в Olvin. Закрывает [#62](https://github.com/DanyaDel/Olvin/issues/62). Коротко: `fmt`, `clippy`, `test`, коммит по правилам, обновлённая документация.

## Репозиторий

```
crates/   движок (olvin_core … olvin — фасад)
editor/   olvin_editor и элементы
hub/      olvin_hub (ядро), olvin_hub_cli, olvin_hub_gui
apps/     sandbox, script_demo, editor_dev, vertical_slice
docs/     wiki, adr, screenshots
```

Перед работой прочитайте [ARCHITECTURE.md](https://github.com/DanyaDel/Olvin/blob/main/ARCHITECTURE.md): что лежит в каждом крейте. Он же источник правды при расхождении с кодом, и его дополняют после каждого значимого изменения (а русский и английский файлы вместе, [языки](language-policy.md)).

## Проверки перед коммитом

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Это же гоняет CI (`.github/workflows/ci.yml`). Rust 1.95+, edition 2024. Тяжёлые сборки держите в `CARGO_TARGET_DIR` на диске, а не в `/tmp` (на Arch это tmpfs).

## Тесты и GPU

Большая часть тестов обычные. Тесты рендера рисуют в offscreen-текстуру и читают пиксели: им нужен адаптер. В CI и локально без видеокарты их гоняют на Mesa llvmpipe:

```sh
export __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json WGPU_BACKEND=gl
cargo test -p olvin_render
cargo test -p olvin_gui --features render
```

С Vulkan (по умолчанию, без переменных) те же тесты идут на реальной видеокарте. Особенности:

- PCSS и другие тесты Full-тира требуют нативного адаптера (Vulkan / Metal / DX12): при `WGPU_BACKEND=gl` они пропускаются.
- Тесты, помеченные `#[ignore]`, рисуют скриншоты и интерфейс с egui_kittest и wgpu; запускайте с `-- --ignored`:

  ```sh
  cargo test -p editor_dev --test screenshots -- --ignored     # docs/screenshots/editor-*.png
  cargo test -p editor_dev --test ide -- --ignored             # IDE реальными кликами
  cargo test -p editor_dev --test animator -- --ignored
  cargo test -p olvin_hub_gui -- --ignored screenshots         # OLVIN_HUB_SHOTS=<папка>
  ```

- `cargo test -p vertical_slice`: 7 headless-тестов играют настоящий уровень без окна и GPU (через `HeadlessPlugins`).
- Фикстуры форматов лежат в `crates/olvin_scene/tests/fixtures/` и `crates/olvin_game/tests/fixtures/`.

В egui 0.36 headless-тесты обязаны чистить `textures_delta`.

## Стиль кода

- `cargo fmt` без исключений; clippy без предупреждений.
- Идентификаторы, комментарии, doc-комментарии, сообщения логов и ошибок: английский. Ошибки: что случилось и что делать.
- Горячие пути без аллокаций в установившемся режиме; плоские массивы, а не объекты.
- Пользовательские данные (пути, файлы) обрабатываются через `Result`, не `unwrap()`. `unwrap()` допустим в тестах.
- Не добавляйте `TODO`/`todo!`: заводите issue.
- Визуальный стиль редактора и хаба: [STYLE.md](https://github.com/DanyaDel/Olvin/blob/main/STYLE.md); интерфейс английский, без восклицательных знаков.
- Ломающее изменение формата сцены или сохранения: `FORMAT_VERSION += 1` (`SAVE_VERSION`), миграция, фикстура старой версии.

## Коммиты

Сообщение на английском: `Область: что сделано (#номер)`.

```
Editor: prefabs (#24)

Create Prefab from the selection, drag .prefab.ron into the scene ...

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_...
```

- Один коммит одна логическая правка; в теле: что и почему.
- Если код написан с помощью ИИ-агента, в конце сообщения остаются трейлеры `Co-Authored-By:` и `Claude-Session:` (их подставляет Claude Code; не удаляйте).
- Номер issue в заголовке (`(#24)`), если правка его закрывает или продвигает.
- Issues и обсуждения: по-русски.

## Добавить плагин движка

1. Крейт `crates/olvin_<имя>`, зависящий от `olvin_core`; `pub struct XPlugin; impl Plugin for XPlugin { fn build(&self, app: &mut App) { ... } }`. Плагин регистрирует ресурсы и системы (`add_system(Stage::Update, f)`), события (`add_event::<T>()`) и общается с остальными только через ресурсы. Зависимости добавляет сам: `app.add_plugin(OtherPlugin)` (повторы пропускаются).
2. В `Cargo.toml` workspace: `olvin_<имя>` в `[workspace.dependencies]`.
3. Фасад `crates/olvin`: опциональная зависимость, фича (`name = ["dep:olvin_<имя>"]`), реэкспорт модулем, подключение в `DefaultPlugins` (и `HeadlessPlugins`, если не требует окна), типы в `prelude`.
4. Хаб: запись `[[module]]` в `hub/olvin_hub/catalog.toml` (`kind = "olvin"`, `feature`, `requires`); тест сверяет каталог с фичами фасада.
5. Тесты в крейте (без GPU, если возможно), строка в ARCHITECTURE.md и ROADMAP.md, страница в этой вики.

## Добавить элемент редактора

1. Крейт `editor/olvin_editor_<имя>`, зависящий от `olvin_editor`.
2. `impl EditorPlugin { fn build(&self, r: &mut Registry) }`: `add_panel(PanelKind { id, title, factory })` (панель реализует `Panel::ui(ui, &mut EditorState)`), `add_workspace_template`, `add_viewport_extension` (гизмо, инструменты), `add_component` (редактор компонента), `add_create_item` (путь `"CSG/Box"` даёт подменю), `add_frame_hook`.
3. Запись `[[editor_module]]` в `catalog.toml` хаба; хаб сам впишет плагин в сгенерированный `main.rs` редактора.
4. UI-тест в `apps/editor_dev/tests/` (kittest) и строка в ARCHITECTURE.md.

## Документация

- Документация русская основная, английская копия рядом (`*.en.md`; в вики: `ru/` и `en/` с одинаковыми именами файлов).
- ARCHITECTURE.md и ROADMAP.md обновляются вместе с кодом: галочки ставятся в том же коммите.
- Вики: добавили или изменили возможность, обновите страницу; новый `ctx:`-метод Lua должен попасть в [справочник](../reference/lua-api.md), иначе CI красный. Правила страниц: `docs/wiki/README.md` в репозитории.
- Скриншоты рендерятся без окна (llvmpipe), см. `docs/wiki/README.md`.

## См. также

- [Архитектура](architecture.md)
- [План (ROADMAP)](roadmap.md)
- [Языки документации](language-policy.md)
- [Ошибки сборки](../troubleshooting/build-errors.md)
