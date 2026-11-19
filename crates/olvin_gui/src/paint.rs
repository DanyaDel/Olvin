//! Turns the laid-out tree into a [`DrawList`]. The default look is deliberately
//! plain and themable; it has no connection to the editor's egui themes.

use crate::NodeId;
use crate::draw::{DrawList, TextAlign, TextRun};
use crate::geom::Color;
use crate::node::Kind;
use crate::ui::{ButtonState, Ui};

/// Colours and metrics of the default look.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub text: Color,
    pub text_disabled: Color,
    pub button: Color,
    pub button_hovered: Color,
    pub button_focused: Color,
    pub button_pressed: Color,
    pub button_disabled: Color,
    /// Outline around the focused button (gamepad users need to see it).
    pub focus_ring: Color,
    pub focus_ring_width: f32,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            text: Color::rgb(0.93, 0.94, 0.96),
            text_disabled: Color::rgb(0.5, 0.52, 0.56),
            button: Color::rgb(0.16, 0.17, 0.2),
            button_hovered: Color::rgb(0.22, 0.24, 0.28),
            button_focused: Color::rgb(0.2, 0.22, 0.27),
            button_pressed: Color::rgb(0.1, 0.11, 0.13),
            button_disabled: Color::rgb(0.12, 0.12, 0.14),
            focus_ring: Color::rgb(0.35, 0.65, 1.0),
            focus_ring_width: 2.0,
        }
    }
}

impl Ui {
    /// Fills `out` (cleared first) with this frame's quads and text, parents
    /// before children. Nodes the last layout did not reach are skipped.
    pub fn paint(&self, theme: &Theme, out: &mut DrawList) {
        out.clear();
        self.paint_node(self.root(), theme, out);
    }

    fn paint_node(&self, id: NodeId, theme: &Theme, out: &mut DrawList) {
        let n = &self.nodes[id.idx()];
        if !n.shown {
            return;
        }
        let state = self.state(id);
        let (bg, text_color) = match n.kind {
            Kind::Button => {
                let bg = match state {
                    ButtonState::Normal => theme.button,
                    ButtonState::Hovered => theme.button_hovered,
                    ButtonState::Focused => theme.button_focused,
                    ButtonState::Pressed => theme.button_pressed,
                    ButtonState::Disabled => theme.button_disabled,
                };
                let fg = if state == ButtonState::Disabled {
                    theme.text_disabled
                } else {
                    theme.text
                };
                (Some(bg), n.color.unwrap_or(fg))
            }
            _ => (n.background, n.color.unwrap_or(theme.text)),
        };
        if let Some(bg) = bg {
            out.quad(n.rect, bg);
        }
        if n.kind == Kind::Button && self.focused() == Some(id) {
            out.outline(n.rect, theme.focus_ring_width, theme.focus_ring);
        }
        if matches!(n.kind, Kind::Label | Kind::Button) {
            out.text(TextRun {
                rect: n.rect.shrink(n.padding),
                text: n.text.clone(),
                size: n.text_size,
                color: text_color,
                align: if n.kind == Kind::Button {
                    TextAlign::Center
                } else {
                    TextAlign::Left
                },
            });
        }
        for &c in &n.children {
            self.paint_node(c, theme, out);
        }
    }
}
