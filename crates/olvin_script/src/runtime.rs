//! Script instances and the passes that call them.

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};

use olvin_core::{Resources, Time};
use olvin_ecs::{Entity, World};
use olvin_transform::Parent;

use crate::ctx::{Commands, TimeInfo};
use crate::{Behaviour, Params, ScriptCtx, ScriptRegistry, Scripts};

/// Marks an entity whose `Scripts` were turned into instances.
#[derive(Clone, Copy, Debug, Default)]
pub struct ScriptsAttached;

struct Slot {
    name: String,
    behaviour: Box<dyn Behaviour>,
    started: bool,
    /// Panicked once: no more callbacks.
    failed: bool,
}

struct Entry {
    entity: Entity,
    slots: Vec<Slot>,
}

/// Saved states of one entity's scripts: `(script name, state)` in attach
/// order, only for scripts whose [`Behaviour::save_state`] returned something.
pub type ScriptState = Vec<(String, String)>;

/// Live script instances, by entity. A resource (instances are not ECS
/// components because hecs needs `Send + Sync` and scripts don't).
#[derive(Default)]
pub struct ScriptRuntime {
    entries: Vec<Entry>,
    index: HashMap<Entity, usize>,
    pending: Vec<(Entity, String, Box<dyn Behaviour>)>,
    /// States restored before the entity's scripts were attached.
    pending_states: HashMap<Entity, ScriptState>,
    warned: HashSet<String>,
}

impl ScriptRuntime {
    /// Distinct unknown script names warned about so far.
    pub fn warned_count(&self) -> usize {
        self.warned.len()
    }

    /// Attaches an instance made in code; `start` runs at the next pass.
    pub fn attach(&mut self, entity: Entity, name: &str, behaviour: impl Behaviour) {
        self.pending
            .push((entity, name.to_owned(), Box::new(behaviour)));
    }

    /// The first script of type `T` on `entity` (outside of script passes).
    pub fn get<T: Behaviour>(&self, entity: Entity) -> Option<&T> {
        let entry = &self.entries[*self.index.get(&entity)?];
        entry
            .slots
            .iter()
            .find_map(|s| (s.behaviour.as_ref() as &dyn Any).downcast_ref::<T>())
    }

    pub fn get_mut<T: Behaviour>(&mut self, entity: Entity) -> Option<&mut T> {
        let i = *self.index.get(&entity)?;
        self.entries[i]
            .slots
            .iter_mut()
            .find_map(|s| (s.behaviour.as_mut() as &mut dyn Any).downcast_mut::<T>())
    }

    /// Names of the scripts running on an entity.
    pub fn scripts_of(&self, entity: Entity) -> Vec<&str> {
        self.index
            .get(&entity)
            .map(|&i| {
                self.entries[i]
                    .slots
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Entities with at least one script.
    pub fn entity_count(&self) -> usize {
        self.entries.len()
    }

    /// What the entity's scripts want saved ([`Behaviour::save_state`]). A
    /// state handed to [`ScriptRuntime::load_states`] and not applied yet is
    /// returned as is.
    pub fn save_states(&self, entity: Entity) -> ScriptState {
        if let Some(states) = self.pending_states.get(&entity) {
            return states.clone();
        }
        let Some(&i) = self.index.get(&entity) else {
            return Vec::new();
        };
        self.entries[i]
            .slots
            .iter()
            .filter_map(|s| Some((s.name.clone(), s.behaviour.save_state()?)))
            .collect()
    }

    /// Gives saved states back to the entity's scripts: the k-th state saved
    /// for a script name goes to the k-th script of that name. Scripts not
    /// attached yet get it right after they are created, before `start`.
    pub fn load_states(&mut self, entity: Entity, states: ScriptState) {
        match self.index.get(&entity) {
            Some(&i) => apply_states(entity, &mut self.entries[i].slots, &states),
            None => {
                self.pending_states.insert(entity, states);
            }
        }
    }

    fn add(&mut self, entity: Entity, name: String, behaviour: Box<dyn Behaviour>) {
        let slot = Slot {
            name,
            behaviour,
            started: false,
            failed: false,
        };
        match self.index.get(&entity) {
            Some(&i) => self.entries[i].slots.push(slot),
            None => {
                self.index.insert(entity, self.entries.len());
                self.entries.push(Entry {
                    entity,
                    slots: vec![slot],
                });
            }
        }
    }

    fn remove(&mut self, entity: Entity) -> Option<Entry> {
        let i = self.index.remove(&entity)?;
        let entry = self.entries.remove(i);
        for (j, e) in self.entries.iter().enumerate().skip(i) {
            self.index.insert(e.entity, j);
        }
        Some(entry)
    }
}

fn apply_states(entity: Entity, slots: &mut [Slot], states: &ScriptState) {
    for (k, (name, state)) in states.iter().enumerate() {
        let nth = states[..k].iter().filter(|(n, _)| n == name).count();
        let Some(slot) = slots.iter_mut().filter(|s| &s.name == name).nth(nth) else {
            log::warn!("saved state for missing script `{name}` on {entity:?} skipped");
            continue;
        };
        if let Err(e) = slot.behaviour.load_state(state) {
            log::warn!("script `{name}` on {entity:?}: state not loaded: {e}");
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Phase {
    Update,
    Fixed,
}

/// What a callback needs, borrowed for the whole pass.
struct Env<'a> {
    world: &'a mut World,
    res: &'a Resources,
    commands: Commands,
    time: TimeInfo,
    #[cfg(feature = "input")]
    input: Option<std::cell::RefMut<'a, olvin_input::ActionState>>,
}

impl Env<'_> {
    /// Calls `f` on one script with a fresh context; a panic disables that script.
    fn call(
        &mut self,
        entity: Entity,
        slot: &mut Slot,
        what: &str,
        f: impl FnOnce(&mut dyn Behaviour, &mut ScriptCtx<'_>),
    ) {
        if slot.failed {
            return;
        }
        let mut ctx = ScriptCtx {
            entity,
            world: &mut *self.world,
            res: self.res,
            commands: &mut self.commands,
            time: self.time,
            script: &slot.name,
            #[cfg(feature = "input")]
            input: self.input.as_deref_mut(),
        };
        let behaviour = slot.behaviour.as_mut();
        if catch_unwind(AssertUnwindSafe(|| f(behaviour, &mut ctx))).is_err() {
            log::error!(
                "script `{}` on {entity:?} panicked in {what}; it is disabled",
                slot.name
            );
            slot.failed = true;
        }
    }
}

fn time_info(res: &Resources, phase: Phase, dt: Option<f32>) -> TimeInfo {
    #[cfg(feature = "physics")]
    let fixed_dt = olvin_physics::FIXED_DT;
    #[cfg(not(feature = "physics"))]
    let fixed_dt = 1.0 / 60.0;
    let (delta, elapsed, frame) = res
        .get::<Time>()
        .map_or((0.0, 0.0, 0), |t| (t.delta(), t.elapsed(), t.frame()));
    TimeInfo {
        dt: match (phase, dt) {
            (_, Some(dt)) => dt,
            (Phase::Fixed, None) => fixed_dt,
            (Phase::Update, None) => delta,
        },
        elapsed,
        fixed_dt,
        frame,
    }
}

/// Turns new `Scripts` components and code attachments into instances.
fn attach_new(rt: &mut ScriptRuntime, res: &Resources, world: &mut World) {
    let fresh: Vec<(Entity, Scripts)> = world
        .query::<(Entity, &Scripts)>()
        .without::<&ScriptsAttached>()
        .iter()
        .map(|(e, s)| (e, s.clone()))
        .collect();
    if !fresh.is_empty() {
        let registry = res.get::<ScriptRegistry>();
        for (entity, scripts) in fresh {
            let _ = world.insert_one(entity, ScriptsAttached);
            for desc in &scripts.0 {
                let made = registry.as_ref().and_then(|r| {
                    r.create(
                        &desc.name,
                        &Params::new(&desc.params, Some(world), &desc.name),
                    )
                });
                match made {
                    Some(b) => rt.add(entity, desc.name.clone(), b),
                    None => {
                        if rt.warned.insert(desc.name.clone()) {
                            let known: Vec<&str> = registry
                                .as_ref()
                                .map(|r| r.names().collect())
                                .unwrap_or_default();
                            log::warn!(
                                "unknown script `{}` (registered: {})",
                                desc.name,
                                known.join(", ")
                            );
                        }
                    }
                }
            }
        }
    }
    for (e, name, b) in std::mem::take(&mut rt.pending) {
        if world.contains(e) {
            rt.add(e, name, b);
        }
    }
    if !rt.pending_states.is_empty() {
        let ready: Vec<Entity> = rt
            .pending_states
            .keys()
            .copied()
            .filter(|e| rt.index.contains_key(e) || !world.contains(*e))
            .collect();
        for e in ready {
            let states = rt.pending_states.remove(&e).unwrap_or_default();
            if let Some(&i) = rt.index.get(&e) {
                apply_states(e, &mut rt.entries[i].slots, &states);
            }
        }
    }
}

/// One script pass: attach, start, (collision events), update or fixed update,
/// then apply deferred commands and run `on_destroy`.
pub(crate) fn run_pass(res: &Resources, phase: Phase, dt: Option<f32>) {
    let Some(mut rt) = res.get_mut::<ScriptRuntime>() else {
        return;
    };
    let Some(mut world) = res.get_mut::<World>() else {
        return;
    };
    attach_new(&mut rt, res, &mut world);
    if rt.entries.is_empty() {
        return;
    }
    let mut env = Env {
        world: &mut world,
        res,
        commands: Commands::default(),
        time: time_info(res, phase, dt),
        #[cfg(feature = "input")]
        input: res.get_mut::<olvin_input::ActionState>(),
    };

    for entry in &mut rt.entries {
        let entity = entry.entity;
        if !env.world.contains(entity) {
            continue;
        }
        for slot in &mut entry.slots {
            if !slot.started {
                slot.started = true;
                env.call(entity, slot, "start", |b, c| b.start(c));
            }
        }
    }

    #[cfg(feature = "physics")]
    if phase == Phase::Update {
        dispatch_collisions(&mut rt, &mut env);
    }

    for entry in &mut rt.entries {
        let entity = entry.entity;
        if !env.world.contains(entity) {
            continue;
        }
        for slot in &mut entry.slots {
            match phase {
                Phase::Update => env.call(entity, slot, "update", |b, c| b.update(c)),
                Phase::Fixed => env.call(entity, slot, "fixed_update", |b, c| b.fixed_update(c)),
            }
        }
    }

    apply_commands(&mut rt, &mut env);
}

#[cfg(feature = "physics")]
fn dispatch_collisions(rt: &mut ScriptRuntime, env: &mut Env<'_>) {
    let events: Vec<olvin_physics::CollisionEvent> =
        match env.res.get::<olvin_physics::CollisionEvents>() {
            Some(e) if !e.events.is_empty() => e.events.clone(),
            _ => return,
        };
    for ev in events {
        for (me, other) in [(ev.a, ev.b), (ev.b, ev.a)] {
            let Some(&i) = rt.index.get(&me) else {
                continue;
            };
            for slot in &mut rt.entries[i].slots {
                match (ev.sensor, ev.started) {
                    (false, true) => env.call(me, slot, "on_collision_enter", |b, c| {
                        b.on_collision_enter(c, other)
                    }),
                    (false, false) => env.call(me, slot, "on_collision_exit", |b, c| {
                        b.on_collision_exit(c, other)
                    }),
                    (true, true) => env.call(me, slot, "on_trigger_enter", |b, c| {
                        b.on_trigger_enter(c, other)
                    }),
                    (true, false) => env.call(me, slot, "on_trigger_exit", |b, c| {
                        b.on_trigger_exit(c, other)
                    }),
                }
            }
        }
    }
}

/// Entity and all its descendants (through `Parent`).
fn with_descendants(world: &World, root: Entity) -> Vec<Entity> {
    let links: Vec<(Entity, Entity)> = world
        .query::<(Entity, &Parent)>()
        .iter()
        .map(|(e, p)| (e, p.0))
        .collect();
    let mut out = vec![root];
    let mut i = 0;
    while i < out.len() && out.len() < 1_000_000 {
        let parent = out[i];
        out.extend(
            links
                .iter()
                .filter(|(c, p)| *p == parent && !out.contains(c))
                .map(|(c, _)| *c)
                .collect::<Vec<_>>(),
        );
        i += 1;
    }
    out
}

fn destroy_entry(env: &mut Env<'_>, mut entry: Entry) {
    for slot in &mut entry.slots {
        if slot.started {
            env.call(entry.entity, slot, "on_destroy", |b, c| b.on_destroy(c));
        }
    }
}

fn apply_commands(rt: &mut ScriptRuntime, env: &mut Env<'_>) {
    // on_destroy may despawn more; a few rounds settle it.
    for _ in 0..8 {
        let despawn = std::mem::take(&mut env.commands.despawn);
        if despawn.is_empty() {
            break;
        }
        let mut doomed = Vec::new();
        for e in despawn {
            if env.world.contains(e) {
                doomed.extend(with_descendants(env.world, e));
            }
        }
        doomed.sort();
        doomed.dedup();
        for &e in &doomed {
            if let Some(entry) = rt.remove(e) {
                destroy_entry(env, entry);
            }
        }
        for e in doomed {
            let _ = env.world.despawn(e);
        }
    }
    env.commands.buffer.run_on(env.world);
    for (e, name, b) in std::mem::take(&mut env.commands.attach) {
        rt.pending.push((e, name, b));
    }
    // Entities that vanished some other way.
    let gone: Vec<Entity> = rt
        .entries
        .iter()
        .map(|e| e.entity)
        .filter(|e| !env.world.contains(*e))
        .collect();
    for e in gone {
        if let Some(entry) = rt.remove(e) {
            destroy_entry(env, entry);
        }
    }
    // Commands from late on_destroy calls.
    env.commands.buffer.run_on(env.world);
}

/// Runs one `update` pass now. `dt` overrides the frame delta (deterministic
/// tests, replays, headless tools); the plugin calls it with `None`.
pub fn run_update(res: &Resources, dt: Option<f32>) {
    run_pass(res, Phase::Update, dt);
}

/// Runs one `fixed_update` pass now (the plugin does this before each physics step).
pub fn run_fixed_update(res: &Resources) {
    run_pass(res, Phase::Fixed, None);
}
