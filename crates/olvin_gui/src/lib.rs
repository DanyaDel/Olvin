//! In-game UI, separate from egui (which is editor-only): buttons, labels and
//! panels in a retained tree; flex-like rows and columns, overlays and
//! anchors; focus with directional navigation for gamepad and keyboard;
//! click events. Depends on `olvin_core`, `olvin_input` and `glam` only.
//!
//! The core does not draw. [`Ui::paint`] produces a [`DrawList`] (quads and
//! text runs, see [`DrawList::vertices`] for ready vertex data) that a
//! renderer turns into pixels. Text is measured through [`TextMeasure`], so
//! layout can use real font metrics. Feature `render` adds that renderer:
//! `GuiRenderPlugin` lays text out with `olvin_text` and fills
//! `olvin_render::Overlay`, drawn over the finished frame.
//!
//! Sizes are UI pixels: physical pixels divided by [`Gui::scale`] (display
//! scale × [`Gui::ui_scale`]), so a menu keeps its size on a HiDPI screen.
//!
//! Everything in [`Ui`], [`NavRepeat`] and [`DrawList`] is plain data and works
//! without an `App`; [`GuiPlugin`] only wires it to `olvin_input` actions.

mod draw;
mod geom;
mod layout;
mod nav;
mod node;
mod paint;
mod plugin;
#[cfg(feature = "render")]
mod render;
mod ui;

pub use draw::{DrawItem, DrawList, GuiVertex, Quad, TextAlign, TextRun};
pub use geom::{Color, Dir, Rect};
pub use layout::{Monospace, TextMeasure};
pub use nav::NavRepeat;
pub use node::{Align, Dim, Justify, Layout, Node, NodeId, Place};
/// Fonts, glyph atlas and text layout the renderer uses (feature `render`).
#[cfg(feature = "render")]
pub use olvin_text as text;
pub use paint::Theme;
pub use plugin::{Gui, GuiPlugin, UiActions};
#[cfg(feature = "render")]
pub use render::{FontMeasure, GuiRenderPlugin, GuiText, set_font};
pub use ui::{ButtonState, Ui, UiEvent, UiInput};
