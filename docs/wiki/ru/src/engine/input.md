# Ввод и переназначение

Два слоя: `Input` в `olvin_window` (клавиши, мышь, сырой `mouse_delta`, `scroll`, курсор) и карты действий в `olvin_input` (модуль `input`): игра читает не клавиши, а действия.

## Карты действий

`InputMap { actions: имя → [Binding] }`. Привязки: клавиши, кнопки и колесо мыши, кнопки геймпада, `Axis1D`, `Axis2D`, `Chord`. Карта по умолчанию: Move, Look, Jump, Sprint, Fire, Interact, Pause, Ascend. Сохранённая карта лежит в `assets/input.ron` (`ProjectPaths::asset`).

```rust
let a = res.fetch::<ActionState>();
if a.just_pressed("Jump") { /* ... */ }
let mv = a.axis2d("Move");          // Vec2
```

`ActionState`: `pressed`, `just_pressed`, `just_released`, `value`, `axis2d`, `lock_cursor(bool)`. Обновляется в `Stage::First`. Геймпады: gilrs (фича `gamepad`). Свои действия: `InputMap::bind(name, [Binding::Key(KeyCode::F5)])`, а `fill_missing(&map)` добавляет их в уже сохранённую игроком карту, не затирая переназначений.

## Переназначение на лету

Ресурс `Rebind`: `listen(action, Slot::Replace(i) | Slot::Add)`. Плагин ловит первую нажатую клавишу, кнопку мыши или геймпада (`Esc` отменяет), записывает в `InputMap` и шлёт `RebindEvent::{Bound { conflicts }, Cancelled}` (пересечения с другими действиями только сообщаются). Пока идёт ожидание, действия читаются как неактивные. `Rebind.autosave` сохраняет карту в RON после каждого изменения. Также: `set_binding`, `remove_binding`, `reset_action`, `conflicts`.

## Подсказки и вибрация

`Device::{KeyboardMouse, Gamepad}` (чего игрок касался последним), `PadStyle::{Xbox, PlayStation, Nintendo}`; `InputMap::hint(action, device, style)` возвращает подпись («Space», «Cross», «W/A/S/D»). Вибрация: ресурс `Haptics { enabled, intensity, supported }`: `rumble(strong, weak, duration)`, `pulse`, `stop`. Без фичи `gamepad` очередь просто сбрасывается.

## Из Lua

`ctx:pressed`, `ctx:just_pressed`, `ctx:just_released`, `ctx:value`, `ctx:axis2d` ([справочник](../reference/lua-api.md#ввод)).

## UI и ввод

Игровой UI «съедает» ввод под указателем и на сфокусированном виджете: см. [Игровой UI](game-ui.md).

## См. также

- [Игровой UI](game-ui.md)
- [Скрипты](scripting.md)
- [Туториал: шаг 4](../tutorial/04-player.md)
- [Горячие клавиши редактора](../editor/hotkeys.md)
