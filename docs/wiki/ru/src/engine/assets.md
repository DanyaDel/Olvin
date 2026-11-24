# Ассеты и GUID

Ассеты устроены как в Unity: рядом с файлом лежит `<имя>.meta` (RON) с GUID и настройками импортёра. Код: `olvin_assets` (без GPU).

## AssetRoot и .meta

- `AssetRoot` это папка `assets/`. `resolve` не выходит за её пределы (`..` только внутри, части с `:` отбрасываются).
- `scan()` создаёт недостающие `.meta`: `Meta { guid, importer }`, importer: `None`, `Texture(TextureImport { srgb, max_size, generate_mips, point_filter })`, `Model(ModelImport { scale, .. })`.
- Копия с чужим `.meta` получает новый GUID. `move_asset` и `delete_asset` переносят `.meta` вместе с файлом.

## Что умеет загружать

| Тип | Что |
|---|---|
| Картинки | PNG и др., HDR, EXR (в float); миниатюры (`ThumbnailCache`, LRU) |
| glTF (`import_gltf`) | меши, материалы, скины в `Skeleton`, анимации в `Clip`, камеры; встроенные картинки в `<модель>_textures/`; `KHR_texture_transform` запекается в UV |
| Материалы | `*.mat.ron` |
| Префабы | `*.prefab.ron` (`AssetKind::Prefab`) |

Не поддержаны в glTF: смещения тангентов у морф-таргетов, ортографические камеры как компонент, анимация узлов вне скелета на сущностях.

## AssetManager

Ресурс `AssetManager` (его гонит `AssetsPlugin` в `Stage::First`, `with_hot_reload()`):

- `load(path)` сразу отдаёт счётный `Handle<T>`, грузит в пуле потоков; тот же путь даёт тот же хэндл.
- `get` → `Arc<T>`, `state` (`Loading` / `Ready` / `Failed`), `is_fully_loaded`, `wait_idle`.
- Зависимости: материал держит свои текстуры, модель текстуры своих материалов.
- Выгрузка: `unload_unused` снимает записи без хэндлов (и в `update` при `auto_unload`).
- Горячая перезагрузка (`AssetWatcher`, notify): пока грузятся новые данные, доступны старые; при ошибке остаются последние рабочие.
- Паника декодера на битом файле ловится в воркере, запись становится `Failed`.

В игре сцена с рендером использует кэш `RenderAssets` в фасаде `olvin`: путь → хэндл для материалов, текстур, шейдеров и неба, с горячей перезагрузкой по mtime раз в 0,5 с.

> **Статус:** упаковка ассетов в архив и сжатие текстур не сделаны ([#26](https://github.com/DanyaDel/Olvin/issues/26)). `Meshes` не умеет освобождать слоты: повторный спавн сцен растит GPU-память.

## См. также

- [Панель Assets](../editor/assets.md)
- [Формат сцены](scene-format.md)
- [Раскладка проекта](project-layout.md)
- [Фичи olvin](../reference/olvin-features.md)
