//! HUD on `olvin_gui`: a crosshair, a toast for messages (controls, the door,
//! saves), "loading" while the level is parsed and a pause menu with buttons.
//! The tree is built once; [`update`] only shows, hides and retitles nodes.
//! Without a renderer (tests) the same tree is laid out, just not drawn.

use olvin::core::{EventCursor, Events};
use olvin::game::State;
use olvin::gui::{Align, Color, Node, NodeId, UiEvent};
use olvin::prelude::*;

use crate::GameState;

/// How long a message stays up, seconds of real time.
pub const MESSAGE_SECS: f32 = 4.0;

#[derive(Debug, Default)]
pub struct Hud {
    /// The message on screen now.
    pub message: Option<String>,
    remaining: f32,
    /// Every message shown, oldest first (tests read it).
    pub history: Vec<String>,
}

impl Hud {
    pub fn show(&mut self, text: &str) {
        log::info!("HUD: {text}");
        self.message = Some(text.to_string());
        self.remaining = MESSAGE_SECS;
        self.history.push(text.to_string());
    }
}

/// Nodes [`update`] touches.
pub struct HudNodes {
    pub crosshair: NodeId,
    pub toast: NodeId,
    pub toast_text: NodeId,
    pub loading: NodeId,
    pub pause: NodeId,
    events: EventCursor,
}

const PLATE: Color = Color::rgba(0.05, 0.06, 0.08, 0.78);
const ACCENT: Color = Color::rgb(0.98, 0.8, 0.25);

/// `Stage::Startup`: builds the tree in `Gui`.
pub fn build(res: &mut Resources) {
    let mut gui = res.fetch_mut::<Gui>();
    let ui = &mut gui.ui;
    let root = ui.root();
    let crosshair = ui.add(
        root,
        Node::panel()
            .px(4.0, 4.0)
            .background(Color::rgba(1.0, 1.0, 1.0, 0.85))
            .centered(),
    );
    let toast = ui.add(
        root,
        Node::panel()
            .padding(12.0)
            .background(PLATE)
            .anchor(
                Vec2::new(0.5, 0.0),
                Vec2::new(0.5, 0.0),
                Vec2::new(0.0, 24.0),
            )
            .hidden(),
    );
    let toast_text = ui.add(toast, Node::label("").text_size(20.0));
    let loading = ui.add(
        root,
        Node::label("Загрузка…")
            .text_size(28.0)
            .padding(16.0)
            .background(PLATE)
            .centered(),
    );
    let pause = ui.add(
        root,
        Node::panel()
            .fill()
            .background(Color::rgba(0.0, 0.0, 0.0, 0.55))
            .hidden(),
    );
    let menu = ui.add(
        pause,
        Node::panel()
            .column()
            .gap(12.0)
            .padding(24.0)
            .align(Align::Stretch)
            .background(PLATE)
            .centered(),
    );
    ui.add(menu, Node::label("Пауза").text_size(40.0).color(ACCENT));
    ui.add(menu, Node::button("Продолжить").name("resume"));
    ui.add(menu, Node::button("Выйти").name("quit"));
    ui.add(
        menu,
        Node::label("Esc — продолжить, Q — выйти\nF5 — сохранить, F9 — загрузить")
            .text_size(15.0)
            .color(Color::rgb(0.7, 0.72, 0.76)),
    );
    drop(gui);
    res.insert(HudNodes {
        crosshair,
        toast,
        toast_text,
        loading,
        pause,
        events: EventCursor::default(),
    });
}

/// Shows what fits the state, counts the message down, handles menu clicks.
pub fn update(res: &mut Resources) {
    let dt = res.fetch::<Time>().unscaled_delta();
    let state = res.fetch::<State<GameState>>().get();
    let message = {
        let mut hud = res.fetch_mut::<Hud>();
        if hud.message.is_some() {
            hud.remaining -= dt;
            if hud.remaining <= 0.0 {
                hud.message = None;
            }
        }
        hud.message.clone()
    };
    let Some(mut nodes) = res.get_mut::<HudNodes>() else {
        return;
    };
    let mut clicked = Vec::new();
    for e in res.fetch::<Events<UiEvent>>().read(&mut nodes.events) {
        if let UiEvent::Click { name: Some(n), .. } = e {
            clicked.push(n.clone());
        }
    }
    {
        let mut gui = res.fetch_mut::<Gui>();
        let ui = &mut gui.ui;
        let paused = state == GameState::Paused;
        let was_paused = ui.node(nodes.pause).is_some_and(|n| n.visible);
        ui.set_visible(nodes.crosshair, state == GameState::Playing);
        ui.set_visible(nodes.loading, state == GameState::Loading);
        ui.set_visible(nodes.pause, paused);
        if paused && !was_paused {
            ui.focus_first();
        }
        ui.set_visible(nodes.toast, message.is_some());
        if let Some(m) = &message {
            ui.set_text(nodes.toast_text, m);
        }
    }
    drop(nodes);
    for name in clicked {
        match name.as_str() {
            "resume" => res.fetch_mut::<State<GameState>>().set(GameState::Playing),
            "quit" => res.fetch_mut::<AppExit>().requested = true,
            _ => {}
        }
    }
}
