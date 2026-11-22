//! Particles. A [`ParticleEmitter`] component holds plain serializable
//! parameters ([`EmitterParams`]: rate, bursts, lifetime, speed, emission
//! shape, gravity, colour and size over life) and the live particles. Each
//! frame [`update`] steps them in `Stage::PostUpdate`, and the
//! [`ParticleRenderer`] draws them as camera-facing quads, one instanced
//! draw per texture / blend run. Nothing allocates in the steady state: the
//! particle storage is reserved up to `max_particles` (at most
//! [`MAX_RESERVED`] slots; a bigger cap just grows on demand) and swap-removed.
//!
//! One API, two simulations ([`ParticleMode`]): on the CPU (every tier) the
//! emitter owns its particles; on the GPU (Full tier, `EmitterParams::gpu`) the
//! emitter only schedules births and the compute pass in
//! [`crate::particles_gpu`] simulates them in a storage buffer, so tens of
//! thousands cost almost nothing on the CPU.

use std::collections::VecDeque;
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicBool, Ordering};

/// Most particle slots reserved up front: `max_particles` comes from files,
/// and a typo must not turn into a multi-gigabyte allocation.
pub const MAX_RESERVED: usize = 1 << 16;

use glam::{Mat4, Vec3, Vec4Swizzles};
use olvin_core::{Resources, Time};
use olvin_ecs::Entity;
use olvin_ecs::World;
use olvin_transform::GlobalTransform;

use crate::core::GpuMeshes;
use crate::particles_gpu::{self, GpuParticle, GpuSim};
use crate::quad::{BatchKey, QuadBatcher, QuadBlend, QuadFilter, QuadInstance};
use olvin_scene::Transform;

use crate::{TextureData, TextureHandle};

/// Where particles are born, in the emitter's local space; the emission
/// direction depends on the shape.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum EmitShape {
    /// One point, any direction.
    Point,
    /// Inside a sphere (or only on its `surface`), flying outwards.
    Sphere { radius: f32, surface: bool },
    /// The upper (+Y) half of a sphere.
    Hemisphere { radius: f32, surface: bool },
    /// A disc in XZ spraying into a cone around +Y (`angle_deg` from the axis).
    Cone { angle_deg: f32, radius: f32 },
    /// Inside a box, flying along +Y.
    Box { half_extents: Vec3 },
    /// A disc in XZ (or only its `edge`), flying outwards in the plane.
    Circle { radius: f32, edge: bool },
}

/// Particles emitted at one moment of the emitter's cycle.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Burst {
    /// Seconds since the cycle started.
    pub time: f32,
    pub count: u32,
}

/// Whether particles follow the emitter once born.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SimSpace {
    /// Left behind in the world as the emitter moves (smoke trails).
    #[default]
    World,
    /// Carried along with the emitter (auras, thrusters); gravity is local too.
    Local,
}

/// A texture cut into `columns × rows` equal frames, played once over a
/// particle's life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FlipBook {
    pub columns: u32,
    pub rows: u32,
}

/// Everything an emitter does, as data for scenes and the editor. Ranges are
/// `[min, max]`, picked uniformly per particle.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct EmitterParams {
    /// Cap on live particles; further births wait.
    pub max_particles: u32,
    /// Particles per second while emitting.
    pub rate: f32,
    pub bursts: Vec<Burst>,
    /// Length of one emission cycle, seconds.
    pub duration: f32,
    /// Restart the cycle (and its bursts) after `duration`; otherwise emit once.
    pub looping: bool,
    pub shape: EmitShape,
    pub lifetime: [f32; 2],
    /// Initial speed along the emission direction.
    pub speed: [f32; 2],
    /// Initial size (quad edge, world units).
    pub size: [f32; 2],
    /// Initial rotation, radians.
    pub rotation: [f32; 2],
    /// Rotation speed, radians per second.
    pub spin: [f32; 2],
    pub gravity: Vec3,
    /// Fraction of velocity lost per second.
    pub drag: f32,
    pub space: SimSpace,
    /// Linear RGBA multiplying `color_over_life`.
    pub color: [f32; 4],
    /// With a value, each particle picks a random tint between `color` and this.
    pub color_max: Option<[f32; 4]>,
    /// `(life fraction, RGBA)`, ascending, linearly interpolated; empty = white.
    pub color_over_life: Vec<(f32, [f32; 4])>,
    /// `(life fraction, size multiplier)`, ascending; empty = constant.
    pub size_over_life: Vec<(f32, f32)>,
    pub blend: QuadBlend,
    pub flip_book: Option<FlipBook>,
    /// Draw far particles first (alpha blending only).
    pub sort: bool,
    /// Texture to look up by the scene loader, which sets
    /// [`ParticleEmitter::texture`].
    pub texture_path: Option<String>,
    /// Random seed: same seed, same sequence (on the CPU; the GPU path is
    /// deterministic per step but draws other numbers).
    pub seed: u32,
    /// Start as if one full lifetime had already been simulated.
    pub prewarm: bool,
    /// Ask for the GPU simulation; it is used where the device can compute
    /// (see [`gpu_particles_available`]), the CPU otherwise.
    pub gpu: bool,
}

impl Default for EmitterParams {
    fn default() -> Self {
        Self {
            max_particles: 1000,
            rate: 20.0,
            bursts: Vec::new(),
            duration: 5.0,
            looping: true,
            shape: EmitShape::Cone {
                angle_deg: 25.0,
                radius: 0.1,
            },
            lifetime: [1.0, 2.0],
            speed: [1.0, 2.0],
            size: [0.2, 0.3],
            rotation: [0.0, 0.0],
            spin: [0.0, 0.0],
            gravity: Vec3::new(0.0, -9.81, 0.0),
            drag: 0.0,
            space: SimSpace::World,
            color: [1.0; 4],
            color_max: None,
            color_over_life: vec![(0.0, [1.0; 4]), (1.0, [1.0, 1.0, 1.0, 0.0])],
            size_over_life: Vec::new(),
            blend: QuadBlend::Alpha,
            flip_book: None,
            sort: true,
            texture_path: None,
            seed: 1,
            prewarm: false,
            gpu: false,
        }
    }
}

impl EmitterParams {
    /// Colour at `t` (0..1 of a particle's life), `color` tint included.
    pub fn color_at(&self, t: f32) -> [f32; 4] {
        let c = self.life_color(t);
        std::array::from_fn(|k| c[k] * self.color[k])
    }

    /// The gradient at `t`, without the tint.
    pub fn life_color(&self, t: f32) -> [f32; 4] {
        match self.color_over_life.as_slice() {
            [] => [1.0; 4],
            [(_, c)] => *c,
            stops => {
                let i = stops.partition_point(|(s, _)| *s <= t);
                if i == 0 {
                    stops[0].1
                } else if i == stops.len() {
                    stops[i - 1].1
                } else {
                    let ((t0, a), (t1, b)) = (stops[i - 1], stops[i]);
                    let f = ((t - t0) / (t1 - t0).max(1e-6)).clamp(0.0, 1.0);
                    std::array::from_fn(|k| a[k] + (b[k] - a[k]) * f)
                }
            }
        }
    }

    /// Size multiplier at `t`.
    pub fn size_at(&self, t: f32) -> f32 {
        match self.size_over_life.as_slice() {
            [] => 1.0,
            [(_, v)] => *v,
            keys => {
                let i = keys.partition_point(|(s, _)| *s <= t);
                if i == 0 {
                    keys[0].1
                } else if i == keys.len() {
                    keys[i - 1].1
                } else {
                    let ((t0, a), (t1, b)) = (keys[i - 1], keys[i]);
                    a + (b - a) * ((t - t0) / (t1 - t0).max(1e-6)).clamp(0.0, 1.0)
                }
            }
        }
    }
}

/// The default particle sprite: a white disc whose alpha falls off smoothly to
/// the rim (sRGB texture data, 64 x 64).
pub fn soft_circle_texture() -> TextureData {
    const N: u32 = 64;
    let mut pixels = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            let (dx, dy) = (
                (x as f32 + 0.5) / N as f32 * 2.0 - 1.0,
                (y as f32 + 0.5) / N as f32 * 2.0 - 1.0,
            );
            let r = (dx * dx + dy * dy).sqrt().min(1.0);
            let a = (1.0 - r) * (1.0 - r) * (3.0 - 2.0 * (1.0 - r));
            pixels.extend_from_slice(&[255, 255, 255, (a * 255.0 + 0.5) as u8]);
        }
    }
    TextureData::rgba8(N, N, pixels, true)
}

/// PCG-style generator: tiny, deterministic, no dependencies.
#[derive(Clone, Copy, Debug)]
struct Rng(u32);

impl Rng {
    fn new(seed: u32) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9) | 1)
    }

    fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
        let word = ((self.0 >> ((self.0 >> 28) + 4)) ^ self.0).wrapping_mul(277_803_737);
        (word >> 22) ^ word
    }

    /// In [0, 1).
    fn unit(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }

    fn range(&mut self, [lo, hi]: [f32; 2]) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    fn on_sphere(&mut self) -> Vec3 {
        let z = self.unit() * 2.0 - 1.0;
        let a = self.unit() * TAU;
        let r = (1.0 - z * z).sqrt();
        Vec3::new(r * a.cos(), z, r * a.sin())
    }

    fn in_disc(&mut self, radius: f32) -> (f32, f32) {
        let r = radius * self.unit().sqrt();
        let a = self.unit() * TAU;
        (r * a.cos(), r * a.sin())
    }
}

impl EmitShape {
    /// A birth point and a unit direction, in the emitter's local space.
    fn sample(&self, rng: &mut Rng) -> (Vec3, Vec3) {
        match *self {
            Self::Point => (Vec3::ZERO, rng.on_sphere()),
            Self::Sphere { radius, surface } => {
                let dir = rng.on_sphere();
                let r = if surface {
                    radius
                } else {
                    radius * rng.unit().cbrt()
                };
                (dir * r, dir)
            }
            Self::Hemisphere { radius, surface } => {
                let mut dir = rng.on_sphere();
                dir.y = dir.y.abs();
                let r = if surface {
                    radius
                } else {
                    radius * rng.unit().cbrt()
                };
                (dir * r, dir)
            }
            Self::Cone { angle_deg, radius } => {
                let (x, z) = rng.in_disc(radius);
                let cos = 1.0 + (angle_deg.to_radians().cos() - 1.0) * rng.unit();
                let sin = (1.0 - cos * cos).max(0.0).sqrt();
                let a = rng.unit() * TAU;
                (
                    Vec3::new(x, 0.0, z),
                    Vec3::new(sin * a.cos(), cos, sin * a.sin()),
                )
            }
            Self::Box { half_extents } => {
                let p = Vec3::new(rng.unit(), rng.unit(), rng.unit()) * 2.0 - Vec3::ONE;
                (p * half_extents, Vec3::Y)
            }
            Self::Circle { radius, edge } => {
                let (x, z) = if edge {
                    let a = rng.unit() * TAU;
                    (radius * a.cos(), radius * a.sin())
                } else {
                    rng.in_disc(radius)
                };
                let dir = Vec3::new(x, 0.0, z).try_normalize().unwrap_or(Vec3::X);
                (Vec3::new(x, 0.0, z), dir)
            }
        }
    }
}

/// Most particle slots of one GPU emitter (64 bytes each).
pub const MAX_GPU_PARTICLES: u32 = 1 << 20;

/// Most simulation steps an emitter keeps for the GPU renderers to catch up on.
const MAX_STEPS: usize = 32;

static GPU_AVAILABLE: AtomicBool = AtomicBool::new(false);

/// Whether a renderer on a compute-capable device exists (Full tier). Emitters
/// with `EmitterParams::gpu` switch to the GPU simulation once it does; on
/// Compat (WebGL2/GLES3, no compute) it stays false and they run on the CPU.
pub fn gpu_particles_available() -> bool {
    GPU_AVAILABLE.load(Ordering::Relaxed)
}

pub(crate) fn set_gpu_particles_available(on: bool) {
    GPU_AVAILABLE.store(on, Ordering::Relaxed);
}

/// Where an emitter's particles are simulated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ParticleMode {
    /// On the CPU, in the emitter ([`ParticleEmitter::count`] is exact).
    #[default]
    Cpu,
    /// In a compute pass; the emitter only schedules births.
    Gpu,
}

/// One simulation step of a GPU emitter: the births of a frame.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GpuStep {
    pub seq: u64,
    pub dt: f32,
    pub births: u32,
    /// The emitter's world matrix at the step.
    pub matrix: Mat4,
}

#[derive(Clone, Copy, Debug)]
struct Particle {
    pos: Vec3,
    vel: Vec3,
    age: f32,
    life: f32,
    rot: f32,
    spin: f32,
    size: f32,
    tint: [f32; 4],
}

/// Emits and simulates particles at the entity's transform. Draw it with
/// [`ParticleRenderer`]; step it with [`update`] (the plugin does).
#[derive(Clone, Debug)]
pub struct ParticleEmitter {
    pub params: EmitterParams,
    pub texture: Option<TextureHandle>,
    /// Emission on/off; live particles keep flying either way.
    pub playing: bool,
    particles: Vec<Particle>,
    rng: Rng,
    /// Seconds into the cycle.
    time: f32,
    /// Fractional particles carried to the next step.
    carry: f32,
    /// Emission of the cycle is over (not looping).
    finished: bool,
    /// Queued by [`Self::burst`].
    pending: u32,
    mode: ParticleMode,
    /// The mode was set by hand ([`Self::force_mode`]): not re-derived from
    /// `params.gpu` and the device.
    mode_forced: bool,
    /// The prewarm of this cycle has run.
    warmed: bool,
    /// GPU mode: queued steps, newest last, and the next sequence number.
    steps: VecDeque<GpuStep>,
    seq: u64,
    /// Bumped when the GPU particles must be wiped ([`Self::clear`]).
    epoch: u32,
    /// GPU mode: seconds until the last birth has died (for [`Self::is_done`]).
    gpu_alive_for: f32,
}

impl Default for ParticleEmitter {
    fn default() -> Self {
        Self::new(EmitterParams::default())
    }
}

impl ParticleEmitter {
    pub fn new(params: EmitterParams) -> Self {
        Self {
            rng: Rng::new(params.seed),
            particles: Vec::with_capacity((params.max_particles as usize).min(MAX_RESERVED)),
            params,
            texture: None,
            playing: true,
            time: 0.0,
            carry: 0.0,
            finished: false,
            pending: 0,
            mode: ParticleMode::Cpu,
            mode_forced: false,
            warmed: false,
            steps: VecDeque::new(),
            seq: 0,
            epoch: 0,
            gpu_alive_for: 0.0,
        }
    }

    pub fn with_texture(mut self, texture: TextureHandle) -> Self {
        self.texture = Some(texture);
        self
    }

    /// Live particles (CPU mode; the GPU keeps its own, see
    /// [`ParticleRenderer::read_gpu`]).
    pub fn count(&self) -> usize {
        self.particles.len()
    }

    /// Lifetimes of the live particles (statistical tests).
    #[cfg(test)]
    pub(crate) fn live_lives(&self) -> Vec<f32> {
        self.particles.iter().map(|p| p.life).collect()
    }

    /// Where the particles are simulated right now.
    pub fn mode(&self) -> ParticleMode {
        self.mode
    }

    /// Pins the simulation to `mode`, whatever `params.gpu` and the device say
    /// (tests, tools). A GPU emitter on a device without compute draws nothing.
    pub fn force_mode(&mut self, mode: ParticleMode) {
        self.mode_forced = true;
        self.switch_mode(mode);
    }

    fn switch_mode(&mut self, mode: ParticleMode) {
        if self.mode != mode {
            self.mode = mode;
            self.particles.clear();
            self.steps.clear();
            self.epoch = self.epoch.wrapping_add(1);
            self.gpu_alive_for = 0.0;
        }
    }

    /// Bumped when the GPU particles must be wiped.
    pub(crate) fn epoch(&self) -> u32 {
        self.epoch
    }

    /// Steps newer than `seq`, oldest first.
    pub(crate) fn steps_after(&self, seq: u64) -> impl Iterator<Item = &GpuStep> {
        self.steps.iter().filter(move |s| s.seq > seq)
    }

    /// Whether nothing is alive and nothing more will be born (a one-shot
    /// effect that can be despawned).
    pub fn is_done(&self) -> bool {
        self.particles.is_empty()
            && self.gpu_alive_for <= 0.0
            && self.pending == 0
            && (self.finished || (!self.playing && self.params.rate <= 0.0))
    }

    /// Starts the cycle over (and resumes emission); live particles stay.
    pub fn restart(&mut self) {
        self.time = 0.0;
        self.carry = 0.0;
        self.finished = false;
        self.playing = true;
        self.warmed = false;
    }

    /// Removes every live particle.
    pub fn clear(&mut self) {
        self.particles.clear();
        self.steps.clear();
        self.epoch = self.epoch.wrapping_add(1);
        self.gpu_alive_for = 0.0;
    }

    /// Queues `count` extra particles for the next [`Self::update`].
    pub fn burst(&mut self, count: u32) {
        self.pending = self.pending.saturating_add(count);
    }

    /// Advances by `dt` seconds. `matrix` is the emitter's world transform.
    pub fn update(&mut self, dt: f32, matrix: &Mat4) {
        // NaN would poison `time` and `carry` for good.
        if dt.is_nan() || dt <= 0.0 {
            return;
        }
        if !self.mode_forced {
            let want = if self.params.gpu && gpu_particles_available() {
                ParticleMode::Gpu
            } else {
                ParticleMode::Cpu
            };
            self.switch_mode(want);
        }
        if self.params.prewarm && !self.warmed {
            self.warmed = true;
            if self.playing {
                // One lifetime in coarse steps; the real step follows.
                let span = self.params.lifetime[1].clamp(0.0, 30.0);
                let n = if self.mode == ParticleMode::Gpu {
                    24
                } else {
                    60
                };
                for _ in 0..n {
                    self.step(span / n as f32, matrix);
                }
            }
        }
        self.step(dt, matrix);
    }

    fn step(&mut self, dt: f32, matrix: &Mat4) {
        if dt <= 0.0 {
            return;
        }
        let max = self.params.max_particles as usize;
        if self.mode == ParticleMode::Cpu {
            let reserved = max.min(MAX_RESERVED);
            if self.particles.capacity() < reserved {
                self.particles.reserve(reserved - self.particles.len());
            }
            self.step_particles(dt);
        } else {
            self.gpu_alive_for = (self.gpu_alive_for - dt).max(0.0);
        }

        let mut births = std::mem::take(&mut self.pending);
        if self.playing && !self.finished {
            let duration = self.params.duration.max(1e-3);
            let (from, to) = (self.time, self.time + dt);
            let end = to.min(duration);
            self.carry += self.params.rate.max(0.0) * (end - from).max(0.0);
            let whole = self.carry.floor();
            self.carry -= whole;
            births += whole as u32;
            births += self.bursts_in(from, end, to >= duration);
            self.time = to;
            if to >= duration {
                if self.params.looping {
                    self.time = to - duration;
                    births += self.bursts_in(0.0, self.time, false);
                    // A step longer than a whole cycle just loses the excess.
                    self.time = self.time.min(duration);
                } else {
                    self.finished = true;
                }
            }
        }
        if self.mode == ParticleMode::Gpu {
            self.push_step(dt, births, matrix);
            return;
        }
        let room = max.saturating_sub(self.particles.len());
        self.spawn((births as usize).min(room), dt, matrix);
    }

    fn push_step(&mut self, dt: f32, births: u32, matrix: &Mat4) {
        if births > 0 {
            self.gpu_alive_for = self.params.lifetime[1].max(dt);
        }
        self.seq += 1;
        let step = GpuStep {
            seq: self.seq,
            dt,
            births,
            matrix: *matrix,
        };
        // Renderers that fell more than MAX_STEPS behind skip the oldest steps.
        if self.steps.len() >= MAX_STEPS {
            self.steps.pop_front();
        }
        self.steps.push_back(step);
    }

    /// Particles of bursts with `from <= time < to` (`inclusive`: also at `to`).
    fn bursts_in(&self, from: f32, to: f32, inclusive: bool) -> u32 {
        self.params
            .bursts
            .iter()
            .filter(|b| b.time >= from && (b.time < to || (inclusive && b.time <= to)))
            .map(|b| b.count)
            .sum()
    }

    fn step_particles(&mut self, dt: f32) {
        let gravity = self.params.gravity;
        let keep = (1.0 - self.params.drag * dt).max(0.0);
        let mut i = 0;
        while i < self.particles.len() {
            let p = &mut self.particles[i];
            p.age += dt;
            if p.age >= p.life {
                self.particles.swap_remove(i);
                continue;
            }
            p.vel = (p.vel + gravity * dt) * keep;
            p.pos += p.vel * dt;
            p.rot += p.spin * dt;
            i += 1;
        }
    }

    fn spawn(&mut self, count: usize, dt: f32, matrix: &Mat4) {
        for i in 0..count {
            let (local_pos, dir) = self.params.shape.sample(&mut self.rng);
            let speed = self.rng.range(self.params.speed);
            let life = self.rng.range(self.params.lifetime).max(1e-3);
            let (mut pos, mut vel) = (local_pos, dir * speed);
            if self.params.space == SimSpace::World {
                pos = matrix.transform_point3(local_pos);
                vel = matrix.transform_vector3(dir).normalize_or_zero() * speed;
            }
            let mut p = Particle {
                pos,
                vel,
                age: 0.0,
                life,
                rot: self.rng.range(self.params.rotation),
                spin: self.rng.range(self.params.spin),
                size: self.rng.range(self.params.size),
                tint: self.params.color,
            };
            if let Some(max) = self.params.color_max {
                let f = self.rng.unit();
                let min = self.params.color;
                p.tint = std::array::from_fn(|k| min[k] + (max[k] - min[k]) * f);
            }
            // Born somewhere inside this step: spread births over it so a
            // moving emitter leaves a trail, not clumps.
            let early = dt * (1.0 - (i as f32 + 0.5) / count as f32);
            p.age = early.min(life * 0.999);
            p.pos += p.vel * early;
            self.particles.push(p);
        }
    }
}

/// `Stage::PostUpdate`: steps every emitter by the frame time.
pub(crate) fn update(res: &mut Resources) {
    let dt = res.get::<Time>().map_or(0.0, |t| t.delta());
    update_emitters(&mut res.fetch_mut::<World>(), dt);
}

/// Steps every [`ParticleEmitter`] in `world` by `dt` seconds.
pub fn update_emitters(world: &mut World, dt: f32) {
    for (global, transform, emitter) in world.query_mut::<(
        Option<&GlobalTransform>,
        Option<&Transform>,
        &mut ParticleEmitter,
    )>() {
        let matrix = global.map_or_else(
            || transform.map_or(Mat4::IDENTITY, Transform::matrix),
            |g| g.0,
        );
        emitter.update(dt, &matrix);
    }
}

/// Draws [`ParticleEmitter`]s as camera-facing quads into a pass, after the
/// meshes: depth-tested against them, never writing depth.
pub struct ParticleRenderer {
    quads: QuadBatcher,
    scratch: Vec<(f32, QuadInstance)>,
    color: wgpu::TextureFormat,
    /// The compute simulation; `None` where the device cannot run it.
    gpu: Option<GpuSim>,
}

impl ParticleRenderer {
    /// `color` / `samples` / `depth` must match the pass `draw` goes into.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        color: wgpu::TextureFormat,
        samples: u32,
        depth: wgpu::TextureFormat,
    ) -> Self {
        let gpu = particles_gpu::supported(device)
            .then(|| GpuSim::new(device, queue, color, samples, depth));
        if gpu.is_some() {
            set_gpu_particles_available(true);
        }
        Self {
            quads: QuadBatcher::new(device, queue, color, samples, Some(depth), true),
            scratch: Vec::new(),
            color,
            gpu,
        }
    }

    /// Whether this renderer simulates `EmitterParams::gpu` emitters on the GPU
    /// (Full tier); otherwise they run on the CPU.
    pub fn gpu_enabled(&self) -> bool {
        self.gpu.is_some()
    }

    /// The slots of a GPU emitter read back from the GPU (tests, tools; stalls
    /// until the GPU is idle). `None` for CPU emitters or without compute.
    pub fn read_gpu(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        entity: Entity,
    ) -> Option<Vec<GpuParticle>> {
        self.gpu.as_ref()?.read(device, queue, entity)
    }

    /// For an HDR target: skip sRGB encoding (the post chain does it).
    pub fn set_hdr_output(&mut self, on: bool) {
        self.quads.set_hdr_output(on, self.color);
        if let Some(gpu) = &mut self.gpu {
            gpu.set_hdr_output(on, self.color);
        }
    }

    /// Forgets the last frame's particles (no camera to draw them with).
    pub fn clear(&mut self) {
        self.quads.clear();
        if let Some(gpu) = &mut self.gpu {
            gpu.clear();
        }
    }

    /// Collects the quads of every CPU emitter in `world` for the camera
    /// `view_proj` and uploads them; runs the pending steps of the GPU
    /// emitters and readies their draws. Returns the number of particles drawn
    /// (for GPU emitters: their slots, dead ones included).
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu: &GpuMeshes,
        world: &World,
        view_proj: Mat4,
    ) -> usize {
        self.quads.clear();
        // Rows of view-proj are the camera's axes scaled by the projection.
        let right = view_proj.row(0).xyz().normalize_or_zero();
        let up = view_proj.row(1).xyz().normalize_or_zero();
        let w = view_proj.row(3);
        let depth_row = if w.xyz().length_squared() > 1e-8 {
            w
        } else {
            view_proj.row(2)
        };
        for (global, transform, emitter) in world
            .query::<(
                Option<&GlobalTransform>,
                Option<&Transform>,
                &ParticleEmitter,
            )>()
            .iter()
        {
            if emitter.mode != ParticleMode::Cpu || emitter.particles.is_empty() {
                continue;
            }
            let matrix = global.map_or_else(
                || transform.map_or(Mat4::IDENTITY, Transform::matrix),
                |g| g.0,
            );
            let params = &emitter.params;
            let key = BatchKey {
                texture: emitter.texture,
                filter: QuadFilter::Mipmapped,
                blend: params.blend,
            };
            let sort = params.sort && params.blend == QuadBlend::Alpha;
            let local = params.space == SimSpace::Local;
            self.scratch.clear();
            for p in &emitter.particles {
                let pos = if local {
                    matrix.transform_point3(p.pos)
                } else {
                    p.pos
                };
                let t = p.age / p.life;
                let size = p.size * params.size_at(t);
                let (s, c) = p.rot.sin_cos();
                let (x, y) = ((right * c + up * s) * size, (up * c - right * s) * size);
                let origin = pos - (x + y) * 0.5;
                let uv = match params.flip_book {
                    Some(FlipBook { columns, rows }) if columns * rows > 0 => {
                        let frames = columns * rows;
                        let f = ((t * frames as f32) as u32).min(frames - 1);
                        let (u, v) = ((f % columns) as f32, (f / columns) as f32);
                        let (du, dv) = (1.0 / columns as f32, 1.0 / rows as f32);
                        [u * du, (v + 1.0) * dv, (u + 1.0) * du, v * dv]
                    }
                    _ => [0.0, 1.0, 1.0, 0.0],
                };
                let inst = QuadInstance {
                    origin: origin.extend(0.0).to_array(),
                    axis_x: x.extend(0.0).to_array(),
                    axis_y: y.extend(0.0).to_array(),
                    color: {
                        let c = params.life_color(t);
                        std::array::from_fn(|k| c[k] * p.tint[k])
                    },
                    uv,
                };
                if sort {
                    self.scratch.push((depth_row.dot(pos.extend(1.0)), inst));
                } else {
                    self.quads.push(key, inst);
                }
            }
            if sort {
                self.scratch.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
                for (_, inst) in &self.scratch {
                    self.quads.push(key, *inst);
                }
            }
        }
        self.quads.upload(device, queue, gpu, view_proj);
        let mut drawn = self.quads.len();
        if let Some(sim) = &mut self.gpu {
            let mut query = world.query::<(
                Entity,
                Option<&GlobalTransform>,
                Option<&Transform>,
                &ParticleEmitter,
            )>();
            let emitters = query
                .iter()
                .filter(|(.., e)| e.mode == ParticleMode::Gpu)
                .map(|(entity, global, transform, emitter)| {
                    let matrix = global.map_or_else(
                        || transform.map_or(Mat4::IDENTITY, Transform::matrix),
                        |g| g.0,
                    );
                    (entity, matrix, emitter)
                })
                .collect::<Vec<_>>();
            sim.prepare(device, queue, gpu, emitters.into_iter(), view_proj);
            drawn += sim.slots();
        }
        drawn
    }

    /// Draws what the last `prepare` collected into `pass`.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.quads.draw(pass);
        if let Some(gpu) = &self.gpu {
            gpu.draw(pass);
        }
    }

    /// Draw calls `draw` issues.
    pub fn draw_calls(&self) -> usize {
        self.quads.batch_count() + self.gpu.as_ref().map_or(0, GpuSim::draw_calls)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steady(params: EmitterParams) -> ParticleEmitter {
        ParticleEmitter::new(params)
    }

    #[test]
    fn rate_emits_the_expected_count() {
        let mut e = steady(EmitterParams {
            rate: 100.0,
            lifetime: [10.0, 10.0],
            gravity: Vec3::ZERO,
            ..Default::default()
        });
        for _ in 0..60 {
            e.update(1.0 / 60.0, &Mat4::IDENTITY);
        }
        assert!((99..=100).contains(&e.count()), "{}", e.count());
    }

    #[test]
    fn nan_steps_and_huge_caps_are_harmless() {
        let mut e = steady(EmitterParams {
            rate: 100.0,
            max_particles: u32::MAX,
            lifetime: [10.0, 10.0],
            gravity: Vec3::ZERO,
            ..Default::default()
        });
        assert!(e.particles.capacity() <= MAX_RESERVED);
        e.update(f32::NAN, &Mat4::IDENTITY);
        for _ in 0..60 {
            e.update(1.0 / 60.0, &Mat4::IDENTITY);
        }
        assert!((99..=100).contains(&e.count()), "{}", e.count());
    }

    #[test]
    fn particles_die_and_cap_holds() {
        let mut e = steady(EmitterParams {
            rate: 1000.0,
            max_particles: 50,
            lifetime: [0.5, 0.5],
            ..Default::default()
        });
        for _ in 0..30 {
            e.update(1.0 / 60.0, &Mat4::IDENTITY);
            assert!(e.count() <= 50);
        }
        e.playing = false;
        for _ in 0..40 {
            e.update(1.0 / 60.0, &Mat4::IDENTITY);
        }
        assert_eq!(e.count(), 0);
    }

    #[test]
    fn one_shot_burst_finishes() {
        let mut e = steady(EmitterParams {
            rate: 0.0,
            looping: false,
            duration: 1.0,
            bursts: vec![Burst {
                time: 0.0,
                count: 30,
            }],
            lifetime: [0.2, 0.2],
            ..Default::default()
        });
        e.update(0.016, &Mat4::IDENTITY);
        assert_eq!(e.count(), 30);
        assert!(!e.is_done());
        for _ in 0..120 {
            e.update(0.016, &Mat4::IDENTITY);
        }
        assert_eq!(e.count(), 0);
        assert!(e.is_done());
    }

    #[test]
    fn looping_bursts_repeat() {
        let mut e = steady(EmitterParams {
            rate: 0.0,
            duration: 0.5,
            bursts: vec![Burst {
                time: 0.25,
                count: 10,
            }],
            lifetime: [100.0, 100.0],
            ..Default::default()
        });
        for _ in 0..200 {
            e.update(0.01, &Mat4::IDENTITY);
        }
        // 2 s = four cycles.
        assert_eq!(e.count(), 40);
    }

    #[test]
    fn world_space_stays_local_space_follows() {
        let moved = Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0));
        let params = EmitterParams {
            rate: 0.0,
            bursts: vec![Burst {
                time: 0.0,
                count: 1,
            }],
            shape: EmitShape::Point,
            speed: [0.0, 0.0],
            gravity: Vec3::ZERO,
            lifetime: [5.0, 5.0],
            ..Default::default()
        };
        let mut world = steady(params.clone());
        world.update(0.01, &moved);
        assert!((world.particles[0].pos.x - 10.0).abs() < 1e-3);
        let mut local = steady(EmitterParams {
            space: SimSpace::Local,
            ..params
        });
        local.update(0.01, &moved);
        assert!(local.particles[0].pos.length() < 1e-3);
    }

    #[test]
    fn gravity_pulls_and_same_seed_repeats() {
        let params = EmitterParams {
            rate: 50.0,
            seed: 7,
            ..Default::default()
        };
        let run = || {
            let mut e = steady(params.clone());
            for _ in 0..30 {
                e.update(1.0 / 60.0, &Mat4::IDENTITY);
            }
            e.particles.iter().map(|p| p.pos).collect::<Vec<_>>()
        };
        assert_eq!(run(), run());
        let mut e = steady(EmitterParams {
            rate: 0.0,
            bursts: vec![Burst {
                time: 0.0,
                count: 1,
            }],
            speed: [0.0, 0.0],
            ..Default::default()
        });
        e.update(0.1, &Mat4::IDENTITY);
        e.update(0.1, &Mat4::IDENTITY);
        assert!(e.particles[0].pos.y < -0.05);
    }

    #[test]
    fn shapes_emit_inside_their_bounds() {
        let mut rng = Rng::new(3);
        let shapes = [
            EmitShape::Point,
            EmitShape::Sphere {
                radius: 2.0,
                surface: false,
            },
            EmitShape::Hemisphere {
                radius: 2.0,
                surface: true,
            },
            EmitShape::Cone {
                angle_deg: 30.0,
                radius: 1.0,
            },
            EmitShape::Box {
                half_extents: Vec3::new(1.0, 2.0, 3.0),
            },
            EmitShape::Circle {
                radius: 2.0,
                edge: true,
            },
        ];
        for shape in shapes {
            for _ in 0..200 {
                let (pos, dir) = shape.sample(&mut rng);
                assert!((dir.length() - 1.0).abs() < 1e-3, "{shape:?}");
                assert!(pos.abs().max_element() <= 3.001, "{shape:?}");
                if let EmitShape::Cone { angle_deg, .. } = shape {
                    assert!(dir.y >= angle_deg.to_radians().cos() - 1e-4);
                }
            }
        }
    }

    #[test]
    fn curves_interpolate() {
        let p = EmitterParams {
            color_over_life: vec![(0.0, [1.0, 0.0, 0.0, 1.0]), (1.0, [0.0, 0.0, 1.0, 0.0])],
            size_over_life: vec![(0.0, 1.0), (0.5, 3.0), (1.0, 1.0)],
            ..Default::default()
        };
        assert_eq!(p.color_at(0.5), [0.5, 0.0, 0.5, 0.5]);
        assert_eq!(p.color_at(2.0), [0.0, 0.0, 1.0, 0.0]);
        assert_eq!(p.size_at(0.25), 2.0);
        assert_eq!(p.size_at(1.0), 1.0);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn params_round_trip_through_ron() {
        let params = EmitterParams {
            bursts: vec![Burst {
                time: 0.5,
                count: 12,
            }],
            shape: EmitShape::Sphere {
                radius: 1.5,
                surface: true,
            },
            flip_book: Some(FlipBook {
                columns: 4,
                rows: 4,
            }),
            blend: QuadBlend::Additive,
            texture_path: Some("fx/smoke.png".into()),
            ..Default::default()
        };
        let text = ron::to_string(&params).unwrap();
        assert_eq!(ron::from_str::<EmitterParams>(&text).unwrap(), params);
        // Missing fields fall back to defaults.
        let partial: EmitterParams = ron::from_str("(rate: 3.0)").unwrap();
        assert_eq!(partial.rate, 3.0);
        assert_eq!(partial.max_particles, 1000);
    }
}
