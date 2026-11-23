//! Buses, EQ, reverb sends and occlusion.

use glam::Vec3;

use mixer::Bus;

use super::*;

impl Audio {
    /// A bus by name (`sfx`, `music`, `ambient`, `voice`, or one you added).
    pub fn bus(&self, name: &str) -> Option<BusId> {
        self.buses.iter().position(|b| b.name == name).map(BusId)
    }

    /// Creates a bus (or returns the one with that name).
    pub fn add_bus(&mut self, name: &str) -> BusId {
        if let Some(id) = self.bus(name) {
            return id;
        }
        let mut bus = Bus::new(name);
        if let Some(m) = &mut self.manager {
            bus.open(m, self.reverb.as_ref());
        }
        self.buses.push(bus);
        BusId(self.buses.len() - 1)
    }

    pub fn bus_name(&self, bus: BusId) -> Option<&str> {
        self.buses.get(bus.0).map(|b| b.name.as_str())
    }

    pub fn bus_volume(&self, bus: BusId) -> f32 {
        self.buses.get(bus.0).map_or(0.0, |b| b.volume)
    }

    /// Volume of everything on the bus (linear).
    pub fn set_bus_volume(&mut self, bus: BusId, volume: f32) {
        if let Some(b) = self.buses.get_mut(bus.0) {
            b.volume = volume.max(0.0);
            b.apply_volume();
        }
    }

    pub fn is_bus_muted(&self, bus: BusId) -> bool {
        self.buses.get(bus.0).is_some_and(|b| b.muted)
    }

    /// Silences a bus without losing its volume setting.
    pub fn set_bus_muted(&mut self, bus: BusId, muted: bool) {
        if let Some(b) = self.buses.get_mut(bus.0) {
            b.muted = muted;
            b.apply_volume();
        }
    }

    pub fn bus_eq(&self, bus: BusId) -> EqSettings {
        self.buses.get(bus.0).map(|b| b.eq).unwrap_or_default()
    }

    /// Equalizer of the bus; gains are clamped to ±[`EQ_MAX_DB`].
    pub fn set_bus_eq(&mut self, bus: BusId, eq: EqSettings) {
        if let Some(b) = self.buses.get_mut(bus.0) {
            b.set_eq(eq);
        }
    }

    pub fn reverb(&self) -> ReverbSettings {
        self.reverb_settings
    }

    /// Room of the shared reverb (all buses that send into it).
    pub fn set_reverb(&mut self, settings: ReverbSettings) {
        self.reverb_settings = mixer::clamp_reverb(settings);
        if let Some(r) = &mut self.reverb {
            r.apply(self.reverb_settings);
        }
    }

    pub fn bus_reverb(&self, bus: BusId) -> f32 {
        self.buses.get(bus.0).map_or(0.0, |b| b.reverb_send)
    }

    /// How much of the bus goes into the reverb, 0 (none) .. 1.
    pub fn set_bus_reverb(&mut self, bus: BusId, level: f32) {
        if let Some(b) = self.buses.get_mut(bus.0) {
            b.set_reverb_send(level, self.reverb.as_ref());
        }
    }

    // Occlusion.

    /// Tells the mixer how blocked the way between two points is: from the
    /// listener to a sound, 0 (clear) .. 1 (fully behind walls). Typically a
    /// physics ray cast; partial values suit thin or partial obstacles. Called
    /// once per frame for every positional occludable sound with a voice.
    pub fn set_occluder(&mut self, occluder: impl FnMut(Vec3, Vec3) -> f32 + 'static) {
        self.occluder = Some(Box::new(occluder));
    }

    pub fn clear_occluder(&mut self) {
        self.occluder = None;
    }

    /// Sets a sound's occlusion by hand (0..1), overriding the occluder;
    /// `None` gives control back to it.
    pub fn set_occlusion(&mut self, id: SoundId, occlusion: Option<f32>) {
        if let Some(p) = self.playing.get_mut(&id) {
            p.occlusion.manual = occlusion.map(|o| o.clamp(0.0, 1.0));
        }
    }

    /// The occlusion currently applied to a sound (eased), 0..1.
    pub fn occlusion(&self, id: SoundId) -> Option<f32> {
        self.playing.get(&id).map(|p| p.occlusion.current)
    }
}
