//! L2: replication. The server owns the world; clients get copies.
//!
//! * [`Replicated`] on an entity makes it a networked object. The server
//!   assigns a [`NetId`]; clients create the same prefab (by asset path) and
//!   keep its replicated components up to date.
//! * Which components travel is registered by type: `net.replicate::<Health>()`.
//! * [`Owner`] says which client controls an entity. It never lets a client
//!   write state: clients send commands, the server changes the world.
//! * Remote entities are shown a little in the past ([`Interp`]) so loss and
//!   jitter between snapshots are invisible.

pub(crate) mod client;
mod codec;
mod entry;
pub(crate) mod server;

use serde::{Deserialize, Serialize};

pub(crate) use client::ReplClient;
pub use codec::{Codec, transform_codec};
pub(crate) use entry::{ErasedComponent, new_entry};
pub(crate) use server::ReplServer;

use crate::net::ClientId;

/// Marks an entity as a networked object. Add it in code, or put a `Replicated`
/// entry in the scene or prefab `components` in the editor.
///
/// The client copy is made from the entity's `PrefabInstanceOf` (so "a networked
/// object is a prefab with a marker"), or from `prefab` here if you set it. An
/// entity from a scene file with neither is matched by its `SceneId` on both sides.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Replicated {
    /// Prefab to instantiate on clients, overriding `PrefabInstanceOf`.
    pub prefab: Option<String>,
}

/// Identifies a replicated entity across the network. Assigned by the server,
/// never reused in a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NetId(pub u64);

/// The client in control of an entity. Replicated like any component. The
/// server sets it (`world.insert_one(e, Owner(id))`); changing it is how
/// ownership moves. A client cannot change it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owner(pub ClientId);

/// On the client's copy of a replicated entity (not on the server's own entity).
/// Systems that must only run for the authoritative copy, like physics or AI,
/// query `Without<Remote>`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Remote;

/// On a client entity that has at least one interpolated component.
#[derive(Clone, Copy, Debug, Default)]
pub struct Interpolated;

/// When a component is sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplicateMode {
    /// Whenever its value changed (compared after quantisation).
    OnChange,
    /// Checked every `n` ticks: fine for values that change constantly but matter little.
    Interval(u32),
    /// Every tick, changed or not.
    Always,
}

/// How a remote component is shown between snapshots.
pub enum Interp<T> {
    /// Jump to the newest value.
    None,
    /// `f(a, b, t)` between two snapshots, `t` in `0..1`. Use [`Interpolate::interpolate`]
    /// for the common case ([`ReplicateOptions::interpolated`]).
    Custom(fn(&T, &T, f32) -> T),
}

/// Blending of two values for interpolation. Implemented for `f32`, `Vec3`,
/// `Quat` (slerp) and `Transform`; implement it for your own types.
pub trait Interpolate {
    fn interpolate(&self, other: &Self, t: f32) -> Self;
}

impl Interpolate for f32 {
    fn interpolate(&self, other: &Self, t: f32) -> Self {
        self + (other - self) * t
    }
}

impl Interpolate for glam::Vec3 {
    fn interpolate(&self, other: &Self, t: f32) -> Self {
        self.lerp(*other, t)
    }
}

impl Interpolate for glam::Quat {
    fn interpolate(&self, other: &Self, t: f32) -> Self {
        self.slerp(*other, t)
    }
}

impl Interpolate for olvin_scene::Transform {
    fn interpolate(&self, other: &Self, t: f32) -> Self {
        Self {
            translation: self.translation.lerp(other.translation, t),
            rotation: self.rotation.slerp(other.rotation, t),
            scale: self.scale.lerp(other.scale, t),
        }
    }
}

/// Per-component settings for [`Net::replicate_with`](crate::Net::replicate_with).
pub struct ReplicateOptions<T> {
    pub mode: ReplicateMode,
    /// Rounds the value before it is compared and sent: smaller packets for a
    /// codec that is, and no resend for jitter below the step.
    pub quantize: Option<fn(&mut T)>,
    pub interp: Interp<T>,
    /// Your own byte format (default: `postcard` of the serde form).
    pub codec: Option<Codec<T>>,
}

impl<T> Default for ReplicateOptions<T> {
    fn default() -> Self {
        Self {
            mode: ReplicateMode::OnChange,
            quantize: None,
            interp: Interp::None,
            codec: None,
        }
    }
}

impl<T> ReplicateOptions<T> {
    pub fn mode(mut self, mode: ReplicateMode) -> Self {
        self.mode = mode;
        self
    }

    /// Checks the value every `n` ticks.
    pub fn every(self, n: u32) -> Self {
        self.mode(ReplicateMode::Interval(n.max(1)))
    }

    pub fn always(self) -> Self {
        self.mode(ReplicateMode::Always)
    }

    pub fn quantize(mut self, f: fn(&mut T)) -> Self {
        self.quantize = Some(f);
        self
    }

    pub fn interpolate_with(mut self, f: fn(&T, &T, f32) -> T) -> Self {
        self.interp = Interp::Custom(f);
        self
    }

    pub fn codec(mut self, codec: Codec<T>) -> Self {
        self.codec = Some(codec);
        self
    }
}

impl<T: Interpolate> ReplicateOptions<T> {
    /// Interpolates with [`Interpolate::interpolate`] (lerp, slerp for rotations).
    pub fn interpolated(self) -> Self {
        self.interpolate_with(T::interpolate)
    }
}

/// Seconds of server time a tick is worth.
pub(crate) fn tick_time(tick: u32, tick_rate: u32) -> f64 {
    tick as f64 / tick_rate.max(1) as f64
}

/// Prefab paths from the network are only ever relative `*.prefab.ron` assets:
/// a server must not make a client open arbitrary files.
pub(crate) fn valid_prefab_path(path: &str) -> bool {
    path.len() <= 200
        && path.ends_with(".prefab.ron")
        && !path.starts_with('/')
        && !path.contains(['\\', ':', '\0'])
        && path
            .split('/')
            .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

/// The type's last path segment without generics: `my_game::Health` is `Health`.
pub(crate) fn short_type_name<T>() -> String {
    let full = std::any::type_name::<T>();
    let base = full.split('<').next().unwrap_or(full);
    base.rsplit("::").next().unwrap_or(base).to_owned()
}

/// Defaults for `T`: `Transform` gets the compact codec and interpolation, everything else plain.
pub(crate) fn default_options<T: 'static>() -> ReplicateOptions<T> {
    if std::any::TypeId::of::<T>() == std::any::TypeId::of::<olvin_scene::Transform>() {
        let opts: Box<dyn std::any::Any> = Box::new(
            ReplicateOptions::<olvin_scene::Transform>::default()
                .codec(transform_codec())
                .interpolated(),
        );
        if let Ok(opts) = opts.downcast::<ReplicateOptions<T>>() {
            return *opts;
        }
    }
    ReplicateOptions::default()
}

/// Relevancy by distance to the entities a client owns.
pub(crate) fn distance_relevancy(
    radius: f32,
) -> impl Fn(&olvin_ecs::World, ClientId, olvin_ecs::Entity) -> bool + 'static {
    move |world, client, entity| {
        let Ok(t) = world.get::<&olvin_scene::Transform>(entity) else {
            return true;
        };
        let mut owns_any = false;
        for (o, ot) in world.query::<(&Owner, &olvin_scene::Transform)>().iter() {
            if o.0 == client {
                owns_any = true;
                if ot.translation.distance(t.translation) <= radius {
                    return true;
                }
            }
        }
        // An owner always sees its own entities, and a client without one sees everything.
        !owns_any || world.get::<&Owner>(entity).is_ok_and(|o| o.0 == client)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefab_paths_are_confined() {
        assert!(valid_prefab_path("prefabs/player.prefab.ron"));
        for bad in [
            "../x.prefab.ron",
            "/etc/passwd.prefab.ron",
            "a/../b.prefab.ron",
            "a\\b.prefab.ron",
            "c:x.prefab.ron",
            "player.ron",
            "a//b.prefab.ron",
            "",
        ] {
            assert!(!valid_prefab_path(bad), "{bad}");
        }
        assert!(!valid_prefab_path(&format!(
            "{}.prefab.ron",
            "a".repeat(300)
        )));
    }

    #[test]
    fn replicated_parses_from_scene_text() {
        // What a designer's inspector stores in `EntityDesc::components`.
        for text in [
            "()",
            "(prefab: None)",
            "Replicated()",
            "(prefab: Some(\"p.prefab.ron\"))",
        ] {
            let r: Result<Replicated, _> = ron::from_str(text);
            assert!(r.is_ok(), "{text}: {r:?}");
        }
    }
}
