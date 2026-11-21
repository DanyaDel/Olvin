use std::any::Any;

use olvin_ecs::Entity;

use crate::{ScriptCtx, ScriptMessage};

/// A Unity-style script. Every method is optional.
///
/// Order within a frame: `start` (once, before the script's first other
/// callback), `fixed_update` before each physics step (PreUpdate), then in
/// `Stage::Update`: messages sent since the previous pass (`on_message`),
/// collision/trigger callbacks for the frame's physics events,
/// then `update`. `on_destroy` runs when the entity is despawned through
/// [`ScriptCtx::despawn`] (entity still alive) or when it disappeared any
/// other way (entity already gone, `ctx.alive()` is false).
#[allow(unused_variables)]
pub trait Behaviour: Any {
    fn start(&mut self, ctx: &mut ScriptCtx<'_>) {}
    fn update(&mut self, ctx: &mut ScriptCtx<'_>) {}
    fn fixed_update(&mut self, ctx: &mut ScriptCtx<'_>) {}
    fn on_collision_enter(&mut self, ctx: &mut ScriptCtx<'_>, other: Entity) {}
    fn on_collision_exit(&mut self, ctx: &mut ScriptCtx<'_>, other: Entity) {}
    fn on_trigger_enter(&mut self, ctx: &mut ScriptCtx<'_>, other: Entity) {}
    fn on_trigger_exit(&mut self, ctx: &mut ScriptCtx<'_>, other: Entity) {}
    fn on_destroy(&mut self, ctx: &mut ScriptCtx<'_>) {}

    /// State to keep in save games (any text, usually RON: [`crate::state_to_text`]).
    /// `None`: nothing to save (the default), the script restarts from its params.
    /// Lua behaviours get the same hook by implementing these two methods for
    /// their `self` table.
    fn save_state(&self) -> Option<String> {
        None
    }

    /// Restores what [`Behaviour::save_state`] returned. Runs outside of the
    /// script passes; for an instance not attached yet (a freshly loaded
    /// level) it runs right after creation, before `start`.
    fn load_state(&mut self, state: &str) -> Result<(), String> {
        Ok(())
    }

    /// A [`ScriptMessage`] addressed to this entity or broadcast (sent by
    /// [`ScriptCtx::send_message`], a Lua `ctx:send`, or the game through
    /// `Events<ScriptMessage>`). Delivered once, in the next `update` pass.
    fn on_message(&mut self, ctx: &mut ScriptCtx<'_>, message: &ScriptMessage) {}
}
