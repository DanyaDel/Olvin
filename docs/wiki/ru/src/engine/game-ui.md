# Игровой UI

`olvin_gui` (дерево узлов, раскладка, фокус) и `olvin_text` (шрифт, атлас, раскладка строк): UI в рантайме без egui. Модуль `gui` (фича фасада; в каталоге хаба его нет, добавляется в `Cargo.toml` вручную).

## Текст

`olvin_text` без GPU и окна: `Font` (TrueType из байтов; `Font::builtin()` это Inter 4.0 Regular, латиница + Latin-1/Extended-A + кириллица, 41 КБ, OFL), `GlyphAtlas` (кэш растеризованных глифов, полочная упаковка), `layout(..)` (строки по `\n`, UTF-8 по символам, без лигатур и RTL). `set_font` меняет шрифт на лету.

## Дерево узлов

`Gui { ui, .. }` ресурс. Узлы: `Node::panel()`, `label(text)`, `button(text)`; макет `row()`, `column()`, `Layout::Overlay`; размеры `px`, `fill`, `Dim::{Fixed, Content, Fill(вес)}`; `gap`, `padding`, `align`, `justify`, `anchor(anchor, pivot, offset)`, `centered`, `name`, `disabled`, `hidden`. Раскладка как flex: размеры по содержимому снизу вверх, затем сверху вниз.

```rust
let button = ui.add(menu, Node::button("Resume").name("resume"));
```

## Ввод

`GuiPlugin` добавляет действия `UiNavigate`, `UiAccept`, `UiCancel`, `UiNext`, `UiPrev` в `InputMap`, если их нет. Фокус: Tab, стрелки (ближайшая кнопка в направлении), мышь (наведение), геймпад (`NavRepeat` для стика и d-pad). События `UiEvent::{Click { name, .. }, Focus, Cancel}` лежат в `Events<UiEvent>`. Меню работают на паузе: ввод UI идёт по реальному времени.

UI «съедает» ввод: пока указатель над кнопкой или панелью с фоном, действия на кнопках мыши (`Fire`) читаются как idle; пока сфокусирован виджет, так же читаются действия на его клавишах (`Move` на WASD, `Jump` на пробеле); `UiCancel` (Esc) не съедается.

## Отрисовка

`GuiRenderPlugin` (фича `render`) превращает `DrawList` в квады `Overlay`, всё меню это один текстурный батч. Рисуется после постобработки и тонмаппинга, работает на Compat. Размеры в UI-пикселях, масштаб: `SurfaceTarget.scale_factor × Gui::ui_scale`. Тема своя (`Theme`), к `olvin_ui` не привязана. Пример: `apps/vertical_slice/src/hud.rs` (прицел, toast, «Загрузка…», меню паузы).

> **Статус:** не сделано: перенос строк (только явный `\n`) и обрезка, поля ввода, слайдеры, прокрутка, узел-картинка (есть `Overlay::image`), 9-slice.

## См. также

- [Туториал: шаг 7](../tutorial/07-ui.md)
- [Ввод](input.md)
- [Рендер](rendering.md)
- [Состояния и сохранения](game-states-saves.md)
