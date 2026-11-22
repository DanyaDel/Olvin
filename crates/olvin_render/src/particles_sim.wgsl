// GPU particle simulation (Full tier). One thread per particle slot. Slots
// form a ring: a step's births take the `counts.y` slots from `counts.x`, so
// no free list is needed (the oldest slots are overwritten when more than
// `max` are alive). A zeroed slot is dead (age >= life).

struct Particle {
    pos: vec3<f32>,
    age: f32,
    vel: vec3<f32>,
    life: f32,
    rot: f32,
    spin: f32,
    size: f32,
    pad: f32,
    tint: vec4<f32>,
};

struct Sim {
    matrix: mat4x4<f32>,
    // xyz gravity, w = fraction of velocity kept this step.
    gravity_keep: vec4<f32>,
    // x = dt.
    step: vec4<f32>,
    // x = first birth slot, y = births, z = seed, w = slot count.
    counts: vec4<u32>,
    // x = shape (0 point, 1 sphere, 2 hemisphere, 3 cone, 4 box, 5 circle),
    // y = radius, z = cos(cone angle), w = 1 for surface / edge only.
    shape: vec4<f32>,
    // xyz = box half extents, w = 1: local space (births stay in local coordinates).
    half: vec4<f32>,
    life_speed: vec4<f32>,
    size_rot: vec4<f32>,
    spin: vec4<f32>,
    color_a: vec4<f32>,
    color_b: vec4<f32>,
};

@group(0) @binding(0) var<uniform> sim: Sim;
@group(0) @binding(1) var<storage, read_write> particles: array<Particle>;

const TAU: f32 = 6.28318530718;

fn pcg(v: u32) -> u32 {
    let state = v * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn rand(s: ptr<function, u32>) -> f32 {
    *s = pcg(*s);
    return f32(*s >> 8u) / 16777216.0;
}

fn range2(s: ptr<function, u32>, r: vec2<f32>) -> f32 {
    return r.x + (r.y - r.x) * rand(s);
}

fn on_sphere(s: ptr<function, u32>) -> vec3<f32> {
    let z = rand(s) * 2.0 - 1.0;
    let a = rand(s) * TAU;
    let r = sqrt(max(1.0 - z * z, 0.0));
    return vec3<f32>(r * cos(a), z, r * sin(a));
}

fn in_disc(s: ptr<function, u32>, radius: f32) -> vec2<f32> {
    let r = radius * sqrt(rand(s));
    let a = rand(s) * TAU;
    return vec2<f32>(r * cos(a), r * sin(a));
}

struct Born {
    pos: vec3<f32>,
    dir: vec3<f32>,
};

fn sample_shape(s: ptr<function, u32>) -> Born {
    var b: Born;
    let kind = u32(sim.shape.x + 0.5);
    let radius = sim.shape.y;
    let hollow = sim.shape.w > 0.5;
    if (kind == 1u || kind == 2u) {
        var dir = on_sphere(s);
        if (kind == 2u) {
            dir.y = abs(dir.y);
        }
        var r = radius;
        if (!hollow) {
            r = radius * pow(rand(s), 1.0 / 3.0);
        }
        b.pos = dir * r;
        b.dir = dir;
    } else if (kind == 3u) {
        let d = in_disc(s, radius);
        let c = 1.0 + (sim.shape.z - 1.0) * rand(s);
        let sn = sqrt(max(1.0 - c * c, 0.0));
        let a = rand(s) * TAU;
        b.pos = vec3<f32>(d.x, 0.0, d.y);
        b.dir = vec3<f32>(sn * cos(a), c, sn * sin(a));
    } else if (kind == 4u) {
        let p = vec3<f32>(rand(s), rand(s), rand(s)) * 2.0 - vec3<f32>(1.0);
        b.pos = p * sim.half.xyz;
        b.dir = vec3<f32>(0.0, 1.0, 0.0);
    } else if (kind == 5u) {
        var xz: vec2<f32>;
        if (hollow) {
            let a = rand(s) * TAU;
            xz = vec2<f32>(radius * cos(a), radius * sin(a));
        } else {
            xz = in_disc(s, radius);
        }
        b.pos = vec3<f32>(xz.x, 0.0, xz.y);
        var d = vec3<f32>(xz.x, 0.0, xz.y);
        if (dot(d, d) > 1e-12) {
            d = normalize(d);
        } else {
            d = vec3<f32>(1.0, 0.0, 0.0);
        }
        b.dir = d;
    } else {
        b.pos = vec3<f32>(0.0);
        b.dir = on_sphere(s);
    }
    return b;
}

@compute @workgroup_size(64)
fn cs_sim(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    let slots = sim.counts.w;
    if (i >= slots) {
        return;
    }
    let dt = sim.step.x;
    var p = particles[i];
    let k = (i + slots - sim.counts.x) % slots;
    if (k < sim.counts.y) {
        var s = pcg(i * 9781u + sim.counts.z * 6271u + 1u);
        s = pcg(s);
        let born = sample_shape(&s);
        let speed = range2(&s, sim.life_speed.zw);
        let life = max(range2(&s, sim.life_speed.xy), 0.001);
        var pos = born.pos;
        var vel = born.dir * speed;
        if (sim.half.w < 0.5) {
            pos = (sim.matrix * vec4<f32>(born.pos, 1.0)).xyz;
            let world_dir = (sim.matrix * vec4<f32>(born.dir, 0.0)).xyz;
            if (dot(world_dir, world_dir) > 1e-12) {
                vel = normalize(world_dir) * speed;
            } else {
                vel = vec3<f32>(0.0);
            }
        }
        // Born somewhere inside the step, as on the CPU.
        let early = dt * (1.0 - (f32(k) + 0.5) / f32(sim.counts.y));
        p.pos = pos + vel * early;
        p.vel = vel;
        p.age = min(early, life * 0.999);
        p.life = life;
        p.rot = range2(&s, sim.size_rot.zw);
        p.spin = range2(&s, sim.spin.xy);
        p.size = range2(&s, sim.size_rot.xy);
        p.pad = 0.0;
        p.tint = mix(sim.color_a, sim.color_b, rand(&s));
        particles[i] = p;
        return;
    }
    if (p.age < p.life) {
        p.age = p.age + dt;
        if (p.age >= p.life) {
            p.age = p.life;
        } else {
            p.vel = (p.vel + sim.gravity_keep.xyz * dt) * sim.gravity_keep.w;
            p.pos = p.pos + p.vel * dt;
            p.rot = p.rot + p.spin * dt;
        }
        particles[i] = p;
    }
}
