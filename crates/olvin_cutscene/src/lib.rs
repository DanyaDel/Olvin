//! Cutscenes: a timeline that moves objects, switches the camera, plays
//! sounds and sends messages to scripts.
//!
//! A cutscene is a RON file (`*.cutscene.ron`, usually in `assets/cutscenes/`):
//!
//! ```ron
//! (
//!     name: "intro",
//!     duration: 6.0,
//!     tracks: [
//!         (target: "Shot Camera", keys: [
//!             (time: 0.0, position: Some((0.0, 2.0, 8.0)), ease: Smooth),
//!             (time: 4.0, position: Some((0.0, 2.0, 3.0)), rotation: Some((-10.0, 0.0, 0.0))),
//!         ]),
//!     ],
//!     cues: [
//!         (time: 0.0, action: Camera("Shot Camera")),
//!         (time: 2.0, action: Message(name: "open_door", target: Some("Door"))),
//!         (time: 3.0, action: Sound(path: "sfx/boom.ogg", volume: 1.0)),
//!     ],
//! )
//! ```
//!
//! *Tracks* move objects found by name (position, rotation as Euler degrees,
//! scale; each channel is interpolated between the keys that set it). *Cues*
//! happen once at their time. Scripts start and stop cutscenes with messages,
//! so Lua needs no extra API: `ctx:send("cutscene:play", "cutscenes/intro.cutscene.ron")`,
//! `ctx:send("cutscene:skip")`, `ctx:send("cutscene:stop")`; when one ends the
//! game gets the event [`CutsceneFinished`] and every script the message
//! `cutscene_finished` with the cutscene's name.

mod data;
mod player;

pub use data::{Cue, CueAction, CueValue, Cutscene, CutsceneError, Ease, Key, TransformTrack};
pub use player::{CutsceneFinished, CutsceneId, CutscenePlugin, Cutscenes};
