//! What a behaviour sees: its entity, the world, time, input, physics.

use std::cell::{Ref, RefMut};
use std::fmt::Display;

use glam::{EulerRot, Quat, Vec3};
use hecs::DynamicBundle;
use olvin_core::Resources;
use olvin_ecs::{Component, Entity, World};
use olvin_scene::{Name, Tag, Transform};
use olvin_transform::world_matrix;

use crate::{Behaviour, ScriptMessage, ScriptValue, ScriptVars};

/// Frame timing as seen by the current callback.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TimeInfo {
    /// Seconds this callback covers: frame delta in `update`, the fixed step in `fixed_update`.
    pub dt: f32,
    pub elapsed: f64,
    pub fixed_dt: f32,
    pub frame: u64,
}

/// Structural changes requested by scripts, applied after the pass.
#[derive(Default)]
pub(crate) struct Commands {
    pub(crate) buffer: olvin_ecs::CommandBuffer,
    pub(crate) despawn: Vec<Entity>,
    pub(crate) attach: Vec<(Entity, String, Box<dyn Behaviour>)>,
}

/// The script's handle on the engine for one callback.
///
/// Components can be read and written right away (`get`, `get_mut`,
/// `world_mut`). Spawning, despawning, adding/removing components and
/// attaching scripts are deferred until every script of the pass has run, so
/// no script sees a half-changed world. `on_destroy` runs before the entity is
/// actually removed.
pub struct ScriptCtx<'a> {
    pub(crate) entity: Entity,
    pub(crate) world: &'a mut World,
    pub(crate) res: &'a Resources,
    pub(crate) commands: &'a mut Commands,
    pub(crate) time: TimeInfo,
    pub(crate) script: &'a str,
    #[cfg(feature = "input")]
    pub(crate) input: Option<&'a mut olvin_input::ActionState>,
}

impl<'a> ScriptCtx<'a> {
    /// The entity this script is attached to.
    pub fn entity(&self) -> Entity {
        self.entity
    }

    /// Name of the running script (as registered).
    pub fn script_name(&self) -> &str {
        self.script
    }

    /// Whether the entity still exists (false in a late `on_destroy`).
    pub fn alive(&self) -> bool {
        self.world.contains(self.entity)
    }

    // --- World -----------------------------------------------------------

    pub fn world(&self) -> &World {
        self.world
    }

    pub fn world_mut(&mut self) -> &mut World {
        self.world
    }

    pub fn get<T: Component>(&self, entity: Entity) -> Option<hecs::Ref<'_, T>> {
        self.world.get::<&T>(entity).ok()
    }

    pub fn get_mut<T: Component>(&self, entity: Entity) -> Option<hecs::RefMut<'_, T>> {
        self.world.get::<&mut T>(entity).ok()
    }

    /// A component of this script's own entity.
    pub fn component<T: Component>(&self) -> Option<hecs::Ref<'_, T>> {
        self.get::<T>(self.entity)
    }

    pub fn component_mut<T: Component>(&self) -> Option<hecs::RefMut<'_, T>> {
        self.get_mut::<T>(self.entity)
    }

    pub fn has<T: Component>(&self, entity: Entity) -> bool {
        self.world.satisfies::<&T>(entity)
    }

    /// Spawns at the end of the pass; the id is valid right away (e.g. to
    /// `attach` a script to it or remember it).
    pub fn spawn(&mut self, components: impl DynamicBundle) -> Entity {
        let e = self.world.reserve_entity();
        self.commands.buffer.insert(e, components);
        e
    }

    /// Despawns (with its children) at the end of the pass, after `on_destroy`.
    pub fn despawn(&mut self, entity: Entity) {
        self.commands.despawn.push(entity);
    }

    /// Despawns this script's entity (Unity's `Destroy(gameObject)`).
    pub fn destroy_self(&mut self) {
        self.despawn(self.entity);
    }

    pub fn insert(&mut self, entity: Entity, components: impl DynamicBundle) {
        self.commands.buffer.insert(entity, components);
    }

    pub fn remove<T: Component>(&mut self, entity: Entity) {
        self.commands.buffer.remove_one::<T>(entity);
    }

    /// Attaches a script instance made in code; its `start` runs next pass.
    pub fn attach(&mut self, entity: Entity, behaviour: impl Behaviour) {
        let name = std::any::type_name_of_val(&behaviour)
            .rsplit("::")
            .next()
            .unwrap_or("script")
            .to_owned();
        self.commands
            .attach
            .push((entity, name, Box::new(behaviour)));
    }

    pub fn find_by_name(&self, name: &str) -> Option<Entity> {
        crate::params::find_by_name(self.world, name)
    }

    /// First entity with this tag.
    pub fn find_by_tag(&self, tag: &str) -> Option<Entity> {
        self.find_all_by_tag(tag).into_iter().next()
    }

    pub fn find_all_by_tag(&self, tag: &str) -> Vec<Entity> {
        self.world
            .query::<(Entity, &Tag)>()
            .iter()
            .filter(|(_, t)| t.0 == tag)
            .map(|(e, _)| e)
            .collect()
    }

    /// A direct child of this entity with this name.
    pub fn find_child(&self, name: &str) -> Option<Entity> {
        self.world
            .query::<(Entity, &olvin_transform::Parent, &Name)>()
            .iter()
            .find(|(_, p, n)| p.0 == self.entity && n.0 == name)
            .map(|(e, _, _)| e)
    }

    pub fn name(&self) -> String {
        self.name_of(self.entity)
    }

    pub fn name_of(&self, entity: Entity) -> String {
        self.get::<Name>(entity)
            .map(|n| n.0.clone())
            .unwrap_or_else(|| format!("{entity:?}"))
    }

    /// Any other engine resource. Don't ask for `World` or `ScriptRuntime`
    /// (already borrowed by the script pass: that panics).
    pub fn res<T: 'static>(&self) -> Option<Ref<'_, T>> {
        self.res.get::<T>()
    }

    pub fn res_mut<T: 'static>(&self) -> Option<RefMut<'_, T>> {
        self.res.get_mut::<T>()
    }

    // --- Messages and shared variables -----------------------------------

    /// Sends a [`ScriptMessage`] from this entity: to `target`, or to every
    /// behaviour with `None`. Delivered (`on_message`) in the next `update`
    /// pass; game systems can read `Events<ScriptMessage>` too. False when
    /// there is no queue (no `ScriptPlugin`).
    pub fn send_message(
        &self,
        name: &str,
        value: impl Into<ScriptValue>,
        target: Option<Entity>,
    ) -> bool {
        let Some(mut events) = self.res.get_mut::<olvin_core::Events<ScriptMessage>>() else {
            return false;
        };
        events.send(ScriptMessage {
            name: name.to_owned(),
            value: value.into(),
            from: Some(self.entity),
            target,
        });
        true
    }

    /// Calls `on_message(name)` on this object after `seconds` of game time
    /// (it stops while the game is paused). False when the object already has
    /// [`MAX_TIMERS_PER_OBJECT`](crate::MAX_TIMERS_PER_OBJECT) timers.
    pub fn after(&self, seconds: f32, name: &str) -> bool {
        crate::timer::start(self.res, self.entity, seconds, name, false)
    }

    /// Like [`ScriptCtx::after`], again and again every `seconds`.
    pub fn every(&self, seconds: f32, name: &str) -> bool {
        crate::timer::start(self.res, self.entity, seconds, name, true)
    }

    /// Stops this object's timers called `name`; returns how many.
    pub fn cancel_timer(&self, name: &str) -> usize {
        crate::timer::cancel(self.res, self.entity, name)
    }

    /// The shared [`ScriptVars`], if the plugin added them.
    pub fn vars(&self) -> Option<Ref<'_, ScriptVars>> {
        self.res.get::<ScriptVars>()
    }

    pub fn vars_mut(&self) -> Option<RefMut<'_, ScriptVars>> {
        self.res.get_mut::<ScriptVars>()
    }

    // --- Time ------------------------------------------------------------

    pub fn time(&self) -> TimeInfo {
        self.time
    }

    /// Seconds covered by this callback (frame delta or fixed step).
    pub fn dt(&self) -> f32 {
        self.time.dt
    }

    pub fn elapsed(&self) -> f64 {
        self.time.elapsed
    }

    pub fn fixed_dt(&self) -> f32 {
        self.time.fixed_dt
    }

    // --- Transform of this entity ----------------------------------------

    pub fn transform(&self) -> Transform {
        self.transform_of(self.entity)
    }

    pub fn transform_of(&self, entity: Entity) -> Transform {
        self.get::<Transform>(entity)
            .map(|t| *t)
            .unwrap_or_default()
    }

    pub fn set_transform(&mut self, transform: Transform) {
        self.with_transform(|t| *t = transform);
    }

    /// Edits this entity's `Transform` (adds one if missing).
    pub fn with_transform<R>(&mut self, f: impl FnOnce(&mut Transform) -> R) -> R {
        if let Ok(mut t) = self.world.get::<&mut Transform>(self.entity) {
            return f(&mut t);
        }
        let mut t = Transform::IDENTITY;
        let r = f(&mut t);
        let _ = self.world.insert_one(self.entity, t);
        r
    }

    /// Local position (relative to the parent, if any).
    pub fn position(&self) -> Vec3 {
        self.transform().translation
    }

    pub fn set_position(&mut self, position: Vec3) {
        self.with_transform(|t| t.translation = position);
    }

    /// Position in the world right now (parents included, no need to wait for propagation).
    pub fn world_position(&self) -> Vec3 {
        self.world_position_of(self.entity)
    }

    pub fn world_position_of(&self, entity: Entity) -> Vec3 {
        world_matrix(self.world, entity)
            .map(|m| m.w_axis.truncate())
            .unwrap_or(Vec3::ZERO)
    }

    pub fn rotation(&self) -> Quat {
        self.transform().rotation
    }

    pub fn set_rotation(&mut self, rotation: Quat) {
        self.with_transform(|t| t.rotation = rotation);
    }

    /// Moves along the parent's axes.
    pub fn translate(&mut self, delta: Vec3) {
        self.with_transform(|t| t.translation += delta);
    }

    /// Moves along the entity's own axes (Unity's `Translate` in `Space.Self`).
    pub fn translate_local(&mut self, delta: Vec3) {
        self.with_transform(|t| t.translation += t.rotation * delta);
    }

    /// Rotates around the entity's own axes.
    pub fn rotate(&mut self, rotation: Quat) {
        self.with_transform(|t| t.rotation = (t.rotation * rotation).normalize());
    }

    /// Rotates by Euler degrees (Y, then X, then Z, like the editor).
    pub fn rotate_euler_deg(&mut self, degrees: Vec3) {
        let r = degrees * std::f32::consts::PI / 180.0;
        self.rotate(Quat::from_euler(EulerRot::YXZ, r.y, r.x, r.z));
    }

    /// Turns so `forward()` points at `target` (in the parent's space).
    pub fn look_at(&mut self, target: Vec3, up: Vec3) {
        self.with_transform(|t| {
            if (target - t.translation).length_squared() > 1e-10 {
                t.look_to(target - t.translation, up);
            }
        });
    }

    pub fn forward(&self) -> Vec3 {
        self.rotation() * Vec3::NEG_Z
    }

    pub fn right(&self) -> Vec3 {
        self.rotation() * Vec3::X
    }

    pub fn up(&self) -> Vec3 {
        self.rotation() * Vec3::Y
    }

    // --- Logging ---------------------------------------------------------

    pub fn log(&self, message: impl Display) {
        log::info!(target: "script", "[{} / {}] {message}", self.name(), self.script);
    }

    pub fn warn(&self, message: impl Display) {
        log::warn!(target: "script", "[{} / {}] {message}", self.name(), self.script);
    }

    // --- Input -----------------------------------------------------------

    /// Actions of the current frame. Idle (all false / zero) without the input plugin.
    #[cfg(feature = "input")]
    pub fn input(&self) -> &olvin_input::ActionState {
        static IDLE: std::sync::OnceLock<olvin_input::ActionState> = std::sync::OnceLock::new();
        match &self.input {
            Some(s) => s,
            None => IDLE.get_or_init(Default::default),
        }
    }

    /// For cursor lock / visibility requests.
    #[cfg(feature = "input")]
    pub fn input_mut(&mut self) -> Option<&mut olvin_input::ActionState> {
        self.input.as_deref_mut()
    }

    /// Raw keys and mouse (`olvin_window::Input`), when you really need a key by code.
    #[cfg(feature = "input")]
    pub fn raw_input(&self) -> Option<Ref<'_, olvin_input::Input>> {
        self.res.get::<olvin_input::Input>()
    }

    // --- Physics ---------------------------------------------------------

    #[cfg(feature = "physics")]
    pub fn physics(&self) -> Option<Ref<'_, olvin_physics::Physics>> {
        self.res.get::<olvin_physics::Physics>()
    }

    /// First solid hit along a ray, ignoring this entity's own body.
    #[cfg(feature = "physics")]
    pub fn raycast(&self, origin: Vec3, direction: Vec3, max: f32) -> Option<olvin_physics::Hit> {
        self.physics()?
            .raycast_filtered(origin, direction, max, Some(self.entity), false)
    }

    /// Pushes this entity's dynamic body with a force (N) for the next physics
    /// step: call it from `fixed_update` every step. False without a dynamic body.
    #[cfg(feature = "physics")]
    pub fn apply_force(&mut self, force: Vec3) -> bool {
        match self.res.get_mut::<olvin_physics::Physics>() {
            Some(mut p) => p.apply_force(self.world, self.entity, force),
            None => false,
        }
    }

    /// Instant kick (N*s) to this entity's dynamic body: velocity changes by
    /// `impulse / mass`. False without a dynamic body.
    #[cfg(feature = "physics")]
    pub fn apply_impulse(&mut self, impulse: Vec3) -> bool {
        match self.res.get_mut::<olvin_physics::Physics>() {
            Some(mut p) => p.apply_impulse(self.world, self.entity, impulse),
            None => false,
        }
    }

    /// Sets this entity's dynamic body velocity (m/s). Characters use `motor()`.
    #[cfg(feature = "physics")]
    pub fn set_velocity(&mut self, velocity: Vec3) -> bool {
        match self.res.get_mut::<olvin_physics::Physics>() {
            Some(mut p) => p.set_velocity(self.world, self.entity, velocity),
            None => false,
        }
    }

    /// Linear velocity of this entity's dynamic body.
    #[cfg(feature = "physics")]
    pub fn velocity(&self) -> Option<Vec3> {
        self.physics()?.velocity(self.world, self.entity)
    }

    /// This entity's character motor (set `velocity`, read `grounded`).
    #[cfg(feature = "physics")]
    pub fn motor(&self) -> Option<hecs::RefMut<'_, olvin_physics::CharacterMotor>> {
        self.component_mut::<olvin_physics::CharacterMotor>()
    }

    /// `CharacterController.Move`: moves this entity's character now, with collisions.
    #[cfg(feature = "physics")]
    pub fn move_character(&mut self, desired: Vec3) -> Option<olvin_physics::CharacterMove> {
        let mut physics = self.res.get_mut::<olvin_physics::Physics>()?;
        olvin_physics::move_character(&mut physics, self.world, self.entity, desired)
    }
}
