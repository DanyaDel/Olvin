# Анимация

Две независимые части: объектная анимация сцены (F-кривые, как в Blender) и скелетная анимация моделей glTF (клипы и граф состояний). Код: `olvin_scene::action`, `olvin_anim` (модуль `anim`, не требует рендера).

## Объектная анимация

`Action { fps (24), frame_start (1), frame_end (250), autoplay, looping, curves }` в `SceneFile::animation`. Кривые `FCurve { object, prop: Location | Rotation | Scale, index, extrapolation, keys }`, ключ `Key`: кадр, значение, ручки, `Interp::{Constant, Linear, Bezier}`, `HandleType::{Free, Auto, Vector, Aligned, AutoClamped}`. Расчёт ручек и вычисление перенесены из Blender 4.x. Поворот: Эйлер в градусах, по каналам.

В игре `ActionPlayer` (ресурс, его заводит `AnimPlugin`) проигрывает действие во времени игры: находит сущности по `SceneId`, пишет `Transform` (каналы без кривой остаются авторскими). `scene_file::spawn` заполняет его из сцены. Редактор: [Анимация](../editor/animation.md).

## Скелет и клипы

- `Skeleton` (`Joint { parent, target, rest, inverse_bind }`), `Pose`, `skin_matrices`.
- `Clip { channels, events }`: translation / rotation / scale, интерполяции Step, Linear (slerp), CubicSpline как в glTF; события клипа.
- `AnimGraph`: состояния (`Motion::Clip` или `Blend1D`), переходы с условиями по `Params` (float, bool, trigger), `exit_time`, кроссфейд.
- `Animator { skeleton, clips, params, layers, .. }`: слои смешиваются по `weight` и маске суставов; `AnimPlugin` двигает все `Animator` в `Update` по `Time::delta` и шлёт `Events<AnimationEvent>`.

Модель в сцене: `EntityDesc::model = ModelDesc { path, animation: Clip(имя) | Graph(путь .animgraph.ron) | Inline(..) }`. `ModelPlugin` строит под объектом меши, скелет, `SkinnedMesh` и `Animator`. `olvin::models::pending_count(res)` показывает, сколько моделей ещё грузится. Параметры графа задаются из кода: `Animator.params`.

## Скиннинг

`SkinnedMesh` скиннится в `PostUpdate`, когда выросла `version` у `Animator` (4 влияния, нормали и тангенты поворачиваются): compute-проходом на GPU в тире Full и на CPU в Compat. Позу получают тени и прозрачные; один слот меша на экземпляр.

> **Статус:** во вьюпорте редактора модели стоят в позе по умолчанию, в Play анимируются. Секвенсор и катсцены в плане.

## Морф-таргеты (блендшейпы)

Морф-таргеты glTF приходят вместе с моделью: у примитива с ними появляется компонент `MorphMesh` (базовый меш, цели по именам, их веса), а анимации весов модели становятся `MorphClips`. Когда веса меняются, смешанный меш пересобирается и загружается в собственный слот меша части; скиннинговое лицо смешивается в bind-позе и затем позируется, на обоих тирах. Имена целей берутся из `extras.targetNames` меша (Blender пишет туда имена shape key), иначе `target 0`, `target 1`, ... Смещения нормалей используются, если они есть в файле. Не поддержаны: смещения тангентов и меши без нормалей (их сгенерированные плоские нормали не учитывают цели).

```lua
-- на корневом объекте модели
ctx:set_morph("smile", 0.7)          -- 0..1, false, если такой формы нет
local names = ctx:morph_names()      -- {"smile", "blink_L", ...}
ctx:morph_play("talk", true)         -- анимация весов glTF, по кругу
ctx:morph_stop()
```

Из Rust: `MorphMesh::set("smile", 0.7)`, `MorphPlayer::new(clip, looping)`. Вьюпорт редактора показывает модели без морфинга (базовый меш); Play и игры морфят.

## См. также

- [Анимация (редактор)](../editor/animation.md)
- [Формат сцены](scene-format.md)
- [Рендер](rendering.md)
- [Время и события](time-events.md)
