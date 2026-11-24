# Физика

`olvin_physics` на Rapier 0.36: тела, коллайдеры, триггеры, контроллер персонажа, запросы. Модуль `physics` (фича фасада).

## Тела и коллайдеры

`PhysicsPlugin { gravity }` и ресурс `Physics`. Каждый кадр (PreUpdate) сущности с `ColliderDesc` или `RigidbodyDesc` без маркера `Body` получают тела: Dynamic, Kinematic или Fixed (если Rigidbody нет). Масштаб объекта запекается в форму коллайдера. `Mesh` берёт `MeshGeometry`. Порядок кадра: кинематические тела следуют за `Transform`, затем фиксированные шаги по 1/60 с (не больше 5 за кадр), затем динамические пишут позу обратно.

**Владение позой:** Fixed и Dynamic телепортируются, когда код изменил `Transform` (скорость сохраняется); статика, которая двигается каждый кадр, лучше сделать Kinematic. `Physics::teleport(world, entity)` ставит тело сразу и обнуляет скорости.

**Слои:** у коллайдера есть `layer` (0..31) и `mask`, слои, с которыми он сталкивается (бит `n` это слой `n`; по умолчанию слой 0 и все слои). Два коллайдера сталкиваются, только если в маске **каждого** есть слой другого, так что призрак на слое 3 проходит сквозь то, что слой 3 не включает. В инспекторе: *Layer* и *Collides With* у коллайдера. Запросы выбирают слои сами, см. «Запросы».

## Силы и скорости

`Physics::apply_force` (одноразовая, на следующий шаг), `apply_impulse` (Δv = импульс / масса), `set_velocity`, `velocity`, `angular_velocity`. Работают только с Dynamic-телом, иначе false. Из Lua: `ctx:apply_force`, `ctx:apply_impulse`, `ctx:set_velocity`, `ctx:velocity`.

## Контроллер персонажа

Капсула + `CharacterMotor { velocity, grounded, sliding, gravity_scale }`, ступени, уклон, `move_character`. Персонаж толкает динамические тела (`push_mass`, по умолчанию 80 кг, 0 выключает) и шлёт `ControllerHit { character, other, point, normal, direction }` в `Events<ControllerHit>`.

## Триггеры и события

Коллайдер с `is_trigger: true` не сталкивается, а даёт события. `CollisionEvents` по сущностям: в `Behaviour` и Lua это `on_collision_enter/exit` и `on_trigger_enter/exit` ([Скрипты](scripting.md)). События коллизий обрабатываются в проходе `Update`.

## Запросы

`raycast(_filtered)`, `spherecast(_filtered)`, `overlap_sphere` и общие с `QueryOptions { mask, exclude, hit_triggers }`: `raycast_with`, `shapecast(shape, position, rotation, direction, max, &options)` (сфера, коробка или капсула: `CastShape`) и `overlap(shape, position, rotation, &options)`. `mask` выбирает слои по **слою коллайдера** (бит `n` это слой `n`), а не по его собственной маске, как `LayerMask` в Unity.

В Lua: `ctx:raycast(origin, dir, max[, mask])`, `ctx:spherecast(origin, radius, dir, max[, mask])`, `ctx:boxcast(origin, half_extents, dir, max[, mask])` (возвращают `{ entity, point, normal, distance }` или `nil`) и `ctx:overlap_sphere(center, radius[, mask])` (список id). Собственный коллайдер того, кто бросает, пропускается.

> **Статус:** не сделано: joints ([#2](https://github.com/DanyaDel/Olvin/issues/2)). Ввод NaN-шага игнорируется, меш-коллайдер с индексами за пределами вершин пропускается с предупреждением.

## См. также

- [Скрипты](scripting.md)
- [Формат сцены](scene-format.md)
- [Туториал: шаг 3](../tutorial/03-physics.md)
- [Время и события](time-events.md)
