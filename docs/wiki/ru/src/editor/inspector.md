# Инспектор

Панель Inspector показывает компоненты выбранного объекта. Заголовок: флаг Active, имя, Tag, Layer. Ниже сворачиваемые секции; **Add Component** открывает меню с поиском и категориями.

## Компоненты

| Категория | Компонент | Данные в сцене |
|---|---|---|
| Rendering | Transform | позиция, поворот (градусы, Эйлер YXZ), масштаб |
| Rendering | Mesh Renderer | примитив и цвет, материал |
| Rendering | Model | glTF-модель, клип или граф анимации |
| Rendering | Camera | перспективная камера |
| Rendering | Light | `LightDesc`: Directional / Point / Spot, цвет, интенсивность, дальность, Cast Shadows |
| Physics | Box / Sphere / Capsule / Mesh Collider | `ColliderDesc` (центр, триггер, трение, упругость) |
| Physics | Rigidbody | Dynamic / Kinematic, масса, drag, гравитация, заморозка вращения, CCD |
| Physics | Character Controller | капсула, шаг, уклон, `push_mass` |
| Audio | Audio Source / Audio Listener | клип, громкость, зацикливание, 3D |
| Scripts | New Script | имя скрипта и параметры ключ–значение |
| Elements | компоненты элементов | CSG Brush и др. |

Примитивы создаются со своим коллайдером, камера с Audio Listener.

## Полезное

- **Light**: быстрые цветовые температуры (Candle… Sky), ручки дальности и конуса прямо во вьюпорте.
- **Material**: выбор и создание `*.mat.ron`, правка всех полей, сохраняется сразу ([Материалы](materials.md)).
- **Ключи анимации**: поля Transform окрашены: жёлтый = ключ на кадре, зелёный = анимировано, оранжевый = изменено вручную. ПКМ: Insert / Delete / Clear Keyframes; `I` / `Alt+I` при наведении ([Анимация](animation.md)).
- **Prefab**: блок с Open / Select / Revert All / Apply All / Unpack, список override с `↺` ([Префабы в редакторе](prefabs.md)).
- **Scripts**: параметры вводятся строками; Lua-скрипты схему параметров в инспекторе пока не показывают (редактор не грузит Lua), для Rust-поведений схема приходит из реестра.
- Широкие виджеты подстраиваются под панель: на узком инспекторе подписи X/Y/Z исчезают.

## См. также

- [Иерархия](hierarchy.md)
- [Материалы](materials.md)
- [Скрипты](../engine/scripting.md)
- [Формат сцены](../engine/scene-format.md)
