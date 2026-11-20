//! HUD without text: the engine cannot draw text (no fonts, `olvin_gui` is
//! not rendered), so messages go to the log and the screen shows coloured
//! screen-space sprites: a crosshair, a bar while a message is up (its length
//! follows the message), and a dim overlay with a "pause" sign.

use olvin::core::SurfaceTarget;
use olvin::game::State;
use olvin::prelude::*;
use olvin::render::{Space2d, Sprite};

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
        println!("[hud] {text}");
        self.message = Some(text.to_string());
        self.remaining = MESSAGE_SECS;
        self.history.push(text.to_string());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HudPart {
    Crosshair,
    MessageBar,
    PauseOverlay,
    PauseBarLeft,
    PauseBarRight,
}

/// Spawns the HUD sprites (positions are set every frame by [`update`]).
pub fn spawn(world: &mut World) {
    let part = |part, color: Color, layer| {
        (
            Transform::IDENTITY,
            Sprite {
                color,
                layer,
                space: Space2d::Screen,
                ..Default::default()
            },
            part,
        )
    };
    world.spawn(part(HudPart::Crosshair, Color([1.0, 1.0, 1.0, 0.8]), 10));
    world.spawn(part(
        HudPart::MessageBar,
        Color([0.95, 0.75, 0.1, 0.85]),
        10,
    ));
    world.spawn(part(
        HudPart::PauseOverlay,
        Color([0.0, 0.0, 0.0, 0.55]),
        20,
    ));
    world.spawn(part(HudPart::PauseBarLeft, Color([1.0, 1.0, 1.0, 0.9]), 21));
    world.spawn(part(
        HudPart::PauseBarRight,
        Color([1.0, 1.0, 1.0, 0.9]),
        21,
    ));
}

/// Lays out the sprites for the window size and the game state.
pub fn update(res: &mut Resources) {
    let dt = res.fetch::<Time>().unscaled_delta();
    let paused = res
        .get::<State<GameState>>()
        .is_some_and(|s| s.is(GameState::Paused));
    let message_len = {
        let mut hud = res.fetch_mut::<Hud>();
        if hud.message.is_some() {
            hud.remaining -= dt;
            if hud.remaining <= 0.0 {
                hud.message = None;
            }
        }
        hud.message.as_ref().map(|m| m.chars().count())
    };
    let (w, h) = res
        .get::<SurfaceTarget>()
        .map_or((1280.0, 720.0), |s| (s.size.0 as f32, s.size.1 as f32));
    let center = Vec2::new(w, h) * 0.5;
    let mut world = res.fetch_mut::<World>();
    for (t, sprite, part) in world.query_mut::<(&mut Transform, &mut Sprite, &HudPart)>() {
        let (pos, size, visible) = match part {
            HudPart::Crosshair => (center, Vec2::splat(4.0), !paused),
            HudPart::MessageBar => {
                let len = message_len.unwrap_or(0) as f32;
                (
                    Vec2::new(center.x, 40.0),
                    Vec2::new((len * 9.0).clamp(80.0, w * 0.9), 14.0),
                    message_len.is_some(),
                )
            }
            HudPart::PauseOverlay => (center, Vec2::new(w, h), paused),
            HudPart::PauseBarLeft => (center - Vec2::X * 18.0, Vec2::new(20.0, 70.0), paused),
            HudPart::PauseBarRight => (center + Vec2::X * 18.0, Vec2::new(20.0, 70.0), paused),
        };
        t.translation = pos.extend(0.0);
        // No "hidden" flag on sprites: zero size hides one.
        sprite.size = if visible { size } else { Vec2::ZERO };
    }
}
