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

`SkinnedMesh` скиннится на CPU (4 влияния, нормали и тангенты поворачиваются) в `PostUpdate`, когда выросла `version` у `Animator`. Работает на всех тирах и даёт позу теням и прозрачным; один слот меша на экземпляр.

> **Статус:** GPU-скиннинг не сделан, идёт отдельно: [#8](https://github.com/DanyaDel/Olvin/issues/8). Морф-таргеты: [#10](https://github.com/DanyaDel/Olvin/issues/10). Во вьюпорте редактора и в Play модели в позе по умолчанию.

## См. также

- [Анимация (редактор)](../editor/animation.md)
- [Формат сцены](scene-format.md)
- [Рендер](rendering.md)
- [Время и события](time-events.md)
