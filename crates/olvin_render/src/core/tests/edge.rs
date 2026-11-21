//! Hostile input: nothing may panic or fail wgpu validation, and the sane
//! objects next to the bad ones must still draw.

use super::*;
use crate::Vertex;

#[test]
fn bad_meshes_transforms_and_lights_do_not_break_the_frame() {
    for (editor_like, compat) in [(false, false), (true, false), (false, true)] {
        let Some(mut h) = Harness::new(editor_like) else {
            return;
        };
        if compat {
            h.core.set_tier(RenderTier::Compat);
        }
        let good = h.meshes.add(MeshData::cube(1.0));
        let empty = h.meshes.add(MeshData {
            vertices: Vec::new(),
            indices: Vec::new(),
        });
        let stray = h.meshes.add(MeshData {
            vertices: vec![Vertex::new([0.0; 3], [0.0, 1.0, 0.0])],
            indices: vec![0, 1, 7],
        });
        let nan = h.meshes.add(MeshData {
            vertices: vec![Vertex::new([f32::NAN; 3], [f32::NAN; 3]); 3],
            indices: vec![0, 1, 2],
        });
        let mut world = World::new();
        world.spawn(top_down_camera(8.0));
        let white = Color::WHITE;
        world.spawn((
            Transform::IDENTITY,
            MeshRenderer::new(good, Color::rgb(1.0, 0.0, 0.0)),
        ));
        for mesh in [empty, stray, nan] {
            world.spawn((Transform::IDENTITY, MeshRenderer::new(mesh, white)));
        }
        let bad = [
            Transform::from_xyz(f32::NAN, 0.0, 0.0),
            Transform::from_xyz(f32::INFINITY, 0.0, 0.0),
            Transform::IDENTITY.with_scale(Vec3::ZERO),
            Transform::IDENTITY.with_scale(Vec3::splat(f32::NAN)),
            Transform::IDENTITY.with_scale(Vec3::splat(1e30)),
        ];
        for t in bad {
            world.spawn((t, MeshRenderer::new(good, white)));
        }
        // A dangling material handle and NaN lights.
        let mut ghost = h.materials.add(Material::default());
        ghost.0 += 50;
        world.spawn((
            Transform::IDENTITY,
            MeshRenderer::new(good, white).with_material(ghost),
        ));
        let mut l = Light::point([f32::NAN; 3], f32::NAN, f32::NAN);
        l.spot_angle_deg = f32::NAN;
        world.spawn((Transform::from_xyz(f32::NAN, 1.0, 0.0), l));
        for i in 0..40 {
            world.spawn((
                Transform::from_xyz(i as f32 * 0.1, 3.0, 0.0),
                Light::point([1.0; 3], 1.0, 0.0),
            ));
        }
        let (_stats, at) = h.render(&world, &RenderSettings::default());
        let mid = at(SIZE / 2, SIZE / 2);
        assert!(mid[0] > 100, "the good cube still draws: {mid:?}");
    }
}

#[test]
fn bad_textures_and_materials_fall_back() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    let plane = h.meshes.add(MeshData::plane(8.0));
    let mut world = World::new();
    world.spawn(top_down_camera(8.0));
    let sizes = [(0, 0), (0, 4), (4, 0), (3, 3)];
    for (w, hh) in sizes {
        let tex = h
            .textures
            .add(TextureData::rgba8(w, hh, vec![200; 5], true));
        let mat = h.materials.add(Material {
            base_texture: Some(tex),
            normal_texture: Some(tex),
            ..Default::default()
        });
        world.spawn((
            Transform::IDENTITY,
            MeshRenderer::new(plane, Color::WHITE).with_material(mat),
        ));
    }
    let huge = h
        .textures
        .add(TextureData::rgba8(1 << 20, 1, vec![0; 4], true));
    let mat = h.materials.add(Material {
        base_texture: Some(huge),
        ..Default::default()
    });
    world.spawn((
        Transform::IDENTITY,
        MeshRenderer::new(plane, Color::WHITE).with_material(mat),
    ));
    let (_stats, at) = h.render(&world, &RenderSettings::default());
    let _ = at(SIZE / 2, SIZE / 2);
}
