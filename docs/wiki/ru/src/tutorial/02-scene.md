# Шаг 2. Сцена

Собираем уровень: пол, четыре стены, перегородка с проёмом, солнце. В срезе так устроен `assets/scenes/level.scene.ron`.

## Объекты

Создавайте через меню **GameObject → 3D Object → Cube** (примитив получает свой Box Collider) и правьте Transform в Inspector:

| Имя | Position | Scale |
|---|---|---|
| Floor | 0, -0.5, 0 | 24, 1, 24 |
| Wall North | 0, 2, -12.25 | 24, 4, 0.5 |
| Wall South | 0, 2, 12.25 | 24, 4, 0.5 |
| Wall East | 12.25, 2, 0 | 0.5, 4, 25 |
| Wall West | -12.25, 2, 0 | 0.5, 4, 25 |
| Divider West | -6.625, 2, 0 | 10.75, 4, 0.5 |
| Divider East | 6.625, 2, 0 | 10.75, 4, 0.5 |
| Lintel | 0, 3.5, 0 | 2.5, 1, 0.5 |

Между Divider West и East остаётся проём 2,5 м: туда встанет дверь (шаг 5). Цвет задаётся в секции Mesh Renderer.

Двойной клик по имени в Hierarchy переименовывает, `Ctrl+D` дублирует (значения Transform копируются).

## Свет

**GameObject → Light → Directional Light**, Rotation `-50, 30, 0`, Cast Shadows включён. Первый направленный свет становится солнцем и тенями. Окружение (небо, туман, экспозиция): **Edit → Settings… → Rendering**, там же пресеты Day / Sunset / Night / Studio, см. [Рендер](../engine/rendering.md).

## Сохранить

`Ctrl+S`, путь `scenes/level.scene.ron` (или `scenes/main.scene.ron`: именно её грузит шаблон). Результат в RON выглядит так (фрагмент среза):

```ron
(id: 10, name: "Floor", position: (0.0, -0.5, 0.0), rotation: (0.0, 0.0, 0.0), scale: (24.0, 1.0, 24.0),
    mesh: Some((shape: Cube, color: (0.32, 0.34, 0.36, 1.0))),
    collider: Some((shape: Box(size: (1.0, 1.0, 1.0)), center: (0.0, 0.0, 0.0), is_trigger: false, friction: 0.8, restitution: 0.0))),
```

Формат подробно: [Формат сцены](../engine/scene-format.md).

## Проверка

`olvin-hub run`: игра загрузит `assets/scenes/main.scene.ron` из шаблона. Камеры в сцене ещё нет, её мы создадим вместе с персонажем в [шаге 4](04-player.md).

## См. также

- [Сцена и вьюпорт](../editor/scene-view.md)
- [Инспектор](../editor/inspector.md)
- [Иерархия](../editor/hierarchy.md)
- [Дальше: шаг 3, физика](03-physics.md)
