//! Materials, textures, skies, surface shaders.

use super::*;
use crate::core::assets::upload_texture;

fn unlit(texture: Option<TextureHandle>) -> Material {
    Material {
        base_color: Color::WHITE,
        base_texture: texture,
        point_filter: true,
        unlit: true,
        ..Default::default()
    }
}

#[test]
fn textured_quad_shows_texels_at_known_uvs() {
    for editor_like in [false, true] {
        let Some(mut h) = Harness::new(editor_like) else {
            return;
        };
        // 2×2: red green / blue white, rows top to bottom.
        let texture = h.textures.add(TextureData::rgba8(
            2,
            2,
            [
                [255, 0, 0, 255],
                [0, 255, 0, 255],
                [0, 0, 255, 255],
                [255; 4],
            ]
            .concat(),
            true,
        ));
        let material = h.materials.add(unlit(Some(texture)));
        let plane = h.meshes.add(MeshData::plane(4.0));
        let mut world = World::new();
        world.spawn((
            Transform::IDENTITY,
            MeshRenderer::new(plane, Color::WHITE).with_material(material),
        ));
        world.spawn(top_down_camera(8.0));
        let (_, at) = h.render(&world, &flat_settings());
        // Image top is -Z (the top of the screen), image left is -X.
        let px = |x: f32, z: f32| at(column(x, 8.0), column(z, 8.0));
        let cases = [
            ((-1.0, -1.0), [255, 0, 0]),
            ((1.0, -1.0), [0, 255, 0]),
            ((-1.0, 1.0), [0, 0, 255]),
            ((1.0, 1.0), [255, 255, 255]),
        ];
        for ((x, z), want) in cases {
            let got = px(x, z);
            assert!(
                close(got, want),
                "editor_like={editor_like} ({x}, {z}): {got:?}"
            );
        }
        // Outside the plane: clear colour.
        assert!(close(px(3.0, 0.0), [0, 0, 255]));
    }
}

#[test]
fn roughness_and_metallic_shape_the_highlight() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    let plane = h.meshes.add(MeshData::plane(20.0));
    let red = Color::rgb(1.0, 0.0, 0.0);
    let mut make = |roughness: f32, metallic: f32, base_color: Color| {
        h.materials.add(Material {
            base_color,
            roughness,
            metallic,
            ..Default::default()
        })
    };
    let smooth = make(0.15, 0.0, Color::WHITE);
    let rough = make(0.9, 0.0, Color::WHITE);
    let red_plastic = make(0.3, 0.0, red);
    let red_metal = make(0.3, 1.0, red);
    let settings = RenderSettings {
        light_color: Color::BLACK,
        ..Default::default()
    };
    let mut centre = |material| {
        let mut world = World::new();
        world.spawn((
            Transform::IDENTITY,
            MeshRenderer::new(plane, Color::WHITE).with_material(material),
        ));
        world.spawn(top_down_camera(8.0));
        // Straight under the camera: the mirror direction at the centre.
        world.spawn((
            Transform::from_xyz(0.0, 2.0, 0.0),
            Light::point([1.0; 3], 1.0, 10.0),
        ));
        let (_, at) = h.render(&world, &settings);
        at(SIZE / 2, SIZE / 2)
    };
    let (s, r) = (centre(smooth), centre(rough));
    assert!(luma(s) > luma(r) + 100, "smooth {s:?} vs rough {r:?}");
    // A dielectric's highlight is white; a metal's takes the base colour.
    let (plastic, metal) = (centre(red_plastic), centre(red_metal));
    assert!(
        plastic[1] > metal[1] + 80 && metal[0] > 200,
        "plastic {plastic:?} vs metal {metal:?}"
    );
}

#[test]
fn emissive_shows_without_lights() {
    let Some(mut h) = Harness::new(true) else {
        return;
    };
    let material = h.materials.add(Material {
        base_color: Color::BLACK,
        emissive: [0.0, 1.0, 0.0],
        ..Default::default()
    });
    let cube = h.meshes.add(MeshData::cube(1.0));
    let mut world = World::new();
    world.spawn((
        Transform::IDENTITY,
        MeshRenderer::new(cube, Color::WHITE).with_material(material),
    ));
    world.spawn((Transform::from_xyz(0.0, 0.0, 3.0), Camera::default()));
    let (_, at) = h.render(&world, &flat_settings());
    let c = at(SIZE / 2, SIZE / 2);
    assert!(close(c, [0, 255, 0]), "{c:?}");
}

#[test]
fn alpha_cutoff_discards() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    // Left texel transparent, right opaque.
    let texture = h.textures.add(TextureData::rgba8(
        2,
        1,
        vec![255, 255, 255, 0, 255, 255, 255, 255],
        true,
    ));
    let material = h.materials.add(Material {
        alpha_cutoff: 0.5,
        ..unlit(Some(texture))
    });
    let plane = h.meshes.add(MeshData::plane(4.0));
    let mut world = World::new();
    world.spawn((
        Transform::IDENTITY,
        MeshRenderer::new(plane, Color::WHITE).with_material(material),
    ));
    world.spawn(top_down_camera(8.0));
    let (_, at) = h.render(&world, &flat_settings());
    let (cut, kept) = (
        at(column(-1.0, 8.0), SIZE / 2),
        at(column(1.0, 8.0), SIZE / 2),
    );
    assert!(close(cut, [0, 0, 255]), "discarded: clear colour {cut:?}");
    assert!(close(kept, [255, 255, 255]), "kept {kept:?}");
}

/// A camera at the origin looking down -Z (horizon across the middle row).
fn horizon_world() -> World {
    let mut world = World::new();
    world.spawn((Transform::IDENTITY, Camera::default()));
    world
}

#[test]
fn procedural_sky_fills_the_background() {
    for editor_like in [false, true] {
        let Some(mut h) = Harness::new(editor_like) else {
            return;
        };
        let world = horizon_world();
        let mut settings = RenderSettings {
            sky: SkyMode::Procedural {
                zenith: [0.0, 0.0, 1.0],
                horizon: [1.0, 0.0, 0.0],
                ground: [0.0, 1.0, 0.0],
                sun_size: 0.0,
            },
            ..flat_settings()
        };
        settings.clear_color = Color::BLACK;
        let (stats, at) = h.render(&world, &settings);
        assert_eq!(stats.draw_calls, 0);
        assert!(h.view.sky_active());
        let (top, middle, bottom) = (
            at(SIZE / 2, 0),
            at(SIZE / 2, SIZE / 2),
            at(SIZE / 2, SIZE - 1),
        );
        assert!(
            top[2] > top[0] + 100,
            "editor_like={editor_like} zenith-ish top {top:?}"
        );
        assert!(
            middle[0] > middle[2] + 100,
            "horizon in the middle {middle:?}"
        );
        assert!(bottom[1] > bottom[0] + 100, "ground below {bottom:?}");

        // A sun straight ahead: a white disc in the middle.
        settings.light_dir = Vec3::Z;
        settings.light_color = Color::WHITE;
        settings.sky = SkyMode::Procedural {
            zenith: [0.0, 0.0, 1.0],
            horizon: [1.0, 0.0, 0.0],
            ground: [0.0, 1.0, 0.0],
            sun_size: 5.0,
        };
        let (_, at) = h.render(&world, &settings);
        assert_eq!(at(SIZE / 2, SIZE / 2), [255, 255, 255]);
    }
}

#[test]
fn equirect_texture_sky() {
    for editor_like in [false, true] {
        let Some(mut h) = Harness::new(editor_like) else {
            return;
        };
        // 4×8 HDR panorama: upper half blue 2.0, lower half red 2.0.
        let mut pixels = Vec::new();
        for row in 0..8 {
            let c = if row < 4 {
                [0.0, 0.0, 2.0, 1.0]
            } else {
                [2.0, 0.0, 0.0, 1.0]
            };
            for _ in 0..4 {
                pixels.extend(c);
            }
        }
        let texture = h.textures.add(TextureData::rgba32f(4, 8, &pixels));
        let world = horizon_world();
        let mut settings = RenderSettings {
            sky: SkyMode::Texture {
                texture,
                rotation: 0.0,
                intensity: 1.0,
            },
            ..flat_settings()
        };
        let (_, at) = h.render(&world, &settings);
        let (top, bottom) = (at(SIZE / 2, 2), at(SIZE / 2, SIZE - 3));
        assert!(close(top, [0, 0, 255]), "editor_like={editor_like} {top:?}");
        assert!(close(bottom, [255, 0, 0]), "{bottom:?}");
        // HDR × intensity: 2.0 × 0.25 = 0.5 linear ≈ 188 sRGB.
        settings.sky = SkyMode::Texture {
            texture,
            rotation: 0.0,
            intensity: 0.25,
        };
        let (_, at) = h.render(&world, &settings);
        let top = at(SIZE / 2, 2);
        assert!(top[2].abs_diff(188) <= 4 && top[0] < 5, "{top:?}");
    }
}

#[test]
fn triplanar_projects_in_world_space() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    // Red | green.
    let texture = h.textures.add(TextureData::rgba8(
        2,
        1,
        vec![255, 0, 0, 255, 0, 255, 0, 255],
        true,
    ));
    let cube = h.meshes.add(MeshData::cube(4.0));
    let triplanar = h.materials.add(Material {
        triplanar: true,
        // One repeat per 2 m: red on x ∈ (0, 1), green on (1, 2) and (-1, 0).
        uv_scale: [0.5, 0.5],
        ..unlit(Some(texture))
    });
    let mesh_uv = h.materials.add(unlit(Some(texture)));
    let mut world = World::new();
    let e = world.spawn((
        Transform::IDENTITY,
        MeshRenderer::new(cube, Color::WHITE).with_material(triplanar),
    ));
    world.spawn(top_down_camera(8.0));
    let settings = flat_settings();
    // The top face is 6 m from the camera.
    let (_, at) = h.render(&world, &settings);
    let px = |x: f32| at(column(x, 6.0), SIZE / 2);
    assert!(close(px(0.5), [255, 0, 0]), "{:?}", px(0.5));
    assert!(close(px(1.5), [0, 255, 0]), "{:?}", px(1.5));
    assert!(close(px(-0.5), [0, 255, 0]), "{:?}", px(-0.5));
    // Mesh UVs instead: the face maps the texture once (left red, right green).
    world.get::<&mut MeshRenderer>(e).unwrap().material = Some(mesh_uv);
    let (_, at) = h.render(&world, &settings);
    let px = |x: f32| at(column(x, 6.0), SIZE / 2);
    assert!(close(px(0.5), [0, 255, 0]) && close(px(-0.5), [255, 0, 0]));
}

#[test]
fn normal_map_tilts_towards_tangent_and_image_up() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    let plane = h.meshes.add(MeshData::plane(20.0));
    let encode = |n: Vec3| {
        let n = n.normalize();
        [n.x, n.y, n.z].map(|c| ((c * 0.5 + 0.5) * 255.0).round() as u8)
    };
    let mut map = |n: Vec3| {
        let [r, g, b] = encode(n);
        let t = h
            .textures
            .add(TextureData::rgba8(1, 1, vec![r, g, b, 255], false));
        h.materials.add(Material {
            base_color: Color::WHITE,
            normal_texture: Some(t),
            ..Default::default()
        })
    };
    // Tangent space: +x = +u = world +X; +y = image up = world -Z on the plane.
    let towards_u = map(Vec3::new(1.0, 0.0, 1.0));
    let towards_up = map(Vec3::new(0.0, 1.0, 1.0));
    let flat = h.materials.add(Material {
        base_color: Color::WHITE,
        ..Default::default()
    });
    let mut lit = |material, light_dir: Vec3| {
        let mut world = World::new();
        world.spawn((
            Transform::IDENTITY,
            MeshRenderer::new(plane, Color::WHITE).with_material(material),
        ));
        world.spawn(top_down_camera(8.0));
        let settings = RenderSettings {
            light_dir,
            ambient: Color::BLACK,
            ground_ambient: Color::BLACK,
            ..Default::default()
        };
        let (_, at) = h.render(&world, &settings);
        luma(at(column(-2.0, 8.0), column(-2.0, 8.0)))
    };
    // Grazing light from +X (travelling -X) and from -Z (travelling +Z).
    let from_x = Vec3::new(-1.0, -0.15, 0.0);
    let from_neg_z = Vec3::new(0.0, -0.15, 1.0);
    let (flat_x, tilted_x) = (lit(flat, from_x), lit(towards_u, from_x));
    assert!(
        tilted_x > flat_x + 150,
        "towards +u: {tilted_x} vs {flat_x}"
    );
    let (flat_z, tilted_z) = (lit(flat, from_neg_z), lit(towards_up, from_neg_z));
    assert!(
        tilted_z > flat_z + 150,
        "towards image up: {tilted_z} vs {flat_z}"
    );
}

const MAGENTA_SURFACE: &str = "fn surface(in: SurfaceInput) -> SurfaceOutput {
    var out = default_surface(in);
    out.color = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    out.emissive = vec3<f32>(1.0, 0.0, 1.0);
    return out;
}
";

/// A cube drawn with `shader`; returns the centre pixel.
fn shaded_cube(h: &mut Harness, shader: ShaderHandle) -> [u8; 3] {
    let material = h.materials.add(Material {
        shader: Some(shader),
        ..Default::default()
    });
    let cube = h.meshes.add(MeshData::cube(1.0));
    let mut world = World::new();
    world.spawn((
        Transform::IDENTITY,
        MeshRenderer::new(cube, Color::WHITE).with_material(material),
    ));
    // A plain cube too, so both pipelines draw in one pass.
    world.spawn((
        Transform::from_xyz(5.0, 0.0, 0.0),
        MeshRenderer::new(cube, Color::WHITE),
    ));
    world.spawn((Transform::from_xyz(0.0, 0.0, 3.0), Camera::default()));
    let settings = RenderSettings {
        light_dir: Vec3::NEG_Z,
        ..flat_settings()
    };
    let (_, at) = h.render(&world, &settings);
    at(SIZE / 2, SIZE / 2)
}

#[test]
fn custom_surface_shader_outputs_its_colour() {
    for editor_like in [false, true] {
        let Some(mut h) = Harness::new(editor_like) else {
            return;
        };
        let shader = h.shaders.add(MAGENTA_SURFACE.into());
        assert_eq!(h.shaders.error(shader), None);
        let c = shaded_cube(&mut h, shader);
        assert!(close(c, [255, 0, 255]), "editor_like={editor_like}: {c:?}");
    }
}

#[test]
fn broken_surface_shader_reports_and_keeps_the_last_good_pipeline() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    let broken = "fn surface(in: SurfaceInput) -> SurfaceOutput {\n    return 1.0;\n}\n";
    // Never compiled: the built-in pipeline draws instead, no panic.
    let shader = h.shaders.add(broken.into());
    let err = h.shaders.error(shader).expect("an error");
    assert!(
        err.contains("return 1.0") && !err.contains("template"),
        "{err}"
    );
    let c = shaded_cube(&mut h, shader);
    // No lights in these settings: the built-in grey material is black.
    assert!(close(c, [0, 0, 0]), "built-in look: {c:?}");
    // Good, then broken again: the good pipeline stays.
    h.shaders.set(shader, MAGENTA_SURFACE.into());
    assert!(close(shaded_cube(&mut h, shader), [255, 0, 255]));
    h.shaders.set(shader, broken.into());
    assert!(h.shaders.error(shader).is_some());
    assert!(close(shaded_cube(&mut h, shader), [255, 0, 255]));
}

#[test]
fn mip_chain_reaches_the_gpu_and_big_textures_shrink_to_fit() {
    let Some((device, queue)) = device() else {
        return;
    };
    let data = TextureData::rgba8(8, 4, vec![200; 8 * 4 * 4], true);
    let t = upload_texture(&device, &queue, &data, u32::MAX);
    assert_eq!(t.mips, TextureData::mip_count(8, 4));
    assert_eq!(t.view.texture().mip_level_count(), 4);
    // Over the limit: starts at the first level that fits.
    let t = upload_texture(&device, &queue, &data, 2);
    assert_eq!(t.mips, 2);
    assert_eq!(t.view.texture().width(), 2);
    // Bad data: a 1×1 stand-in instead of a panic.
    let bad = TextureData {
        format: TextureFormat::Rgba16Float,
        ..data
    };
    assert_eq!(upload_texture(&device, &queue, &bad, u32::MAX).mips, 1);
}

#[test]
fn fog_fades_into_the_sky_at_the_horizon() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    let plane = h.meshes.add(MeshData::plane(400.0));
    let mut world = World::new();
    world.spawn((
        Transform::from_xyz(0.0, -1.0, 0.0),
        MeshRenderer::new(plane, Color::WHITE),
    ));
    world.spawn((Transform::IDENTITY, Camera::default()));
    let settings = RenderSettings {
        sky: SkyMode::Procedural {
            zenith: [0.0, 0.0, 1.0],
            horizon: [1.0, 0.0, 0.0],
            ground: [0.0, 1.0, 0.0],
            sun_size: 0.0,
        },
        fog: Some(crate::Fog {
            color: Color::rgb(0.0, 1.0, 0.0),
            start: 0.0,
            end: 1.0,
        }),
        ..flat_settings()
    };
    let (_, at) = h.render(&world, &settings);
    // The floor just below the horizon is fully fogged: horizon red, not fog green.
    let c = at(SIZE / 2, SIZE / 2 + 4);
    assert!(close(c, [255, 0, 0]), "{c:?}");
}

#[test]
fn skinned_mesh_renders_in_its_posed_place() {
    use crate::SkinnedMesh;
    use olvin_anim::{Joint, JointPose, Skeleton};
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    // A strip along +X of three columns (z = ±0.5): the first on the root
    // joint, the other two on the tip joint that sits at x = 1.
    let joint = |name: &str, parent, target, x: f32| Joint {
        name: name.into(),
        parent,
        target,
        rest: JointPose {
            translation: Vec3::new(x, 0.0, 0.0),
            ..JointPose::IDENTITY
        },
        inverse_bind: glam::Mat4::from_translation(Vec3::new(-x, 0.0, 0.0)),
    };
    let skeleton = Skeleton::new(vec![
        joint("root", None, 0, 0.0),
        joint("tip", Some(0), 1, 1.0),
    ])
    .unwrap();
    let mut bind = MeshData::default();
    for x in [0.0, 1.0, 2.0] {
        for z in [-0.5, 0.5] {
            bind.vertices
                .push(crate::Vertex::new([x, 0.0, z], [0.0, 1.0, 0.0]));
        }
    }
    for c in 0..2u32 {
        let (a, b, c0, d) = (2 * c, 2 * c + 1, 2 * c + 2, 2 * c + 3);
        // Both windings: the test is about where it is drawn, not culling.
        bind.indices.extend([a, b, d, a, d, c0, a, d, b, a, c0, d]);
    }
    let joints = vec![[0; 4], [0; 4], [1; 4], [1; 4], [1; 4], [1; 4]];
    let weights = vec![[1.0, 0.0, 0.0, 0.0]; 6];
    let skinned =
        SkinnedMesh::new(bind.clone(), joints, weights, olvin_ecs::Entity::DANGLING).unwrap();
    let mesh = h.meshes.add(bind);
    let material = h.materials.add(unlit(None));
    let mut world = World::new();
    world.spawn((
        Transform::IDENTITY,
        MeshRenderer::new(mesh, Color::rgb(1.0, 0.0, 0.0)).with_material(material),
    ));
    world.spawn(top_down_camera(8.0));
    let settings = flat_settings();
    let row = |z: f32| ((0.5 + z / (8.0 * 30f32.to_radians().tan()) * 0.5) * SIZE as f32) as u32;

    // Rest pose: red at x = 1.5, z = 0; background at z = 2.
    h.meshes.set(
        mesh,
        skinned.skin(&skeleton.skin_matrices(&skeleton.rest_pose())),
    );
    let (_, at) = h.render(&world, &settings);
    assert!(close(at(column(1.5, 8.0), row(0.0)), [255, 0, 0]));
    assert!(close(at(column(1.5, 8.0), row(2.0)), [0, 0, 255]));

    // The tip slides 2 m along +Z: its columns now cover z = 2.
    let mut pose = skeleton.rest_pose();
    pose.joints[1].translation += Vec3::new(0.0, 0.0, 2.0);
    h.meshes
        .set(mesh, skinned.skin(&skeleton.skin_matrices(&pose)));
    let (_, at) = h.render(&world, &settings);
    assert!(
        close(at(column(1.9, 8.0), row(2.0)), [255, 0, 0]),
        "{:?}",
        at(column(1.9, 8.0), row(2.0))
    );
    assert!(close(at(column(1.9, 8.0), row(0.0)), [0, 0, 255]));
}
