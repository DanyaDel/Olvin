//! Particle emitters in scene files: [`ParticleDesc`] is the serde mirror of the
//! renderer's `EmitterParams` plus the scene-level switches (`gpu`,
//! `play_on_awake`, texture path). The loader (`olvin::particles`) turns it into
//! a `ParticleEmitter`; the editor edits it as part of the scene (so it is
//! undoable and saved like everything else).
//!
//! Angles are degrees here (the renderer works in radians). Every field has a
//! serde default, so a file may list only what differs from [`ParticleDesc::default`].

use glam::Vec3;
use serde::{Deserialize, Serialize};

/// Where particles are born, in the emitter's local space.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ShapeDesc {
    /// One point, any direction.
    Point,
    /// Inside a sphere (or only on its surface), flying outwards.
    Sphere { radius: f32, surface: bool },
    /// The upper (+Y) half of a sphere.
    Hemisphere { radius: f32, surface: bool },
    /// A disc in XZ spraying into a cone around +Y.
    Cone { angle_deg: f32, radius: f32 },
    /// Inside a box, flying along +Y.
    Box { half_extents: Vec3 },
    /// A disc in XZ (or only its edge), flying outwards in the plane.
    Circle { radius: f32, edge: bool },
}

/// Particles emitted at one moment of the emitter's cycle.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BurstDesc {
    /// Seconds since the cycle started.
    pub time: f32,
    pub count: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlendDesc {
    /// Straight alpha.
    #[default]
    Alpha,
    /// Adds colour × alpha: fire, sparks, glow.
    Additive,
}

/// Whether particles follow the emitter once born.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpaceDesc {
    /// Left behind in the world as the emitter moves.
    #[default]
    World,
    /// Carried along with the emitter.
    Local,
}

/// A particle system on a scene object. Ranges are `[min, max]`, picked
/// uniformly per particle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ParticleDesc {
    /// Simulate on the GPU (compute) where the device supports it (the Full
    /// tier); elsewhere it quietly runs on the CPU.
    pub gpu: bool,
    /// Start emitting when the object spawns (otherwise a script plays it).
    pub play_on_awake: bool,
    /// Cap on live particles.
    pub max_particles: u32,
    /// Particles per second while emitting.
    pub rate: f32,
    pub bursts: Vec<BurstDesc>,
    /// Length of one emission cycle, seconds.
    pub duration: f32,
    pub looping: bool,
    /// Start as if one full lifetime had already been simulated.
    pub prewarm: bool,
    pub shape: ShapeDesc,
    pub lifetime: [f32; 2],
    /// Initial speed along the emission direction.
    pub speed: [f32; 2],
    /// Initial size (quad edge, world units).
    pub size: [f32; 2],
    /// Initial rotation, degrees.
    pub rotation: [f32; 2],
    /// Rotation speed, degrees per second.
    pub spin: [f32; 2],
    pub gravity: Vec3,
    /// Fraction of velocity lost per second.
    pub drag: f32,
    pub space: SpaceDesc,
    /// Linear RGBA start colour; with `color_max` each particle picks a random
    /// colour between the two.
    pub color: [f32; 4],
    pub color_max: Option<[f32; 4]>,
    /// `(life fraction, RGBA)`, ascending; empty = white.
    pub color_over_life: Vec<(f32, [f32; 4])>,
    /// `(life fraction, size multiplier)`, ascending; empty = constant.
    pub size_over_life: Vec<(f32, f32)>,
    pub blend: BlendDesc,
    /// Texture, relative to the project's `assets/`; `None` (or a file that
    /// cannot be read): the built-in soft circle.
    pub texture: Option<String>,
    /// Texture cut into `(columns, rows)` frames played over a particle's life.
    pub flip_book: Option<(u32, u32)>,
    /// Draw far particles first (alpha blending, CPU only).
    pub sort: bool,
    pub seed: u32,
}

impl Default for ParticleDesc {
    fn default() -> Self {
        Self {
            gpu: false,
            play_on_awake: true,
            max_particles: 1000,
            rate: 20.0,
            bursts: Vec::new(),
            duration: 5.0,
            looping: true,
            prewarm: false,
            shape: ShapeDesc::Cone {
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
            space: SpaceDesc::World,
            color: [1.0; 4],
            color_max: None,
            color_over_life: vec![(0.0, [1.0; 4]), (1.0, [1.0, 1.0, 1.0, 0.0])],
            size_over_life: Vec::new(),
            blend: BlendDesc::Alpha,
            texture: None,
            flip_book: None,
            sort: true,
            seed: 1,
        }
    }
}

/// A preset's menu name and constructor.
pub type Preset = (&'static str, fn() -> ParticleDesc);

/// Presets for the editor's menu and for code (scripts, library packs).
impl ParticleDesc {
    /// Names and constructors of the presets, in menu order.
    pub const PRESETS: &'static [Preset] = &[
        ("Fire", Self::fire),
        ("Smoke", Self::smoke),
        ("Sparks", Self::sparks),
        ("Dust", Self::dust),
        ("Rain", Self::rain),
        ("Snow", Self::snow),
        ("Explosion burst", Self::explosion),
        ("Magic glow", Self::magic_glow),
    ];

    /// A flame: additive, rises and shrinks, orange to dark red.
    pub fn fire() -> Self {
        Self {
            max_particles: 500,
            rate: 45.0,
            lifetime: [0.6, 1.2],
            speed: [0.6, 1.4],
            size: [0.35, 0.6],
            shape: ShapeDesc::Cone {
                angle_deg: 12.0,
                radius: 0.15,
            },
            gravity: Vec3::new(0.0, 1.2, 0.0),
            drag: 0.3,
            rotation: [0.0, 360.0],
            blend: BlendDesc::Additive,
            sort: false,
            color_over_life: vec![
                (0.0, [1.0, 0.85, 0.35, 0.0]),
                (0.12, [1.0, 0.7, 0.2, 0.9]),
                (0.6, [1.0, 0.3, 0.05, 0.6]),
                (1.0, [0.25, 0.05, 0.0, 0.0]),
            ],
            size_over_life: vec![(0.0, 0.6), (0.3, 1.0), (1.0, 0.2)],
            ..Self::default()
        }
    }

    /// Slow grey puffs that grow and fade.
    pub fn smoke() -> Self {
        Self {
            max_particles: 400,
            rate: 12.0,
            lifetime: [2.5, 4.0],
            speed: [0.3, 0.7],
            size: [0.5, 0.9],
            shape: ShapeDesc::Cone {
                angle_deg: 20.0,
                radius: 0.15,
            },
            gravity: Vec3::new(0.0, 0.35, 0.0),
            drag: 0.2,
            rotation: [0.0, 360.0],
            spin: [-30.0, 30.0],
            color_over_life: vec![
                (0.0, [0.4, 0.4, 0.4, 0.0]),
                (0.15, [0.4, 0.4, 0.4, 0.5]),
                (1.0, [0.15, 0.15, 0.15, 0.0]),
            ],
            size_over_life: vec![(0.0, 0.5), (1.0, 2.2)],
            ..Self::default()
        }
    }

    /// Hot sparks thrown up and pulled down.
    pub fn sparks() -> Self {
        Self {
            max_particles: 600,
            rate: 60.0,
            lifetime: [0.4, 1.0],
            speed: [3.0, 7.0],
            size: [0.03, 0.06],
            shape: ShapeDesc::Cone {
                angle_deg: 35.0,
                radius: 0.02,
            },
            drag: 0.4,
            blend: BlendDesc::Additive,
            sort: false,
            color: [1.0, 0.75, 0.3, 1.0],
            color_over_life: vec![
                (0.0, [1.0; 4]),
                (0.7, [1.0, 0.6, 0.2, 1.0]),
                (1.0, [1.0, 0.2, 0.0, 0.0]),
            ],
            ..Self::default()
        }
    }

    /// Specks drifting in a room-sized box.
    pub fn dust() -> Self {
        Self {
            max_particles: 600,
            rate: 8.0,
            lifetime: [3.0, 6.0],
            speed: [0.0, 0.0],
            size: [0.05, 0.12],
            shape: ShapeDesc::Box {
                half_extents: Vec3::new(3.0, 1.0, 3.0),
            },
            gravity: Vec3::new(0.0, -0.03, 0.0),
            drag: 0.5,
            color: [0.85, 0.8, 0.7, 0.35],
            color_over_life: vec![
                (0.0, [1.0, 1.0, 1.0, 0.0]),
                (0.2, [1.0; 4]),
                (0.8, [1.0; 4]),
                (1.0, [1.0, 1.0, 1.0, 0.0]),
            ],
            ..Self::default()
        }
    }

    /// Drops falling from a wide box; put the emitter above the scene.
    pub fn rain() -> Self {
        Self {
            max_particles: 3000,
            rate: 600.0,
            lifetime: [0.9, 1.2],
            speed: [0.0, 0.0],
            size: [0.015, 0.03],
            shape: ShapeDesc::Box {
                half_extents: Vec3::new(8.0, 0.05, 8.0),
            },
            gravity: Vec3::new(0.0, -22.0, 0.0),
            color: [0.7, 0.8, 1.0, 0.55],
            color_over_life: vec![(0.0, [1.0; 4]), (1.0, [1.0; 4])],
            sort: false,
            ..Self::default()
        }
    }

    /// Flakes drifting down from a wide box.
    pub fn snow() -> Self {
        Self {
            max_particles: 2500,
            rate: 200.0,
            lifetime: [7.0, 10.0],
            speed: [0.0, 0.0],
            size: [0.04, 0.09],
            shape: ShapeDesc::Box {
                half_extents: Vec3::new(8.0, 0.1, 8.0),
            },
            gravity: Vec3::new(0.0, -0.6, 0.0),
            drag: 0.1,
            color: [1.0, 1.0, 1.0, 0.9],
            color_over_life: vec![
                (0.0, [1.0, 1.0, 1.0, 0.0]),
                (0.05, [1.0; 4]),
                (0.9, [1.0; 4]),
                (1.0, [1.0, 1.0, 1.0, 0.0]),
            ],
            sort: false,
            ..Self::default()
        }
    }

    /// One-shot fireball: a burst thrown out in all directions.
    pub fn explosion() -> Self {
        Self {
            max_particles: 300,
            rate: 0.0,
            bursts: vec![BurstDesc {
                time: 0.0,
                count: 120,
            }],
            duration: 1.0,
            looping: false,
            lifetime: [0.5, 1.1],
            speed: [2.0, 7.0],
            size: [0.25, 0.55],
            shape: ShapeDesc::Sphere {
                radius: 0.2,
                surface: false,
            },
            gravity: Vec3::new(0.0, -1.0, 0.0),
            drag: 2.0,
            rotation: [0.0, 360.0],
            blend: BlendDesc::Additive,
            sort: false,
            color_over_life: vec![
                (0.0, [1.0, 0.9, 0.5, 1.0]),
                (0.35, [1.0, 0.45, 0.1, 0.8]),
                (1.0, [0.3, 0.05, 0.0, 0.0]),
            ],
            size_over_life: vec![(0.0, 0.5), (0.4, 1.0), (1.0, 1.4)],
            ..Self::default()
        }
    }

    /// Soft violet-to-cyan motes orbiting out of a shell.
    pub fn magic_glow() -> Self {
        Self {
            max_particles: 400,
            rate: 25.0,
            lifetime: [1.5, 2.5],
            speed: [0.1, 0.4],
            size: [0.08, 0.18],
            shape: ShapeDesc::Sphere {
                radius: 0.5,
                surface: true,
            },
            gravity: Vec3::new(0.0, 0.2, 0.0),
            drag: 0.3,
            blend: BlendDesc::Additive,
            sort: false,
            color: [0.6, 0.4, 1.0, 1.0],
            color_max: Some([0.3, 0.9, 1.0, 1.0]),
            color_over_life: vec![
                (0.0, [1.0, 1.0, 1.0, 0.0]),
                (0.25, [1.0; 4]),
                (1.0, [1.0, 1.0, 1.0, 0.0]),
            ],
            size_over_life: vec![(0.0, 0.0), (0.2, 1.0), (1.0, 0.0)],
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_round_trip_and_partial_files_load() {
        for (name, make) in ParticleDesc::PRESETS {
            let desc = make();
            let text = ron::to_string(&desc).unwrap();
            assert_eq!(
                ron::from_str::<ParticleDesc>(&text).unwrap(),
                desc,
                "{name}"
            );
        }
        let partial: ParticleDesc = ron::from_str("(rate: 3.0, gpu: true)").unwrap();
        assert!(partial.gpu && partial.rate == 3.0 && partial.max_particles == 1000);
    }

    #[test]
    fn presets_are_distinct() {
        let all: Vec<_> = ParticleDesc::PRESETS.iter().map(|(_, f)| f()).collect();
        for (i, a) in all.iter().enumerate() {
            assert_ne!(*a, ParticleDesc::default());
            assert!(all[i + 1..].iter().all(|b| a != b));
        }
    }
}
