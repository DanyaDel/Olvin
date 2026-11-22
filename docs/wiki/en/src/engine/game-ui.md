# Game UI

`olvin_gui` (a node tree, layout, focus) and `olvin_text` (font, atlas, line layout): runtime UI without egui. The `gui` module (a facade feature; it is not in the hub catalog, add it to `Cargo.toml` by hand).

## Text

`olvin_text` needs no GPU or window: `Font` (TrueType from bytes; `Font::builtin()` is Inter 4.0 Regular, Latin + Latin-1/Extended-A + Cyrillic, 41 KB, OFL), `GlyphAtlas` (a cache of rasterized glyphs, shelf packing), `layout(..)` (lines split by `\n`, UTF-8 by characters, no ligatures or RTL). `set_font` changes the font on the fly.

## The node tree

`Gui { ui, .. }` is a resource. Nodes: `Node::panel()`, `label(text)`, `button(text)`; layout `row()`, `column()`, `Layout::Overlay`; sizes `px`, `fill`, `Dim::{Fixed, Content, Fill(weight)}`; `gap`, `padding`, `align`, `justify`, `anchor(anchor, pivot, offset)`, `centered`, `name`, `disabled`, `hidden`. Layout is flex-like: sizes by content bottom-up, then top-down.

```rust
let button = ui.add(menu, Node::button("Resume").name("resume"));
```

## Input

`GuiPlugin` adds the actions `UiNavigate`, `UiAccept`, `UiCancel`, `UiNext`, `UiPrev` to `InputMap` if they are missing. Focus: Tab, arrows (the nearest button in a direction), mouse (hover), gamepad (`NavRepeat` for stick and d-pad). Events `UiEvent::{Click { name, .. }, Focus, Cancel}` are in `Events<UiEvent>`. Menus work during pause: UI input runs on real time.

The UI "eats" input: while the pointer is over a button or a panel with a background, actions on mouse buttons (`Fire`) read as idle; while a widget is focused, actions on its keys read the same way (`Move` on WASD, `Jump` on space); `UiCancel` (Esc) is not eaten.

## Rendering

`GuiRenderPlugin` (feature `render`) turns a `DrawList` into `Overlay` quads, the whole menu is one textured batch. It is drawn after post-processing and tonemapping and works on Compat. Sizes are in UI pixels, scale: `SurfaceTarget.scale_factor × Gui::ui_scale`. The theme is its own (`Theme`), not tied to `olvin_ui`. Example: `apps/vertical_slice/src/hud.rs` (crosshair, toast, "Загрузка…", pause menu).

> **Status:** not done: line wrapping (only an explicit `\n`) and clipping, text fields, sliders, scrolling, an image node (`Overlay::image` exists), 9-slice.

## See also

- [Tutorial: step 7](../tutorial/07-ui.md)
- [Input](input.md)
- [Rendering](rendering.md)
- [States and saves](game-states-saves.md)
