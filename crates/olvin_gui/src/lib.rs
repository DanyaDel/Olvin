//! In-game UI, separate from egui (which is editor-only): buttons, labels and
//! panels in a retained tree; flex-like rows and columns, overlays and
//! anchors; focus with directional navigation for gamepad and keyboard;
//! click events. Depends on `olvin_core`, `olvin_input` and `glam` only.
//!
//! The UI does not draw. [`Ui::paint`] produces a [`DrawList`] (quads and text
//! runs, see [`DrawList::vertices`] for ready vertex data) that a renderer
//! turns into pixels; font shaping and the GPU pass are that renderer's job.
//! Text is measured through [`TextMeasure`], so layout can use real font metrics.
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
mod ui;

pub use draw::{DrawItem, DrawList, GuiVertex, Quad, TextAlign, TextRun};
pub use geom::{Color, Dir, Rect};
pub use layout::{Monospace, TextMeasure};
pub use nav::NavRepeat;
pub use node::{Align, Dim, Justify, Layout, Node, NodeId, Place};
pub use paint::Theme;
pub use plugin::{Gui, GuiPlugin, UiActions};
pub use ui::{ButtonState, Ui, UiEvent, UiInput};
