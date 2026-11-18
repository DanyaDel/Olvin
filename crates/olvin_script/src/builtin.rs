//! Built-in behaviours: handy in scenes as-is and as templates for your own.

use glam::Vec3;
#[cfg(feature = "input")]
use glam::{EulerRot, Quat};
use olvin_ecs::Entity;

use crate::{Behaviour, FromParams, ParamSpec, Params, ScriptCtx, ScriptRegistry};

pub(crate) fn register_all(r: &mut ScriptRegistry) {
    r.register_if_absent::<Rotator>("Rotator");
    r.register_if_absent::<Oscillator>("Oscillator");
    r.register_if_absent::<Mover>("Mover");
    r.register_if_absent::<FollowTarget>("FollowTarget");
    r.register_if_absent::<DestroyAfter>("DestroyAfter");
    #[cfg(feature = "input")]
    r.register_if_absent::<FreeFlyCamera>("FreeFlyCamera");
    #[cfg(all(feature = "input", feature = "physics"))]
    r.register_if_absent::<SimplePlayerController>("SimplePlayerController");
}

/// Spins around its own axes, degrees per second (Euler YXZ).
#[derive(Clone, Debug, PartialEq)]
pub struct Rotator {
    pub speed_deg: Vec3,
}

impl FromParams for Rotator {
    fn from_params(p: &Params) -> Self {
        Self {
            speed_deg: p.vec3("speed_deg", Vec3::new(0.0, 90.0, 0.0)),
        }
    }

    fn schema() -> Vec<ParamSpec> {
        vec![ParamSpec::vec3(
            "speed_deg",
            Vec3::new(0.0, 90.0, 0.0),
            "degrees per second around X, Y, Z",
        )]
    }
}

impl Behaviour for Rotator {
    fn update(&mut self, ctx: &mut ScriptCtx<'_>) {
        let d = self.speed_deg * ctx.dt();
        ctx.rotate_euler_deg(d);
    }
}

/// Swings back and forth around where it started: `offset = amplitude * sin(2π f t)`.
#[derive(Clone, Debug, PartialEq)]
pub struct Oscillator {
    pub amplitude: Vec3,
    pub frequency: f32,
    origin: Option<Vec3>,
    t: f32,
}

impl Oscillator {
    pub fn new(amplitude: Vec3, frequency: f32) -> Self {
        Self {
            amplitude,
            frequency,
            origin: None,
            t: 0.0,
        }
    }
}

impl FromParams for Oscillator {
    fn from_params(p: &Params) -> Self {
        Self::new(
            p.vec3("amplitude", Vec3::new(0.0, 1.0, 0.0)),
            p.f32("frequency", 0.5),
        )
    }

    fn schema() -> Vec<ParamSpec> {
        vec![
            ParamSpec::vec3("amplitude", Vec3::Y, "metres along each axis"),
            ParamSpec::float("frequency", 0.5, "swings per second"),
        ]
    }
}

impl Behaviour for Oscillator {
    fn start(&mut self, ctx: &mut ScriptCtx<'_>) {
        self.origin = Some(ctx.position());
    }

    fn update(&mut self, ctx: &mut ScriptCtx<'_>) {
        self.t += ctx.dt();
        let origin = self.origin.unwrap_or_default();
        let s = (self.t * self.frequency * std::f32::consts::TAU).sin();
        ctx.set_position(origin + self.amplitude * s);
    }
}

/// Moves at a constant velocity (m/s), in its own axes when `local`.
#[derive(Clone, Debug, PartialEq)]
pub struct Mover {
    pub velocity: Vec3,
    pub local: bool,
}

impl FromParams for Mover {
    fn from_params(p: &Params) -> Self {
        Self {
            velocity: p.vec3("velocity", Vec3::new(0.0, 0.0, -1.0)),
            local: p.bool("local", false),
        }
    }

    fn schema() -> Vec<ParamSpec> {
        vec![
            ParamSpec::vec3("velocity", Vec3::NEG_Z, "metres per second"),
            ParamSpec::bool("local", false, "along the object's own axes"),
        ]
    }
}

impl Behaviour for Mover {
    fn update(&mut self, ctx: &mut ScriptCtx<'_>) {
        let d = self.velocity * ctx.dt();
        if self.local {
            ctx.translate_local(d);
        } else {
            ctx.translate(d);
        }
    }
}

/// Follows another entity at an offset, optionally smoothed and looking at it.
#[derive(Clone, Debug, PartialEq)]
pub struct FollowTarget {
    pub target: Option<Entity>,
    pub offset: Vec3,
    /// Seconds to cover most of the distance (0 = snap).
    pub smoothing: f32,
    pub look_at: bool,
}

impl FromParams for FollowTarget {
    fn from_params(p: &Params) -> Self {
        Self {
            target: p.entity("target"),
            offset: p.vec3("offset", Vec3::new(0.0, 3.0, 6.0)),
            smoothing: p.f32("smoothing", 0.15),
            look_at: p.bool("look_at", true),
        }
    }

    fn schema() -> Vec<ParamSpec> {
        vec![
            ParamSpec::entity("target", "entity to follow"),
            ParamSpec::vec3("offset", Vec3::new(0.0, 3.0, 6.0), "world-space offset"),
            ParamSpec::float("smoothing", 0.15, "seconds of lag, 0 snaps"),
            ParamSpec::bool("look_at", true, "turn towards the target"),
        ]
    }
}

impl Behaviour for FollowTarget {
    fn update(&mut self, ctx: &mut ScriptCtx<'_>) {
        let Some(target) = self.target.filter(|t| ctx.world().contains(*t)) else {
            return;
        };
        let goal_pos = ctx.world_position_of(target);
        let goal = goal_pos + self.offset;
        let k = if self.smoothing <= 0.0 {
            1.0
        } else {
            1.0 - (-ctx.dt() / self.smoothing * 3.0).exp()
        };
        let p = ctx.position().lerp(goal, k);
        ctx.set_position(p);
        if self.look_at {
            ctx.look_at(goal_pos, Vec3::Y);
        }
    }
}

/// Despawns its entity after `seconds`.
#[derive(Clone, Debug, PartialEq)]
pub struct DestroyAfter {
    pub seconds: f32,
}

impl FromParams for DestroyAfter {
    fn from_params(p: &Params) -> Self {
        Self {
            seconds: p.f32("seconds", 5.0),
        }
    }

    fn schema() -> Vec<ParamSpec> {
        vec![ParamSpec::float("seconds", 5.0, "lifetime")]
    }
}

impl Behaviour for DestroyAfter {
    fn update(&mut self, ctx: &mut ScriptCtx<'_>) {
        self.seconds -= ctx.dt();
        if self.seconds <= 0.0 {
            ctx.destroy_self();
        }
    }
}

/// Editor-style flying camera: `Move` (WASD), `Ascend` (Space/Ctrl), `Look`
/// (mouse, locks the cursor while `lock_cursor`), `Sprint` multiplies speed.
#[cfg(feature = "input")]
#[derive(Clone, Debug, PartialEq)]
pub struct FreeFlyCamera {
    pub speed: f32,
    pub sprint_multiplier: f32,
    /// Degrees per `Look` unit (mouse pixel).
    pub sensitivity: f32,
    pub lock_cursor: bool,
    yaw: f32,
    pitch: f32,
}

#[cfg(feature = "input")]
impl FromParams for FreeFlyCamera {
    fn from_params(p: &Params) -> Self {
        Self {
            speed: p.f32("speed", 5.0),
            sprint_multiplier: p.f32("sprint_multiplier", 3.0),
            sensitivity: p.f32("sensitivity", 0.12),
            lock_cursor: p.bool("lock_cursor", true),
            yaw: 0.0,
            pitch: 0.0,
        }
    }

    fn schema() -> Vec<ParamSpec> {
        vec![
            ParamSpec::float("speed", 5.0, "metres per second"),
            ParamSpec::float("sprint_multiplier", 3.0, "with Sprint held"),
            ParamSpec::float("sensitivity", 0.12, "degrees per mouse pixel"),
            ParamSpec::bool("lock_cursor", true, "grab the cursor for mouse look"),
        ]
    }
}

#[cfg(feature = "input")]
impl Behaviour for FreeFlyCamera {
    fn start(&mut self, ctx: &mut ScriptCtx<'_>) {
        let (yaw, pitch, _) = ctx.rotation().to_euler(EulerRot::YXZ);
        self.yaw = yaw;
        self.pitch = pitch;
        if self.lock_cursor
            && let Some(input) = ctx.input_mut()
        {
            input.lock_cursor(true);
        }
    }

    fn update(&mut self, ctx: &mut ScriptCtx<'_>) {
        let input = ctx.input();
        let look = input.axis2d("Look");
        let mv = input.axis2d("Move");
        let up = input.value("Ascend");
        let sprint = input.pressed("Sprint");
        self.yaw -= (look.x * self.sensitivity).to_radians();
        self.pitch = (self.pitch + (look.y * self.sensitivity).to_radians())
            .clamp(-89f32.to_radians(), 89f32.to_radians());
        let rotation = Quat::from_euler(EulerRot::YXZ, self.yaw, self.pitch, 0.0);
        ctx.set_rotation(rotation);
        let speed = self.speed * if sprint { self.sprint_multiplier } else { 1.0 };
        let local = Vec3::new(mv.x, 0.0, -mv.y);
        let delta = (rotation * local + Vec3::Y * up) * speed * ctx.dt();
        ctx.translate(delta);
    }
}

/// First-person walker on a Character Controller: `Move` relative to where
/// it faces, `Look` turns the body (yaw) and the `camera` child (pitch),
/// `Jump` when grounded, `Sprint` runs. Needs a `CharacterControllerDesc`.
#[cfg(all(feature = "input", feature = "physics"))]
#[derive(Clone, Debug, PartialEq)]
pub struct SimplePlayerController {
    pub speed: f32,
    pub sprint_speed: f32,
    pub jump_height: f32,
    pub sensitivity: f32,
    pub camera: Option<Entity>,
    yaw: f32,
    pitch: f32,
}

#[cfg(all(feature = "input", feature = "physics"))]
impl FromParams for SimplePlayerController {
    fn from_params(p: &Params) -> Self {
        Self {
            speed: p.f32("speed", 4.0),
            sprint_speed: p.f32("sprint_speed", 7.0),
            jump_height: p.f32("jump_height", 1.2),
            sensitivity: p.f32("sensitivity", 0.12),
            camera: p.entity("camera"),
            yaw: 0.0,
            pitch: 0.0,
        }
    }

    fn schema() -> Vec<ParamSpec> {
        vec![
            ParamSpec::float("speed", 4.0, "walk speed, m/s"),
            ParamSpec::float("sprint_speed", 7.0, "with Sprint held"),
            ParamSpec::float("jump_height", 1.2, "metres"),
            ParamSpec::float("sensitivity", 0.12, "degrees per mouse pixel"),
            ParamSpec::entity("camera", "child camera that pitches (optional)"),
        ]
    }
}

#[cfg(all(feature = "input", feature = "physics"))]
impl Behaviour for SimplePlayerController {
    fn start(&mut self, ctx: &mut ScriptCtx<'_>) {
        let (yaw, _, _) = ctx.rotation().to_euler(EulerRot::YXZ);
        self.yaw = yaw;
        if ctx
            .component::<olvin_scene::file::CharacterControllerDesc>()
            .is_none()
        {
            ctx.warn("needs a Character Controller component");
        }
    }

    fn update(&mut self, ctx: &mut ScriptCtx<'_>) {
        let input = ctx.input();
        let look = input.axis2d("Look");
        let mv = input.axis2d("Move");
        let jump = input.just_pressed("Jump");
        let speed = if input.pressed("Sprint") {
            self.sprint_speed
        } else {
            self.speed
        };
        self.yaw -= (look.x * self.sensitivity).to_radians();
        self.pitch = (self.pitch + (look.y * self.sensitivity).to_radians())
            .clamp(-85f32.to_radians(), 85f32.to_radians());
        let facing = Quat::from_rotation_y(self.yaw);
        ctx.set_rotation(facing);
        if let Some(cam) = self.camera
            && let Some(mut t) = ctx.get_mut::<olvin_scene::Transform>(cam)
        {
            t.rotation = Quat::from_rotation_x(self.pitch);
        }
        let gravity = ctx.physics().map_or(9.81, |p| -p.gravity().y);
        let horizontal = facing * Vec3::new(mv.x, 0.0, -mv.y) * speed;
        if let Some(mut motor) = ctx.motor() {
            motor.velocity.x = horizontal.x;
            motor.velocity.z = horizontal.z;
            if jump && motor.grounded {
                motor.velocity.y = (2.0 * gravity.max(0.0) * self.jump_height).sqrt();
            }
        }
    }
}
