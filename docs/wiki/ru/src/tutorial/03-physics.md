# Шаг 3. Физика

Ящики и мяч, которые можно толкать. Нужен модуль `physics` (он в шаблоне 3D + Physics).

## Ящик

1. **GameObject → 3D Object → Cube**, имя `Crate`, Position `-4, 0.5, 5`.
2. Inspector → **Add Component → Physics → Rigidbody**: Kind Dynamic, Mass 2, Drag 0.1. Box Collider у куба уже есть.

Мяч: Sphere на `2, 0.5, 8.5` с таким же Rigidbody. Тела создаются в рантайме по компонентам `ColliderDesc` и `RigidbodyDesc`, масштаб объекта запекается в форму коллайдера, см. [Физика](../engine/physics.md).

## Префаб

Четыре ящика уровня это экземпляры одного префаба `prefabs/crate.prefab.ron`. Чтобы сделать так же:

1. Выделить `Crate` → **GameObject → Create Prefab** (или контекстное меню в Hierarchy). Появится `prefabs/Crate.prefab.ron`, объект становится экземпляром (значок ◆ синего цвета).
2. Перетащите префаб из Assets во вьюпорт (встанет на землю под курсором) или на строку Hierarchy (станет дочерним).
3. Правка узла экземпляра это override: в Inspector блок **Prefab** покажет список, `↺` откатит поле, **Apply All** запишет изменения обратно в ассет.

В сцене экземпляр занимает одну запись:

```ron
(id: 41, name: "Crate 2", position: (-5.3, 0.5, 5.2), rotation: (0.0, -5.0, 0.0), scale: (1.0, 1.0, 1.0),
    prefab: Some((asset: "prefabs/crate.prefab.ron"))),
```

Подробнее: [Префабы](../engine/prefabs.md), [Префабы в редакторе](../editor/prefabs.md).

## Проверка

В редакторе нажмите ▶ (`Ctrl+P`): ящики упадут и лягут на пол. Stop возвращает сцену как была.

## См. также

- [Физика](../engine/physics.md)
- [Префабы](../engine/prefabs.md)
- [Инспектор](../editor/inspector.md)
- [Дальше: шаг 4, персонаж](04-player.md)
