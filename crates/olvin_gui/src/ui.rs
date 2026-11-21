//! The UI tree, focus, navigation and click handling. Pure data: no windows,
//! no GPU, no `olvin_core` types, so everything here is testable by calling methods.

use glam::Vec2;

use crate::geom::Dir;
use crate::node::{Node, NodeId};

/// Something the user did to the UI, read by game code after [`Ui::update`].
#[derive(Clone, Debug, PartialEq)]
pub enum UiEvent {
    /// A button was pressed and released (pointer on it, or accept while focused).
    Click { node: NodeId, name: Option<String> },
    /// Focus moved to a button (navigation or pointer hover).
    Focus { node: NodeId, name: Option<String> },
    /// The cancel/back action fired.
    Cancel,
}

/// One frame of input for the UI, already reduced to what it needs. The
/// plugin fills it from [`olvin_input`] actions; tests fill it by hand.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UiInput {
    /// Pointer in pixels (+Y down); `None` when there is no pointer (cursor locked, gamepad only).
    pub pointer: Option<Vec2>,
    pub pointer_pressed: bool,
    pub pointer_released: bool,
    /// One navigation step this frame (already repeat-filtered, see [`NavRepeat`](crate::NavRepeat)).
    pub nav: Option<Dir>,
    /// Tab order: next / previous focusable node.
    pub next: bool,
    pub prev: bool,
    pub accept_pressed: bool,
    pub accept_released: bool,
    pub cancel: bool,
}

/// Visual state of a button, for painting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonState {
    Normal,
    Hovered,
    Focused,
    Pressed,
    Disabled,
}

/// A retained tree of nodes. Node 0 is the root and always exists; it fills the viewport.
#[derive(Debug)]
pub struct Ui {
    pub(crate) nodes: Vec<Node>,
    pub(crate) viewport: Vec2,
    focus: Option<NodeId>,
    hover: Option<NodeId>,
    pressed: Option<NodeId>,
    last_pointer: Option<Vec2>,
    /// Where the current pointer press began: on the UI (`Some(true)`) or in the game.
    grab: Option<bool>,
    pointer_captured: bool,
    events: Vec<UiEvent>,
}

impl Default for Ui {
    fn default() -> Self {
        Self::new()
    }
}

impl Ui {
    pub fn new() -> Self {
        let mut root = Node::panel().fill();
        root.parent = None;
        Self {
            nodes: vec![root],
            viewport: Vec2::ZERO,
            focus: None,
            hover: None,
            pressed: None,
            last_pointer: None,
            grab: None,
            pointer_captured: false,
            events: Vec::new(),
        }
    }

    pub fn root(&self) -> NodeId {
        NodeId(0)
    }

    /// Removes everything but the root and resets focus.
    pub fn clear(&mut self) {
        self.nodes.truncate(1);
        self.nodes[0].children.clear();
        self.focus = None;
        self.hover = None;
        self.pressed = None;
        self.grab = None;
        self.pointer_captured = false;
        self.events.clear();
    }

    /// Adds `node` as the last child of `parent` and returns its id.
    ///
    /// # Panics
    /// If `parent` is not a live node of this tree.
    pub fn add(&mut self, parent: NodeId, mut node: Node) -> NodeId {
        assert!(self.alive(parent), "parent {parent:?} is not a live node");
        let id = NodeId(self.nodes.len() as u32);
        node.parent = Some(parent);
        node.children.clear();
        node.alive = true;
        self.nodes.push(node);
        self.nodes[parent.idx()].children.push(id);
        id
    }

    /// Removes a node with its subtree. The root cannot be removed.
    pub fn remove(&mut self, id: NodeId) {
        if id.0 == 0 || !self.alive(id) {
            return;
        }
        if let Some(p) = self.nodes[id.idx()].parent {
            self.nodes[p.idx()].children.retain(|&c| c != id);
        }
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            let node = &mut self.nodes[n.idx()];
            node.alive = false;
            node.shown = false;
            stack.append(&mut node.children);
            for slot in [&mut self.focus, &mut self.hover, &mut self.pressed] {
                if *slot == Some(n) {
                    *slot = None;
                }
            }
        }
    }

    pub fn alive(&self, id: NodeId) -> bool {
        self.nodes.get(id.idx()).is_some_and(|n| n.alive)
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.idx()).filter(|n| n.alive)
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id.idx()).filter(|n| n.alive)
    }

    /// First live node with this name (creation order).
    pub fn find(&self, name: &str) -> Option<NodeId> {
        self.nodes
            .iter()
            .position(|n| n.alive && n.name.as_deref() == Some(name))
            .map(|i| NodeId(i as u32))
    }

    pub fn set_text(&mut self, id: NodeId, text: &str) {
        if let Some(n) = self.node_mut(id) {
            n.text.clear();
            n.text.push_str(text);
        }
    }

    pub fn set_enabled(&mut self, id: NodeId, enabled: bool) {
        if let Some(n) = self.node_mut(id) {
            n.enabled = enabled;
        }
    }

    pub fn set_visible(&mut self, id: NodeId, visible: bool) {
        if let Some(n) = self.node_mut(id) {
            n.visible = visible;
        }
    }

    pub fn rect(&self, id: NodeId) -> crate::Rect {
        self.node(id).map(Node::rect).unwrap_or_default()
    }

    /// Events produced by the last [`Ui::update`].
    pub fn events(&self) -> &[UiEvent] {
        &self.events
    }

    pub fn focused(&self) -> Option<NodeId> {
        self.focus
    }

    pub fn hovered(&self) -> Option<NodeId> {
        self.hover
    }

    /// The pointer belongs to the UI this frame (as of the last [`Ui::update`]):
    /// it is over a button or a panel with a background, or a press that began
    /// on the UI is still held. The game should not treat that click as its own.
    pub fn pointer_captured(&self) -> bool {
        self.pointer_captured
    }

    /// Navigation keys and buttons belong to the UI: a widget is focused.
    /// (A menu eats arrows, WASD and Space/Enter; the first navigation step
    /// focuses the first button, so a HUD with buttons should not bind its
    /// navigation action to gameplay keys.)
    pub fn keyboard_captured(&self) -> bool {
        self.focus.is_some()
    }

    /// Is there UI under `p`: a button, or a panel/label with a background
    /// (the root never counts)? Transparent containers let clicks through.
    pub fn covers(&self, p: Vec2) -> bool {
        self.nodes.iter().skip(1).any(|n| {
            n.alive && n.shown && (n.is_button() || n.background.is_some()) && n.rect.contains(p)
        })
    }

    /// Can this node take focus and clicks right now (button, enabled, shown by the last layout)?
    pub fn focusable(&self, id: NodeId) -> bool {
        self.node(id)
            .is_some_and(|n| n.is_button() && n.enabled && n.shown)
    }

    pub fn state(&self, id: NodeId) -> ButtonState {
        let Some(n) = self.node(id) else {
            return ButtonState::Normal;
        };
        if !n.enabled {
            ButtonState::Disabled
        } else if self.pressed == Some(id) && (self.hover == Some(id) || self.focus == Some(id)) {
            ButtonState::Pressed
        } else if self.focus == Some(id) {
            ButtonState::Focused
        } else if self.hover == Some(id) {
            ButtonState::Hovered
        } else {
            ButtonState::Normal
        }
    }

    /// Gives focus to a focusable node (sends [`UiEvent::Focus`] if it changed).
    pub fn set_focus(&mut self, id: NodeId) {
        if self.focusable(id) && self.focus != Some(id) {
            self.focus = Some(id);
            let name = self.nodes[id.idx()].name.clone();
            self.events.push(UiEvent::Focus { node: id, name });
        }
    }

    /// Focusable nodes in tree order (the Tab order).
    fn order(&self) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack = vec![self.root()];
        while let Some(id) = stack.pop() {
            let n = &self.nodes[id.idx()];
            if !n.shown {
                continue;
            }
            if self.focusable(id) {
                out.push(id);
            }
            stack.extend(n.children.iter().rev());
        }
        out
    }

    /// Focuses the first focusable node, e.g. when a menu opens.
    pub fn focus_first(&mut self) {
        if let Some(&first) = self.order().first() {
            self.set_focus(first);
        }
    }

    pub fn focus_next(&mut self) {
        self.step(1);
    }

    pub fn focus_prev(&mut self) {
        self.step(-1);
    }

    fn step(&mut self, by: isize) {
        let order = self.order();
        if order.is_empty() {
            return;
        }
        let len = order.len() as isize;
        let next = match self.focus.and_then(|f| order.iter().position(|&o| o == f)) {
            Some(i) => (i as isize + by).rem_euclid(len),
            None if by > 0 => 0,
            None => len - 1,
        };
        self.set_focus(order[next as usize]);
    }

    /// Moves focus to the nearest focusable node in `dir` (directional
    /// navigation for d-pad, stick and arrows). With nothing focused it
    /// focuses the first node. Stays put when nothing lies that way.
    pub fn navigate(&mut self, dir: Dir) {
        let Some(from) = self.focus else {
            return self.focus_first();
        };
        let c = self.rect(from).center();
        let mut best: Option<(f32, NodeId)> = None;
        for id in self.order() {
            if id == from {
                continue;
            }
            let d = self.rect(id).center() - c;
            let (along, across) = match dir {
                Dir::Up => (-d.y, d.x.abs()),
                Dir::Down => (d.y, d.x.abs()),
                Dir::Left => (-d.x, d.y.abs()),
                Dir::Right => (d.x, d.y.abs()),
            };
            if along <= 0.0 {
                continue;
            }
            // Off-axis distance counts double: prefer the node straight ahead.
            let score = along + 2.0 * across;
            if best.is_none_or(|(s, _)| score < s) {
                best = Some((score, id));
            }
        }
        if let Some((_, id)) = best {
            self.set_focus(id);
        }
    }

    /// Topmost focusable node under `p` (later in tree order paints on top).
    pub fn hit(&self, p: Vec2) -> Option<NodeId> {
        self.hit_in(self.root(), p)
    }

    fn hit_in(&self, id: NodeId, p: Vec2) -> Option<NodeId> {
        let n = &self.nodes[id.idx()];
        if !n.shown {
            return None;
        }
        n.children
            .iter()
            .rev()
            .find_map(|&c| self.hit_in(c, p))
            .or_else(|| (self.focusable(id) && n.rect.contains(p)).then_some(id))
    }

    /// Processes one frame of input against the last layout and fills
    /// [`Ui::events`]. Call [`Ui::layout`](Ui::layout) before it when the tree changed.
    pub fn update(&mut self, input: &UiInput) {
        self.events.clear();
        if self.focus.is_some_and(|f| !self.focusable(f)) {
            self.focus = None;
        }
        if self.pressed.is_some_and(|p| !self.focusable(p)) {
            self.pressed = None;
        }

        let moved = input.pointer.is_some() && input.pointer != self.last_pointer;
        self.last_pointer = input.pointer;
        self.hover = input.pointer.and_then(|p| self.hit(p));
        if moved && let Some(h) = self.hover {
            self.set_focus(h);
        }

        if let Some(dir) = input.nav {
            self.navigate(dir);
        }
        if input.next {
            self.focus_next();
        }
        if input.prev {
            self.focus_prev();
        }

        if input.pointer_pressed {
            self.grab = Some(input.pointer.is_some_and(|p| self.covers(p)));
        }
        if input.pointer_pressed
            && let Some(h) = self.hover
        {
            self.set_focus(h);
            self.pressed = Some(h);
        }
        if input.accept_pressed && self.focus.is_some() {
            self.pressed = self.focus;
        }
        self.pointer_captured = match self.grab {
            Some(on_ui) => on_ui,
            None => input.pointer.is_some_and(|p| self.covers(p)),
        };
        if input.pointer_released {
            self.grab = None;
        }
        if input.pointer_released {
            if self.pressed.is_some() && self.pressed == self.hover {
                self.click();
            }
            self.pressed = None;
        }
        if input.accept_released {
            if self.pressed.is_some() && self.pressed == self.focus {
                self.click();
            }
            self.pressed = None;
        }
        if input.cancel {
            self.events.push(UiEvent::Cancel);
        }
    }

    fn click(&mut self) {
        if let Some(node) = self.pressed {
            let name = self.nodes[node.idx()].name.clone();
            self.events.push(UiEvent::Click { node, name });
        }
    }
}
