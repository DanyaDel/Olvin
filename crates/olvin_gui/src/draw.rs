//! What the UI hands to a renderer: flat lists of quads and text runs. The UI
//! never touches the GPU; a backend turns [`DrawList`] into vertex buffers
//! (see [`DrawList::vertices`]) and glyphs.

use crate::geom::{Color, Rect};

/// A filled rectangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad {
    pub rect: Rect,
    pub color: Color,
}

/// Horizontal placement of a text run inside its rect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
}

/// Text to draw inside `rect` (vertically centred). Shaping and the font are the backend's.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
    pub rect: Rect,
    pub text: String,
    pub size: f32,
    pub color: Color,
    pub align: TextAlign,
}

/// Vertex of a UI quad: pixel position and colour. `repr(C)`, 24 bytes, so a
/// backend can upload a slice as is (no uv: quads are untextured).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuiVertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
}

/// One frame of UI output in paint order (later is on top). Quads of a node
/// come before its text, but quads of a later node may cover earlier text:
/// backends that care draw by [`DrawList::order`] instead of two passes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrawList {
    pub quads: Vec<Quad>,
    pub texts: Vec<TextRun>,
    /// Paint order across both lists, in the order items were pushed.
    pub order: Vec<DrawItem>,
}

/// An index into [`DrawList::quads`] or [`DrawList::texts`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawItem {
    Quad(usize),
    Text(usize),
}

impl DrawList {
    /// Empties the lists but keeps their capacity.
    pub fn clear(&mut self) {
        self.quads.clear();
        self.texts.clear();
        self.order.clear();
    }

    pub fn quad(&mut self, rect: Rect, color: Color) {
        if color.0[3] <= 0.0 || rect.size().x <= 0.0 || rect.size().y <= 0.0 {
            return;
        }
        self.order.push(DrawItem::Quad(self.quads.len()));
        self.quads.push(Quad { rect, color });
    }

    pub fn text(&mut self, run: TextRun) {
        if run.text.is_empty() {
            return;
        }
        self.order.push(DrawItem::Text(self.texts.len()));
        self.texts.push(run);
    }

    /// A rectangular outline of `width` pixels drawn inside `rect` (four quads).
    pub fn outline(&mut self, rect: Rect, width: f32, color: Color) {
        use glam::Vec2;
        let s = rect.size();
        let w = width.min(s.x * 0.5).min(s.y * 0.5);
        let top = Rect::new(rect.min, Vec2::new(s.x, w));
        let bottom = Rect::new(Vec2::new(rect.min.x, rect.max.y - w), Vec2::new(s.x, w));
        let left = Rect::new(
            Vec2::new(rect.min.x, rect.min.y + w),
            Vec2::new(w, s.y - 2.0 * w),
        );
        let right = Rect::new(
            Vec2::new(rect.max.x - w, rect.min.y + w),
            Vec2::new(w, s.y - 2.0 * w),
        );
        for r in [top, bottom, left, right] {
            self.quad(r, color);
        }
    }

    /// Writes the quads as two triangles each (6 vertices per quad) into `out`,
    /// clearing it first. Winding is clockwise in screen space.
    pub fn vertices(&self, out: &mut Vec<GuiVertex>) {
        out.clear();
        out.reserve(self.quads.len() * 6);
        for q in &self.quads {
            let (a, b) = (q.rect.min, q.rect.max);
            let v = |x: f32, y: f32| GuiVertex {
                position: [x, y],
                color: q.color.0,
            };
            out.extend([
                v(a.x, a.y),
                v(b.x, a.y),
                v(b.x, b.y),
                v(a.x, a.y),
                v(b.x, b.y),
                v(a.x, b.y),
            ]);
        }
    }
}
