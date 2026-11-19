//! Nodes of the UI tree and their layout properties (builder style).

use glam::Vec2;

use crate::geom::{Color, Rect};

/// Index of a node in a [`Ui`](crate::Ui). Ids are not reused until
/// [`Ui::clear`](crate::Ui::clear), so a stale id is merely dead, never wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(pub(crate) u32);

impl NodeId {
    pub(crate) fn idx(self) -> usize {
        self.0 as usize
    }
}

/// Size along one axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Dim {
    /// Exactly this many pixels.
    Fixed(f32),
    /// Fit the content (text, or the children).
    Content,
    /// Take a share of the free space: weight among `Fill` siblings in a row or
    /// column; the whole inner size in an overlay or when anchored.
    Fill(f32),
}

/// How a node arranges its children.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Flow children are stacked on top of each other at the top-left corner;
    /// anchored children go where their anchor says.
    Overlay,
    /// Left to right.
    Row,
    /// Top to bottom.
    Column,
}

/// Cross-axis alignment in a row or column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
    /// Content-sized children take the whole cross size.
    Stretch,
}

/// Main-axis placement of the leftover space (when no child is `Fill`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Justify {
    Start,
    Center,
    End,
    /// Equal gaps between children, none at the ends.
    SpaceBetween,
}

/// Where a node goes in its parent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Place {
    /// Positioned by the parent's layout.
    Flow,
    /// Pinned inside the parent's padded rect: the point `anchor` (0..1 of the
    /// parent) plus `offset` pixels lines up with the point `pivot` (0..1) of this node.
    Anchor {
        anchor: Vec2,
        pivot: Vec2,
        offset: Vec2,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Kind {
    Panel,
    Label,
    Button,
}

/// One node: build with [`Node::panel`], [`Node::label`], [`Node::button`] and
/// the chained setters, then add with [`Ui::add`](crate::Ui::add).
#[derive(Clone, Debug)]
pub struct Node {
    pub(crate) kind: Kind,
    pub(crate) parent: Option<NodeId>,
    pub(crate) children: Vec<NodeId>,
    pub(crate) alive: bool,
    pub name: Option<String>,
    pub text: String,
    pub text_size: f32,
    pub layout: Layout,
    pub place: Place,
    pub width: Dim,
    pub height: Dim,
    /// Space between children and the node's edge (all sides).
    pub padding: f32,
    /// Space between flow children in a row or column.
    pub gap: f32,
    pub align: Align,
    pub justify: Justify,
    /// Drawn behind the node when set (panels and labels; buttons use the theme).
    pub background: Option<Color>,
    /// Overrides the theme text colour.
    pub color: Option<Color>,
    pub visible: bool,
    /// Buttons only: a disabled button is skipped by focus and clicks.
    pub enabled: bool,
    pub(crate) rect: Rect,
    /// Reached by the last layout (visible with all ancestors).
    pub(crate) shown: bool,
}

impl Node {
    fn base(kind: Kind, text: &str) -> Self {
        Self {
            kind,
            parent: None,
            children: Vec::new(),
            alive: true,
            name: None,
            text: text.to_owned(),
            text_size: 18.0,
            layout: Layout::Overlay,
            place: Place::Flow,
            width: Dim::Content,
            height: Dim::Content,
            padding: 0.0,
            gap: 0.0,
            align: Align::Start,
            justify: Justify::Start,
            background: None,
            color: None,
            visible: true,
            enabled: true,
            rect: Rect::default(),
            shown: false,
        }
    }

    /// A container with no visuals of its own (set `background` for a plate).
    pub fn panel() -> Self {
        Self::base(Kind::Panel, "")
    }

    pub fn label(text: &str) -> Self {
        Self::base(Kind::Label, text)
    }

    /// A focusable, clickable button. Padding defaults to 12 px.
    pub fn button(text: &str) -> Self {
        let mut n = Self::base(Kind::Button, text);
        n.padding = 12.0;
        n
    }

    /// Name reported in [`UiEvent`](crate::UiEvent)s.
    pub fn name(mut self, name: &str) -> Self {
        self.name = Some(name.to_owned());
        self
    }

    pub fn text_size(mut self, size: f32) -> Self {
        self.text_size = size;
        self
    }

    pub fn row(mut self) -> Self {
        self.layout = Layout::Row;
        self
    }

    pub fn column(mut self) -> Self {
        self.layout = Layout::Column;
        self
    }

    pub fn size(mut self, width: Dim, height: Dim) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// Fixed size in pixels.
    pub fn px(self, width: f32, height: f32) -> Self {
        self.size(Dim::Fixed(width), Dim::Fixed(height))
    }

    /// Fills the parent (or its share of it).
    pub fn fill(self) -> Self {
        self.size(Dim::Fill(1.0), Dim::Fill(1.0))
    }

    pub fn padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    pub fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    pub fn justify(mut self, justify: Justify) -> Self {
        self.justify = justify;
        self
    }

    pub fn background(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    pub fn hidden(mut self) -> Self {
        self.visible = false;
        self
    }

    /// Pins to a point of the parent (see [`Place::Anchor`]).
    pub fn anchor(mut self, anchor: Vec2, pivot: Vec2, offset: Vec2) -> Self {
        self.place = Place::Anchor {
            anchor,
            pivot,
            offset,
        };
        self
    }

    /// Centred in the parent.
    pub fn centered(self) -> Self {
        self.anchor(Vec2::splat(0.5), Vec2::splat(0.5), Vec2::ZERO)
    }

    /// Computed rectangle from the last layout (zero before the first).
    pub fn rect(&self) -> Rect {
        self.rect
    }

    pub(crate) fn is_button(&self) -> bool {
        self.kind == Kind::Button
    }
}
