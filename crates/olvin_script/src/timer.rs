//! Script timers: `ctx:after(seconds, name)` and `ctx:every(seconds, name)`
//! end as an `on_message(name)` addressed to the object that set them.
//!
//! They are [`olvin_core::Timers`] with a tag `script:<entity>:<name>`; a
//! system turns the fired tags back into [`ScriptMessage`]s, and drops the
//! timers of objects that no longer exist.

use olvin_core::{EventCursor, Events, Resources, TimerFired, Timers};
use olvin_ecs::{Entity, World};

use crate::{ScriptMessage, ScriptValue};

/// Most timers one object may have running; more are refused.
pub const MAX_TIMERS_PER_OBJECT: usize = 32;
/// Most timers of all scripts together.
pub const MAX_SCRIPT_TIMERS: usize = 4096;

const PREFIX: &str = "script:";

pub(crate) fn tag(entity: Entity, name: &str) -> String {
    format!("{PREFIX}{}:{name}", entity.to_bits().get())
}

fn owner_prefix(entity: Entity) -> String {
    format!("{PREFIX}{}:", entity.to_bits().get())
}

/// `script:<bits>:<name>` back to its parts.
fn parse(tag: &str) -> Option<(Entity, &str)> {
    let rest = tag.strip_prefix(PREFIX)?;
    let (bits, name) = rest.split_once(':')?;
    Some((Entity::from_bits(bits.parse().ok()?)?, name))
}

/// Starts a timer for `entity`. False when a limit is reached or there is no
/// [`Timers`] resource (no `TimerPlugin`).
pub(crate) fn start(
    res: &Resources,
    entity: Entity,
    seconds: f32,
    name: &str,
    repeat: bool,
) -> bool {
    let Some(mut timers) = res.get_mut::<Timers>() else {
        return false;
    };
    if timers.len() >= MAX_SCRIPT_TIMERS
        || timers.count_with_prefix(&owner_prefix(entity)) >= MAX_TIMERS_PER_OBJECT
    {
        return false;
    }
    let tag = tag(entity, name);
    if repeat {
        timers.every(seconds, tag);
    } else {
        timers.after(seconds, tag);
    }
    true
}

/// Stops the timers `entity` started with this name; returns how many.
pub(crate) fn cancel(res: &Resources, entity: Entity, name: &str) -> usize {
    res.get_mut::<Timers>()
        .map_or(0, |mut t| t.cancel_tag(&tag(entity, name)))
}

/// The system: fired script timers become messages, orphans are dropped.
pub(crate) fn bridge(res: &mut Resources, cursor: &mut EventCursor) {
    let fired: Vec<TimerFired> = match res.get::<Events<TimerFired>>() {
        Some(events) => events.read(cursor).cloned().collect(),
        None => return,
    };
    if fired.is_empty() {
        return;
    }
    let mut messages = Vec::new();
    let mut orphans = Vec::new();
    {
        let Some(world) = res.get::<World>() else {
            return;
        };
        for f in &fired {
            let Some((entity, name)) = parse(&f.tag) else {
                continue;
            };
            if world.contains(entity) {
                messages.push(ScriptMessage {
                    name: name.to_owned(),
                    value: ScriptValue::Nil,
                    from: Some(entity),
                    target: Some(entity),
                });
            } else {
                orphans.push(owner_prefix(entity));
            }
        }
    }
    if let Some(mut timers) = res.get_mut::<Timers>() {
        for prefix in orphans {
            timers.cancel_prefix(&prefix);
        }
    }
    if let Some(mut events) = res.get_mut::<Events<ScriptMessage>>() {
        events.send_batch(messages);
    }
}
