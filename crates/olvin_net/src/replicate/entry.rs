//! One registered replicated component type, erased so the registry can hold a list of them.

use std::collections::{HashMap, VecDeque};

use olvin_ecs::{Component, Entity, World};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::{Interp, ReplicateMode, ReplicateOptions};

/// Interpolation samples kept per entity and component.
const MAX_SAMPLES: usize = 32;

pub(crate) trait ErasedComponent {
    fn name(&self) -> &str;
    fn mode(&self) -> ReplicateMode;
    fn interpolated(&self) -> bool;
    fn type_id(&self) -> std::any::TypeId;

    /// Server: the entity's value, quantised and encoded; `None` if it has none.
    fn encode(&self, world: &World, entity: Entity) -> Option<Vec<u8>>;

    /// Client: a value (or its removal) for `id`, valid at server time `time`.
    /// Bad bytes are an `Err`, never a panic.
    fn apply(
        &mut self,
        world: &mut World,
        entity: Entity,
        id: u64,
        time: f64,
        data: Option<&[u8]>,
    ) -> Result<(), String>;

    /// Client: writes the interpolated value for `render_time` into every entity with samples.
    fn interpolate(&mut self, world: &mut World, entities: &HashMap<u64, Entity>, render_time: f64);

    /// Client: the entity is gone.
    fn forget(&mut self, id: u64);
}

struct Typed<T> {
    name: String,
    opts: ReplicateOptions<T>,
    samples: HashMap<u64, VecDeque<(f64, T)>>,
}

pub(crate) fn new_entry<T>(name: &str, opts: ReplicateOptions<T>) -> Box<dyn ErasedComponent>
where
    T: Component + Clone + Serialize + DeserializeOwned,
{
    Box::new(Typed {
        name: name.to_owned(),
        opts,
        samples: HashMap::new(),
    })
}

impl<T> ErasedComponent for Typed<T>
where
    T: Component + Clone + Serialize + DeserializeOwned,
{
    fn name(&self) -> &str {
        &self.name
    }

    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<T>()
    }

    fn mode(&self) -> ReplicateMode {
        self.opts.mode
    }

    fn interpolated(&self) -> bool {
        matches!(self.opts.interp, Interp::Custom(_))
    }

    fn encode(&self, world: &World, entity: Entity) -> Option<Vec<u8>> {
        let mut value: T = T::clone(&*world.get::<&T>(entity).ok()?);
        if let Some(q) = self.opts.quantize {
            q(&mut value);
        }
        let mut out = Vec::new();
        if self.opts.ack_prefix {
            let ack = world
                .get::<&crate::predict::InputAck>(entity)
                .map_or(0, |a| a.0);
            super::codec::put_varint(&mut out, ack as u64);
        }
        match &self.opts.codec {
            Some(codec) => (codec.encode)(&value, &mut out),
            None => out.extend(postcard::to_stdvec(&value).ok()?),
        }
        Some(out)
    }

    fn apply(
        &mut self,
        world: &mut World,
        entity: Entity,
        id: u64,
        time: f64,
        data: Option<&[u8]>,
    ) -> Result<(), String> {
        let Some(bytes) = data else {
            self.samples.remove(&id);
            let _ = world.remove_one::<T>(entity);
            return Ok(());
        };
        let mut bytes = bytes;
        if self.opts.ack_prefix {
            super::codec::get_varint(&mut bytes)?;
        }
        let value: T = match &self.opts.codec {
            Some(codec) => (codec.decode)(bytes)?,
            None => {
                let (v, rest) = postcard::take_from_bytes::<T>(bytes).map_err(|e| e.to_string())?;
                if !rest.is_empty() {
                    return Err("trailing bytes".into());
                }
                v
            }
        };
        if !self.interpolated() {
            return world.insert_one(entity, value).map_err(|e| e.to_string());
        }
        let buf = self.samples.entry(id).or_default();
        // Duplicates and stale snapshots are dropped here.
        if buf.back().is_some_and(|(t, _)| *t >= time) {
            return Ok(());
        }
        buf.push_back((time, value));
        while buf.len() > MAX_SAMPLES {
            buf.pop_front();
        }
        Ok(())
    }

    fn interpolate(
        &mut self,
        world: &mut World,
        entities: &HashMap<u64, Entity>,
        render_time: f64,
    ) {
        let Interp::Custom(lerp) = self.opts.interp else {
            return;
        };
        for (id, buf) in &mut self.samples {
            let Some(&entity) = entities.get(id) else {
                continue;
            };
            // Drop samples we are past, keeping the one just behind `render_time`.
            while buf.len() >= 2 && buf[1].0 <= render_time {
                buf.pop_front();
            }
            let value = match (buf.front(), buf.get(1)) {
                (Some((t0, a)), Some((t1, b))) if render_time > *t0 => {
                    let t = ((render_time - t0) / (t1 - t0)).clamp(0.0, 1.0) as f32;
                    lerp(a, b, t)
                }
                (Some((_, a)), _) => a.clone(),
                _ => continue,
            };
            let _ = world.insert_one(entity, value);
        }
    }

    fn forget(&mut self, id: u64) {
        self.samples.remove(&id);
    }
}
