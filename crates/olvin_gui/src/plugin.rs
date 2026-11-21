//! Glue to the engine: reads `olvin_input` actions, runs the UI, publishes
//! [`UiEvent`]s and a [`DrawList`].

use glam::Vec2;
use olvin_core::{App, Plugin, Resources, Stage, SurfaceTarget, Time};
use olvin_input::{
    ActionState, Binding, GamepadButton, Input, InputMap, KeyCode, MouseButton, Stick,
};

use crate::draw::DrawList;
use crate::layout::{Monospace, TextMeasure};
use crate::nav::NavRepeat;
use crate::paint::Theme;
use crate::ui::{Ui, UiEvent, UiInput};

/// Names of the `olvin_input` actions the UI listens to. The plugin adds
/// default bindings for any that the map lacks (a player's remapped
/// `input.ron` wins), so menus work with keyboard, mouse and gamepad out of the box.
#[derive(Clone, Debug, PartialEq)]
pub struct UiActions {
    /// 2D: arrows, WASD, d-pad, left stick.
    pub navigate: String,
    /// Enter, Space, gamepad South.
    pub accept: String,
    /// Escape, gamepad East.
    pub cancel: String,
    /// Tab, right bumper.
    pub next: String,
    /// Left bumper.
    pub prev: String,
}

impl Default for UiActions {
    fn default() -> Self {
        Self {
            navigate: "UiNavigate".into(),
            accept: "UiAccept".into(),
            cancel: "UiCancel".into(),
            next: "UiNext".into(),
            prev: "UiPrev".into(),
        }
    }
}

impl UiActions {
    /// Adds the default bindings for actions `map` does not have yet.
    pub fn bind_missing(&self, map: &mut InputMap) {
        let pad = Binding::GamepadButton;
        let key = Binding::Key;
        let mut add = |name: &str, bindings: Vec<Binding>| {
            if !map.actions.contains_key(name) {
                map.bind(name, bindings);
            }
        };
        add(
            &self.navigate,
            vec![
                Binding::arrows(),
                Binding::wasd(),
                Binding::Axis2D {
                    up: Box::new(pad(GamepadButton::DPadUp)),
                    down: Box::new(pad(GamepadButton::DPadDown)),
                    left: Box::new(pad(GamepadButton::DPadLeft)),
                    right: Box::new(pad(GamepadButton::DPadRight)),
                },
                Binding::GamepadStick {
                    stick: Stick::Left,
                    scale: 1.0,
                },
            ],
        );
        add(
            &self.accept,
            vec![
                key(KeyCode::Enter),
                key(KeyCode::Space),
                pad(GamepadButton::South),
            ],
        );
        add(
            &self.cancel,
            vec![key(KeyCode::Escape), pad(GamepadButton::East)],
        );
        add(
            &self.next,
            vec![key(KeyCode::Tab), pad(GamepadButton::RightBumper)],
        );
        add(&self.prev, vec![pad(GamepadButton::LeftBumper)]);
    }
}

/// The in-game UI as a resource: the tree, its look and the per-frame output.
pub struct Gui {
    pub ui: Ui,
    pub theme: Theme,
    pub actions: UiActions,
    pub repeat: NavRepeat,
    /// Text sizes for layout. `GuiRenderPlugin` (feature `render`) puts the
    /// real font here; the default is a [`Monospace`] estimate.
    pub measure: Box<dyn TextMeasure>,
    /// Viewport in UI pixels. The plugin keeps it equal to
    /// `SurfaceTarget.size / scale` when there is one; set it yourself for headless use.
    pub viewport: Vec2,
    /// Extra zoom on top of the display scale (a "UI size" setting), 1 = none.
    pub ui_scale: f32,
    /// Physical pixels per UI pixel: `SurfaceTarget.scale_factor × ui_scale`,
    /// written by the plugin every frame (`ui_scale` alone without a surface).
    /// Layout, node sizes and text sizes are in UI pixels; the renderer
    /// multiplies by this, and the pointer is divided by it.
    pub scale: f32,
    /// Output of the last frame, for the renderer.
    pub draw: DrawList,
}

impl Default for Gui {
    fn default() -> Self {
        Self {
            ui: Ui::new(),
            theme: Theme::default(),
            actions: UiActions::default(),
            repeat: NavRepeat::default(),
            measure: Box::new(Monospace::default()),
            viewport: Vec2::new(1280.0, 720.0),
            ui_scale: 1.0,
            scale: 1.0,
            draw: DrawList::default(),
        }
    }
}

/// Inserts [`Gui`] and `Events<UiEvent>`. Needs `olvin_input::InputPlugin`,
/// added before this plugin (without it the UI simply sees no input).
///
/// Input is handled in `Stage::First`, right behind `update_actions`, so
/// everything after (physics steps in `PreUpdate`, game systems in `Update`)
/// reads this frame's clicks and already sees what the UI swallowed: while
/// the pointer is over a button or panel (or a press that began there is
/// held), every action bound to a mouse button reads idle; while a widget is
/// focused by keys or the pad, actions sharing a control with the navigation,
/// accept, next and prev actions do (`Move` on WASD, `Jump` on Space). Cancel
/// stays visible, it only emits [`UiEvent::Cancel`] and Escape also pauses.
/// The layout and draw list are rebuilt in `PostUpdate`, after the game changed the tree.
#[derive(Default)]
pub struct GuiPlugin;

impl Plugin for GuiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Gui>()
            .add_event::<UiEvent>()
            .add_system(Stage::Startup, bind_actions)
            .add_system(Stage::First, gui_input)
            .add_system(Stage::PostUpdate, gui_layout);
    }
}

fn bind_actions(res: &mut Resources) {
    if let Some(mut map) = res.get_mut::<InputMap>() {
        res.fetch::<Gui>().actions.bind_missing(&mut map);
    }
}

fn gui_input(res: &mut Resources) {
    let mut gui = res.fetch_mut::<Gui>();
    let gui = &mut *gui;
    // Real time: menus must work while the game is paused or slowed.
    let dt = res.get::<Time>().map_or(0.0, |t| t.unscaled_delta());
    let mut input = UiInput::default();
    if let Some(a) = res.get::<ActionState>() {
        let n = &gui.actions;
        if !a.cursor_locked {
            // The cursor is in physical pixels, the layout in UI pixels.
            input.pointer = Some(a.mouse_position / gui.scale.max(1e-3));
        }
        input.nav = gui.repeat.update(a.axis2d(&n.navigate), dt);
        input.next = a.just_pressed(&n.next);
        input.prev = a.just_pressed(&n.prev);
        input.accept_pressed = a.just_pressed(&n.accept);
        input.accept_released = a.just_released(&n.accept);
        input.cancel = a.just_pressed(&n.cancel);
    }
    // A press that is being captured as a new binding is not a click.
    let rebinding = res
        .get::<olvin_input::Rebind>()
        .is_some_and(|r| r.took_input());
    if !rebinding && let Some(i) = res.get::<Input>() {
        input.pointer_pressed = i.mouse_just_pressed(MouseButton::Left);
        input.pointer_released = i.mouse_just_released(MouseButton::Left);
    }
    gui.ui.update(&input);
    swallow_input(res, gui);
    if !gui.ui.events().is_empty() {
        res.fetch_mut::<olvin_core::Events<UiEvent>>()
            .send_batch(gui.ui.events().iter().cloned());
    }
}

/// Makes the actions the UI just used read idle for the game.
fn swallow_input(res: &Resources, gui: &Gui) {
    let (pointer, keys) = (gui.ui.pointer_captured(), gui.ui.keyboard_captured());
    if !pointer && !keys {
        return;
    }
    let (Some(map), Some(mut state)) = (res.get::<InputMap>(), res.get_mut::<ActionState>()) else {
        return;
    };
    let n = &gui.actions;
    let ui_actions = [
        n.navigate.as_str(),
        n.accept.as_str(),
        n.cancel.as_str(),
        n.next.as_str(),
        n.prev.as_str(),
    ];
    let mut names: Vec<&str> = Vec::new();
    if pointer {
        names.extend(map.actions_using(|b| matches!(b, Binding::Mouse(_)), &ui_actions));
    }
    if keys {
        // What the UI's own navigation reads (cancel stays with the game).
        let used: Vec<&Binding> = [&n.navigate, &n.accept, &n.next, &n.prev]
            .into_iter()
            .filter_map(|a| map.actions.get(a))
            .flatten()
            .flat_map(Binding::leaves)
            .filter(|b| !matches!(b, Binding::Mouse(_)))
            .collect();
        names.extend(map.actions_using(|b| used.contains(&b), &ui_actions));
    }
    for name in names {
        state.consume(name);
    }
}

fn gui_layout(res: &mut Resources) {
    let mut gui = res.fetch_mut::<Gui>();
    let gui = &mut *gui;
    let ui_scale = gui.ui_scale.max(0.1);
    gui.scale = ui_scale;
    if let Some(s) = res.get::<SurfaceTarget>()
        && s.size.0 > 0
        && s.size.1 > 0
    {
        gui.scale = s.scale_factor.max(0.1) * ui_scale;
        gui.viewport = Vec2::new(s.size.0 as f32, s.size.1 as f32) / gui.scale;
    }
    gui.ui.layout(gui.viewport, gui.measure.as_ref());
    gui.ui.paint(&gui.theme, &mut gui.draw);
}
