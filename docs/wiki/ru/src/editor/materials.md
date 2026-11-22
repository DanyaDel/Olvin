# Материалы

Материал это файл `*.mat.ron` (`MaterialDesc`): PBR в конвенции Unity (metallic-roughness). Редактор правит его в Inspector, секция **Material**, и пишет на диск сразу: вьюпорт читает файлы.

## Создание и назначение

- Инспектор → Material → New: файл в `materials/`.
- Assets → картинка → Create material: материал с этой текстурой.
- Перетащить материал из Assets на объект во вьюпорте или в иерархии.
- Импорт glTF кладёт материалы в `materials/<модель>/`.

Материал назначается мешу, `poly`-объекту и граням CSG-браша (`EntityDesc.material`: путь от `assets/`).

## Поля

| Поле | Смысл |
|---|---|
| `base_color`, текстура | цвет и альбедо |
| `normal` + strength | карта нормалей |
| `metallic`, `roughness`, `orm` | параметры или карта occlusion/roughness/metallic |
| `emissive` (+ текстура) | свечение |
| `uv_scale`, `uv_offset`, `triplanar` | развёртка |
| `point_filter` | пиксельная фильтрация |
| `blend`, `alpha_cutoff` | прозрачность или маска |
| `unlit` | без освещения |
| `shader` | свой поверхностный шейдер `*.wgsl` |

В инспекторе есть пресеты. Правки материалов входят в историю undo: каждый шаг несёт содержимое правленных файлов.

## Шейдеры

Поверхностный шейдер реализует контракт `fn surface(in: SurfaceInput) -> SurfaceOutput` (`SURFACE_SHADER_CONTRACT`), номера строк ошибок совпадают с файлом. Правка шейдера в IDE подхватывается на лету.

> **Статус:** редактор материалов с превью не сделан, есть только секция в инспекторе, см. [#41](https://github.com/DanyaDel/Olvin/issues/41). Шейдерного графа нет, см. [#22](https://github.com/DanyaDel/Olvin/issues/22).

## См. также

- [Рендер](../engine/rendering.md)
- [Ассеты](assets.md)
- [IDE](ide.md)
- [Настройки рендера (справочник)](../reference/render-settings.md)
