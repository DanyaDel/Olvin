use std::any::Any;

use olvin_ecs::Entity;

use crate::ScriptCtx;

/// A Unity-style script. Every method is optional.
///
/// Order within a frame: `start` (once, before the script's first other
/// callback), `fixed_update` before each physics step (PreUpdate), then in
/// `Stage::Update`: collision/trigger callbacks for the frame's physics events,
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
}
