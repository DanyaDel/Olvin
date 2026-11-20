//! The pressure plate and the door it opens, plus crate impact sounds.

use olvin::prelude::*;
use serde::{Deserialize, Serialize};

use crate::hud::Hud;
use crate::player::Player;
use crate::save::Progress;

/// A sliding door. The scene has no way to attach game components, so the
/// game adds this after loading (see `level::setup`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DoorState {
    pub open: bool,
    /// Height of the door's centre when closed.
    pub closed_y: f32,
    /// How far up it slides when open.
    pub lift: f32,
}

pub const DOOR_SPEED: f32 = 1.5;

/// Slides doors toward their target height (simulation time: stops on pause).
/// Doors are kinematic bodies, so the collider follows the `Transform`.
pub fn animate(res: &mut Resources) {
    let dt = res.fetch::<Time>().delta();
    let mut world = res.fetch_mut::<World>();
    for (t, door) in world.query_mut::<(&mut Transform, &DoorState)>() {
        let target = door.closed_y + if door.open { door.lift } else { 0.0 };
        let dy = target - t.translation.y;
        let step = DOOR_SPEED * dt;
        t.translation.y += dy.clamp(-step, step);
    }
}

/// Behaviour on the plate's trigger: the player stepping in opens the door
/// named by `door`, plays the door sound there and shows `message`.
pub struct DoorSwitch {
    door: String,
    message: String,
}

impl FromParams for DoorSwitch {
    fn from_params(p: &Params) -> Self {
        Self {
            door: p.string("door", "Door"),
            message: p.string("message", "The door is open"),
        }
    }

    fn schema() -> Vec<ParamSpec> {
        vec![
            ParamSpec::string("door", "Door", "name of the door entity"),
            ParamSpec::string("message", "The door is open", "shown when it opens"),
        ]
    }
}

impl Behaviour for DoorSwitch {
    fn on_trigger_enter(&mut self, ctx: &mut ScriptCtx, other: Entity) {
        if !ctx.has::<Player>(other) {
            return;
        }
        let Some(door) = ctx.find_by_name(&self.door) else {
            ctx.warn(format!("no door named `{}`", self.door));
            return;
        };
        let opened = match ctx.get_mut::<DoorState>(door) {
            Some(mut d) if !d.open => {
                d.open = true;
                true
            }
            _ => false,
        };
        if !opened {
            return;
        }
        let at = ctx.world_position_of(door);
        if let Some(mut audio) = ctx.res_mut::<Audio>() {
            audio.play_with(
                "sfx/door.wav",
                PlayParams {
                    volume: 0.9,
                    max_distance: 30.0,
                    ..PlayParams::at(at)
                },
            );
        }
        if let Some(mut hud) = ctx.res_mut::<Hud>() {
            hud.show(&self.message);
        }
        if let Some(mut progress) = ctx.res_mut::<Progress>() {
            progress.door_opened = true;
        }
    }
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
