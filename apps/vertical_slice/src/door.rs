//! The pressure plate and the door are Lua now (`scripts/plate.lua`,
//! `scripts/door.lua`); this is the Rust side they talk to — the HUD and the
//! saved progress — plus crate impact sounds.

use olvin::core::{EventCursor, Events};
use olvin::prelude::*;
use olvin::script::{ScriptMessage, ScriptVars};

use crate::hud::Hud;
use crate::save::Progress;

/// Shared variable the plate sets and the door follows.
pub const DOOR_OPEN: &str = "door_open";

/// Read position of [`bridge`] in the script message queue.
#[derive(Default)]
pub struct ScriptInbox(EventCursor);

/// Script → game: `hud` messages go on screen, `door_open` into [`Progress`]
/// (which the save slots keep).
pub fn bridge(res: &mut Resources) {
    let texts: Vec<String> = {
        let events = res.fetch::<Events<ScriptMessage>>();
        let mut inbox = res.fetch_mut::<ScriptInbox>();
        events
            .read(&mut inbox.0)
            .filter(|m| m.name == "hud")
            .filter_map(|m| m.value.as_str().map(str::to_owned))
            .collect()
    };
    if !texts.is_empty() {
        let mut hud = res.fetch_mut::<Hud>();
        for text in &texts {
            hud.show(text);
        }
    }
    sync_progress(res);
}

/// Copies the scripts' door flag into [`Progress`].
pub fn sync_progress(res: &Resources) {
    let open = res.fetch::<ScriptVars>().flag(DOOR_OPEN);
    res.fetch_mut::<Progress>().door_opened = open;
}

/// After a load: the restored [`Progress`] tells the scripts whether the door is open.
pub fn restore_vars(res: &Resources) {
    let open = res.fetch::<Progress>().door_opened;
    res.fetch_mut::<ScriptVars>().set(DOOR_OPEN, open);
}

/// Crates thump when they hit something hard enough.
pub struct Thud {
    last: f64,
}

impl FromParams for Thud {
    fn from_params(_: &Params) -> Self {
        Self { last: -1.0 }
    }
}

impl Behaviour for Thud {
    fn on_collision_enter(&mut self, ctx: &mut ScriptCtx, _other: Entity) {
        let speed = ctx.velocity().map_or(0.0, |v| v.length());
        let now = ctx.elapsed();
        // Skip the settling contacts of the first second and tiny bumps.
        if now < 1.0 || speed < 1.2 || now - self.last < 0.15 {
            return;
        }
        self.last = now;
        let at = ctx.world_position();
        if let Some(mut audio) = ctx.res_mut::<Audio>() {
            audio.play_with(
                "sfx/thud.wav",
                PlayParams {
                    volume: (speed / 5.0).clamp(0.2, 1.0),
                    pitch: 0.9 + (speed * 0.37).fract() * 0.2,
                    max_distance: 25.0,
                    ..PlayParams::at(at)
                },
            );
        }
    }
}
