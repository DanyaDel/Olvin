//! Scene particle emitters (`EntityDesc::particles`) as engine components.

use olvin_render::{
    Burst, EmitShape, EmitterParams, FlipBook, ParticleEmitter, QuadBlend, SimSpace, Textures,
};
use olvin_scene::file::{BlendDesc, ParticleDesc, ShapeDesc, SpaceDesc};

use crate::render_assets::RenderAssets;

/// The renderer's parameters for a scene description (degrees to radians, the
/// texture path stays in `texture_path` for the loader).
pub fn emitter_params(d: &ParticleDesc) -> EmitterParams {
    EmitterParams {
        max_particles: d.max_particles,
        rate: d.rate,
        bursts: d
            .bursts
            .iter()
            .map(|b| Burst {
                time: b.time,
                count: b.count,
            })
            .collect(),
        duration: d.duration,
        looping: d.looping,
        shape: match d.shape {
            ShapeDesc::Point => EmitShape::Point,
            ShapeDesc::Sphere { radius, surface } => EmitShape::Sphere { radius, surface },
            ShapeDesc::Hemisphere { radius, surface } => EmitShape::Hemisphere { radius, surface },
            ShapeDesc::Cone { angle_deg, radius } => EmitShape::Cone { angle_deg, radius },
            ShapeDesc::Box { half_extents } => EmitShape::Box { half_extents },
            ShapeDesc::Circle { radius, edge } => EmitShape::Circle { radius, edge },
        },
        lifetime: d.lifetime,
        speed: d.speed,
        size: d.size,
        rotation: d.rotation.map(f32::to_radians),
        spin: d.spin.map(f32::to_radians),
        gravity: d.gravity,
        drag: d.drag,
        space: match d.space {
            SpaceDesc::World => SimSpace::World,
            SpaceDesc::Local => SimSpace::Local,
        },
        color: d.color,
        color_max: d.color_max,
        color_over_life: d.color_over_life.clone(),
        size_over_life: d.size_over_life.clone(),
        blend: match d.blend {
            BlendDesc::Alpha => QuadBlend::Alpha,
            BlendDesc::Additive => QuadBlend::Additive,
        },
        flip_book: d
            .flip_book
            .map(|(columns, rows)| FlipBook { columns, rows }),
        sort: d.sort,
        texture_path: d.texture.clone(),
        seed: d.seed,
        prewarm: d.prewarm,
        gpu: d.gpu,
    }
}

/// The emitter component for `d`, its sprite resolved against the project's
/// `assets/` (a missing file falls back to the soft circle).
pub fn emitter(
    d: &ParticleDesc,
    assets: &mut RenderAssets,
    textures: &mut Textures,
) -> ParticleEmitter {
    let mut e = ParticleEmitter::new(emitter_params(d));
    e.texture = Some(assets.particle_texture(d.texture.as_deref(), textures));
    e.playing = d.play_on_awake;
    e
}

/// [`emitter`] with the registries of `res` (a plain emitter without
/// texture where they are missing, as in a headless game).
pub fn emitter_in(res: &olvin_core::Resources, d: &ParticleDesc) -> ParticleEmitter {
    match (res.get_mut::<RenderAssets>(), res.get_mut::<Textures>()) {
        (Some(mut assets), Some(mut textures)) => emitter(d, &mut assets, &mut textures),
        _ => {
            let mut e = ParticleEmitter::new(emitter_params(d));
            e.playing = d.play_on_awake;
            e
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_converts_and_keeps_its_numbers() {
        for (name, make) in ParticleDesc::PRESETS {
            let d = make();
            let p = emitter_params(&d);
            assert_eq!(p.rate, d.rate, "{name}");
            assert_eq!(p.max_particles, d.max_particles);
            assert_eq!(p.color_over_life, d.color_over_life);
            assert!((p.rotation[1] - d.rotation[1].to_radians()).abs() < 1e-6);
        }
        assert!(
            emitter_params(&ParticleDesc {
                gpu: true,
                ..Default::default()
            })
            .gpu
        );
    }
}
