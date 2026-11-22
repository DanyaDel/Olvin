# Префабы (формат и override)

Префаб это `*.prefab.ron`: обычный `SceneFile` с одним корнем и его детьми (свои `id`, свои миграции). В сцене экземпляр это одна сущность-корень с `EntityDesc::prefab`.

Файл префаба из среза (`apps/vertical_slice/assets/prefabs/crate.prefab.ron`):

```ron
(
    version: 4,
    entities: [
        (id: 1, name: "Crate", position: (0.0, 0.0, 0.0), rotation: (0.0, 0.0, 0.0), scale: (1.0, 1.0, 1.0),
            mesh: Some((shape: Cube, color: (0.6, 0.42, 0.22, 1.0))),
            collider: Some((shape: Box(size: (1.0, 1.0, 1.0)), center: (0.0, 0.0, 0.0), is_trigger: false, friction: 0.5, restitution: 0.0)),
            rigidbody: Some((kind: Dynamic, mass: 2.0, drag: 0.1, angular_drag: 0.1, gravity_scale: 1.0, freeze_rotation: (false, false, false), ccd: false)),
            scripts: [(name: "Thud")]),
    ],
)
```

## PrefabInstance

`PrefabInstance { asset, overrides, removed }`:

| Поле | Смысл |
|---|---|
| `asset` | путь от `assets/`, как у материалов и моделей |
| `overrides` | `BTreeMap<путь узла, BTreeMap<поле, текст RON>>`: разница с префабом |
| `removed` | пути удалённых узлов |

Поле override: верхнеуровневое поле `EntityDesc` по имени (`position`, `mesh`, `scripts`, `material`…) или `components.<Тип>` (пустой текст значит, что компонента на экземпляре нет). Гранулярность: поле целиком. Путь узла это `id` префаба через `/`: `""` корень, `"3"` узел 3, `"3/2"` узел 2 вложенного префаба, который сам узел 3. Корень сам хозяин имени, активности, родителя и трансформа, это не override.

В срезе третий ящик отличается цветом: это `overrides` на узле `""`. Дети, добавленные к экземпляру, остаются обычными сущностями сцены с `parent` в узле экземпляра.

## Развёртывание

`SceneFile::expand_prefabs(loader)` разворачивает экземпляры в обычные сущности; `collapse_prefabs(loader)` делает обратное (разница становится override). `member_id` даёт стабильные id узлов (хеш с `MEMBER_BIT`). Префаб, который не прочитался или зациклен (охрана глубины `MAX_DEPTH` = 16), даёт предупреждение, объект остаётся пустым, паники нет.

Игра: `spawn_ids` разворачивает экземпляры сам; корню добавляется `PrefabInstanceOf(путь)`. Во время игры создать префаб можно так:

```rust
let e = olvin::scene_file::instantiate_prefab(res, "prefabs/crate.prefab.ron", Transform::from_xyz(0.0, 2.0, 0.0))?;
```

Дополнительно: `prefab_pristine`, `prefab_diff`, `revert_prefab[_field]`, `unpack_prefab`, `apply_prefab`.

## См. также

- [Префабы в редакторе](../editor/prefabs.md)
- [Формат сцены](scene-format.md)
- [Ассеты](assets.md)
- [Туториал: шаг 3](../tutorial/03-physics.md)
