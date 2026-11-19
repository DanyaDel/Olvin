//! Layout: bottom-up content sizes, top-down placement. Flex-like rows and
//! columns, overlays, anchors. Text size comes from a [`TextMeasure`].

use glam::Vec2;

use crate::geom::Rect;
use crate::node::{Align, Dim, Justify, Kind, Layout, NodeId, Place};
use crate::ui::Ui;

/// Measures text for layout. The renderer's font knows the truth; use it when
/// there is one (the layout then matches the glyphs exactly).
pub trait TextMeasure {
    /// Size in pixels of `text` (may contain `\n`) at font size `size`.
    fn measure(&self, text: &str, size: f32) -> Vec2;
}

/// Fixed-advance estimate: every character is `advance` em wide, a line is
/// `line_height` em tall. Good for tests and as a fallback before fonts exist.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Monospace {
    pub advance: f32,
    pub line_height: f32,
}

impl Default for Monospace {
    fn default() -> Self {
        Self {
            advance: 0.55,
            line_height: 1.25,
        }
    }
}

impl TextMeasure for Monospace {
    fn measure(&self, text: &str, size: f32) -> Vec2 {
        let lines = text.split('\n');
        let count = lines.clone().count();
        let widest = lines.map(|l| l.chars().count()).max().unwrap_or(0);
        Vec2::new(
            widest as f32 * self.advance * size,
            count as f32 * self.line_height * size,
        )
    }
}

fn axis(v: Vec2, a: usize) -> f32 {
    if a == 0 { v.x } else { v.y }
}

fn with_axis(main: f32, cross: f32, a: usize) -> Vec2 {
    if a == 0 {
        Vec2::new(main, cross)
    } else {
        Vec2::new(cross, main)
    }
}

impl Ui {
    /// Lays the tree out in a viewport of `viewport` pixels (root fills it).
    pub fn layout(&mut self, viewport: Vec2, m: &dyn TextMeasure) {
        self.viewport = viewport;
        for n in &mut self.nodes {
            n.shown = false;
        }
        self.nodes[0].shown = true;
        self.arrange(NodeId(0), Rect::new(Vec2::ZERO, viewport), m);
    }

    /// Size a node asks for: fixed dimensions as given, the rest from its content.
    fn preferred(&self, id: NodeId, m: &dyn TextMeasure) -> Vec2 {
        let n = &self.nodes[id.idx()];
        let content = match n.kind {
            Kind::Label | Kind::Button => {
                m.measure(&n.text, n.text_size) + Vec2::splat(2.0 * n.padding)
            }
            Kind::Panel => {
                let pad = Vec2::splat(2.0 * n.padding);
                let a = usize::from(n.layout == Layout::Column);
                let mut overlay = Vec2::ZERO;
                let (mut main, mut cross, mut flow) = (0.0f32, 0.0f32, 0usize);
                for &c in &n.children {
                    let cn = &self.nodes[c.idx()];
                    if !cn.visible {
                        continue;
                    }
                    let p = self.preferred(c, m);
                    overlay = overlay.max(p);
                    if cn.place == Place::Flow {
                        main += axis(p, a);
                        cross = cross.max(axis(p, 1 - a));
                        flow += 1;
                    }
                }
                if n.layout == Layout::Overlay {
                    overlay + pad
                } else {
                    main += n.gap * flow.saturating_sub(1) as f32;
                    with_axis(main, cross, a) + pad
                }
            }
        };
        Vec2::new(
            match n.width {
                Dim::Fixed(v) => v,
                _ => content.x,
            },
            match n.height {
                Dim::Fixed(v) => v,
                _ => content.y,
            },
        )
    }

    /// Size of a child that is not sized by flex (overlay, anchored, cross axis).
    fn resolve(&self, id: NodeId, space: Vec2, m: &dyn TextMeasure) -> Vec2 {
        let n = &self.nodes[id.idx()];
        let p = self.preferred(id, m);
        Vec2::new(
            match n.width {
                Dim::Fill(_) => space.x,
                _ => p.x,
            },
            match n.height {
                Dim::Fill(_) => space.y,
                _ => p.y,
            },
        )
    }

    fn arrange(&mut self, id: NodeId, rect: Rect, m: &dyn TextMeasure) {
        let i = id.idx();
        self.nodes[i].rect = rect;
        let (layout, pad, gap, align, justify) = {
            let n = &self.nodes[i];
            (n.layout, n.padding, n.gap, n.align, n.justify)
        };
        let inner = rect.shrink(pad);
        let isize = inner.size();
        let count = self.nodes[i].children.len();

        // Anchored children first: independent of the flow.
        for k in 0..count {
            let c = self.nodes[i].children[k];
            let cn = &self.nodes[c.idx()];
            if !cn.visible {
                continue;
            }
            if let Place::Anchor {
                anchor,
                pivot,
                offset,
            } = cn.place
            {
                let size = self.resolve(c, isize, m);
                let pos = inner.min + anchor * isize + offset - pivot * size;
                self.show(c, Rect::new(pos, size), m);
            }
        }

        let flow: Vec<NodeId> = self.nodes[i]
            .children
            .iter()
            .copied()
            .filter(|c| {
                let cn = &self.nodes[c.idx()];
                cn.visible && cn.place == Place::Flow
            })
            .collect();

        if layout == Layout::Overlay {
            for c in flow {
                let size = self.resolve(c, isize, m);
                self.show(c, Rect::new(inner.min, size), m);
            }
            return;
        }

        let a = usize::from(layout == Layout::Column);
        let main_space = axis(isize, a);
        let cross_space = axis(isize, 1 - a);
        let gaps = gap * flow.len().saturating_sub(1) as f32;

        // Main sizes: fixed and content first, then the weighted share of the rest.
        let mut used = gaps;
        let mut weights = 0.0;
        for &c in &flow {
            let cn = &self.nodes[c.idx()];
            match if a == 0 { cn.width } else { cn.height } {
                Dim::Fill(w) => weights += w.max(0.0),
                _ => used += axis(self.preferred(c, m), a),
            }
        }
        let free = (main_space - used).max(0.0);
        let mut offset = 0.0;
        let mut between = gap;
        if weights == 0.0 {
            match justify {
                Justify::Start => {}
                Justify::Center => offset = free * 0.5,
                Justify::End => offset = free,
                Justify::SpaceBetween if flow.len() > 1 => {
                    between = gap + free / (flow.len() - 1) as f32;
                }
                Justify::SpaceBetween => {}
            }
        }

        for &c in &flow {
            let cn = &self.nodes[c.idx()];
            let (main_dim, cross_dim) = if a == 0 {
                (cn.width, cn.height)
            } else {
                (cn.height, cn.width)
            };
            let p = self.preferred(c, m);
            let main = match main_dim {
                Dim::Fill(w) => free * w.max(0.0) / weights,
                _ => axis(p, a),
            };
            let cross = match cross_dim {
                Dim::Fill(_) => cross_space,
                Dim::Content if align == Align::Stretch => cross_space,
                _ => axis(p, 1 - a),
            };
            let cross_off = match align {
                Align::Start | Align::Stretch => 0.0,
                Align::Center => (cross_space - cross) * 0.5,
                Align::End => cross_space - cross,
            };
            let pos = inner.min + with_axis(offset, cross_off, a);
            self.show(c, Rect::new(pos, with_axis(main, cross, a)), m);
            offset += main + between;
        }
    }

    fn show(&mut self, id: NodeId, rect: Rect, m: &dyn TextMeasure) {
        self.nodes[id.idx()].shown = true;
        self.arrange(id, rect, m);
    }
}
