//! The retro (PS1) render style on the mesh shader: vertex snapping, affine
//! textures, colour depth and dither, vertex lighting, draw distance,
//! nearest textures, per-material overrides.

use super::*;
use crate::RetroSettings;

/// Retro on, every effect off: the starting point of each test.
fn only() -> RetroSettings {
    RetroSettings {
        enabled: true,
        snap_resolution: [0, 0],
        snap_grid_m: 0.0,
        affine_strength: 0.0,
        affine_fade_near: 0.0,
        color_bits: 0,
        dither_strength: 0.0,
        vertex_lighting: false,
        shadows: false,
        draw_distance: 0.0,
        dissolve: false,
        cull_small_px: 0.0,
        force_nearest: false,
        internal_resolution: [0, 0],
        pixel_scale: 0,
        letterbox: false,
    }
}

fn with(retro: RetroSettings) -> RenderSettings {
    RenderSettings {
        retro,
        ..flat_settings()
    }
}

fn unlit_material(texture: Option<TextureHandle>, retro: Option<bool>) -> Material {
    Material {
        base_color: Color::WHITE,
        base_texture: texture,
        unlit: true,
        retro,
        ..Default::default()
    }
}

fn all_pixels(at: &impl Fn(u32, u32) -> [u8; 3]) -> Vec<[u8; 3]> {
    (0..SIZE * SIZE).map(|i| at(i % SIZE, i / SIZE)).collect()
}

/// First and last+1 pixel column of `row` that aren't the blue clear colour.
fn extent(at: &impl Fn(u32, u32) -> [u8; 3], row: u32) -> (u32, u32) {
    let lit = |x: u32| at(x, row)[0] > 100;
    let first = (0..SIZE).find(|&x| lit(x)).expect("something drawn");
    let last = (0..SIZE).rev().find(|&x| lit(x)).unwrap();
    (first, last + 1)
}

fn cube_world(h: &mut Harness, x: f32, retro: Option<bool>) -> World {
    let cube = h.meshes.add(MeshData::cube(2.0));
    let material = h.materials.add(unlit_material(None, retro));
    let mut world = World::new();
    world.spawn((
        Transform::from_xyz(x, 0.0, -4.0),
        MeshRenderer::new(cube, Color::WHITE).with_material(material),
    ));
    world.spawn((Transform::IDENTITY, Camera::default()));
    world
}

#[test]
fn retro_with_every_effect_off_is_the_plain_render() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
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
    let lit = h.materials.add(Material {
        base_texture: Some(texture),
        ..Default::default()
    });
    let plane = h.meshes.add(MeshData::plane(6.0));
    let mut world = World::new();
    world.spawn((
        Transform::IDENTITY,
        MeshRenderer::new(plane, Color::WHITE).with_material(lit),
    ));
    world.spawn(top_down_camera(5.0));
    world.spawn((
        Transform::from_xyz(1.0, 1.0, 1.0),
        Light::point([1.0, 0.8, 0.6], 3.0, 6.0),
    ));
    let plain = RenderSettings {
        ambient: Color::rgb(0.1, 0.1, 0.1),
        ..Default::default()
    };
    let (_, off) = h.render(&world, &plain);
    let on = RenderSettings {
        retro: only(),
        ..plain.clone()
    };
    let (_, on) = h.render(&world, &on);
    assert_eq!(all_pixels(&off), all_pixels(&on), "retro without effects");
}

#[test]
fn vertices_snap_to_the_virtual_screen_grid() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    let world = cube_world(&mut h, 0.3, None);
    let (_, plain) = h.render(
        &world,
        &with(RetroSettings {
            enabled: false,
            ..only()
        }),
    );
    let snapped = RetroSettings {
        snap_resolution: [16, 16],
        ..only()
    };
    let (_, snap) = h.render(&world, &with(snapped));
    save_shot("snap_off", SIZE, &plain);
    save_shot("snap_on", SIZE, &snap);
    let row = SIZE / 2;
    let (pa, pb) = extent(&plain, row);
    let (sa, sb) = extent(&snap, row);
    // A 16x16 virtual screen on 64 pixels: cells of 4 pixels.
    assert!(
        pa % 4 != 0 || pb % 4 != 0,
        "unsnapped edges happen to be on the grid: {pa} {pb}"
    );
    assert!(
        sa % 4 == 0 && sb % 4 == 0,
        "snapped edges {sa}..{sb} are on the 4 px grid"
    );
    // Stable: the same frame again is the same image.
    let (_, again) = h.render(&world, &with(snapped));
    assert_eq!(all_pixels(&snap), all_pixels(&again));
    // The material can opt in by itself (retro off globally), or out.
    let world = cube_world(&mut h, 0.3, Some(true));
    let (_, forced) = h.render(
        &world,
        &with(RetroSettings {
            enabled: false,
            ..snapped
        }),
    );
    assert_eq!(extent(&forced, row), (sa, sb), "per-material retro: on");
    let world = cube_world(&mut h, 0.3, Some(false));
    let (_, opted_out) = h.render(&world, &with(snapped));
    assert_eq!(extent(&opted_out, row), (pa, pb), "per-material retro: off");
}

#[test]
fn snapping_survives_geometry_behind_the_camera() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    // A huge floor around the camera: its corners are behind the near plane.
    let floor = h.meshes.add(MeshData::plane(400.0));
    let material = h.materials.add(unlit_material(None, None));
    let mut world = World::new();
    world.spawn((
        Transform::from_xyz(0.0, -1.0, 0.0),
        MeshRenderer::new(floor, Color::WHITE).with_material(material),
    ));
    world.spawn((Transform::IDENTITY, Camera::default()));
    let snapped = RetroSettings {
        snap_resolution: [32, 32],
        snap_grid_m: 0.5,
        ..only()
    };
    let (_, plain) = h.render(
        &world,
        &with(RetroSettings {
            enabled: false,
            ..snapped
        }),
    );
    let (_, snap) = h.render(&world, &with(snapped));
    // The floor still fills the lower half and the sky the top.
    for x in [2, SIZE / 2, SIZE - 3] {
        assert_eq!(plain(x, SIZE - 3), snap(x, SIZE - 3), "floor at {x}");
        assert!(close(snap(x, 2), [0, 0, 255]), "clear colour at the top");
        assert!(snap(x, SIZE - 3)[0] > 200);
    }
}

/// A big checkered floor seen at a grazing angle.
fn floor_world(h: &mut Harness, retro: Option<bool>) -> World {
    let mut pixels = Vec::new();
    for i in 0..4u32 {
        let on = (i % 2 + i / 2) % 2 == 0;
        pixels.extend(if on { [255u8; 4] } else { [0, 0, 0, 255] });
    }
    let texture = h.textures.add(TextureData::rgba8(2, 2, pixels, true));
    let material = h.materials.add(Material {
        point_filter: true,
        uv_scale: [8.0, 8.0],
        ..unlit_material(Some(texture), retro)
    });
    let floor = h.meshes.add(MeshData::plane(60.0));
    let mut world = World::new();
    world.spawn((
        Transform::from_xyz(0.0, -1.0, -30.0),
        MeshRenderer::new(floor, Color::WHITE).with_material(material),
    ));
    world.spawn((Transform::IDENTITY, Camera::default()));
    world
}

fn differing(a: &impl Fn(u32, u32) -> [u8; 3], b: &impl Fn(u32, u32) -> [u8; 3]) -> usize {
    all_pixels(a)
        .iter()
        .zip(all_pixels(b))
        .filter(|(p, q)| p.iter().zip(q.iter()).any(|(x, y)| x.abs_diff(*y) > 30))
        .count()
}

#[test]
fn affine_mapping_warps_textures_and_fades_near_the_camera() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    let world = floor_world(&mut h, None);
    let (_, plain) = h.render(
        &world,
        &with(RetroSettings {
            enabled: false,
            ..only()
        }),
    );
    let affine = RetroSettings {
        affine_strength: 1.0,
        affine_fade_near: 0.0,
        ..only()
    };
    let (_, warped) = h.render(&world, &with(affine));
    save_shot("affine_off", SIZE, &plain);
    save_shot("affine_on", SIZE, &warped);
    let warp = differing(&plain, &warped);
    assert!(
        warp > 200,
        "the affine floor differs from perspective: {warp} pixels"
    );
    // Strength 0 is perspective-correct again, bit for bit.
    let (_, zero) = h.render(
        &world,
        &with(RetroSettings {
            affine_strength: 0.0,
            ..affine
        }),
    );
    assert_eq!(all_pixels(&plain), all_pixels(&zero));
    // A fade distance far beyond the scene brings the perspective back.
    let (_, faded) = h.render(
        &world,
        &with(RetroSettings {
            affine_fade_near: 5000.0,
            ..affine
        }),
    );
    let left = differing(&plain, &faded);
    assert!(
        left * 4 < warp,
        "fade restores perspective: {left} vs {warp}"
    );
}

#[test]
fn colours_snap_to_the_5_bit_lattice_with_a_bayer_dither() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    // A grey ramp texture along x on an unlit quad.
    let pixels: Vec<u8> = (0..64u32)
        .flat_map(|i| {
            let v = (i * 4 + 2).min(255) as u8;
            [v, v, v, 255]
        })
        .collect();
    let ramp = h.textures.add(TextureData::rgba8(64, 1, pixels, false));
    let material = h.materials.add(Material {
        point_filter: true,
        ..unlit_material(Some(ramp), None)
    });
    let plane = h.meshes.add(MeshData::plane(4.0));
    let mut world = World::new();
    world.spawn((
        Transform::IDENTITY,
        MeshRenderer::new(plane, Color::WHITE).with_material(material),
    ));
    world.spawn(top_down_camera(3.47));
    let dither = RetroSettings {
        color_bits: 5,
        dither_strength: 1.0,
        ..only()
    };
    let lattice = |v: u8| {
        let step = 255.0 / 31.0;
        let k = (f32::from(v) / step).round();
        (f32::from(v) - k * step).abs() <= 1.6
    };
    let (_, plain) = h.render(
        &world,
        &with(RetroSettings {
            color_bits: 5,
            dither_strength: 0.0,
            ..dither
        }),
    );
    let mut levels = std::collections::BTreeSet::new();
    for y in 8..SIZE - 8 {
        for x in 8..SIZE - 8 {
            for c in plain(x, y) {
                assert!(lattice(c), "plain quantisation at ({x},{y}): {c}");
            }
            levels.insert(plain(x, y)[0]);
        }
    }
    assert!(
        levels.len() <= 32 && levels.len() > 8,
        "levels: {}",
        levels.len()
    );
    let (_, dithered) = h.render(&world, &with(dither));
    save_shot("dither_off", SIZE, &plain);
    save_shot("dither_on", SIZE, &dithered);
    for y in 8..SIZE - 8 {
        for x in 8..SIZE - 8 {
            for c in dithered(x, y) {
                assert!(lattice(c), "dithered colours stay on the lattice: {c}");
            }
        }
    }
    // The dither pattern repeats every 4 pixels vertically on a flat ramp
    // column and differs from plain rounding.
    assert!(differing(&plain, &dithered) > 0 || all_pixels(&plain) != all_pixels(&dithered));
    let rows: Vec<u8> = (8..12).map(|y| dithered(30, y)[0]).collect();
    let rows2: Vec<u8> = (12..16).map(|y| dithered(30, y)[0]).collect();
    assert_eq!(rows, rows2, "a 4x4 pattern");
}

#[test]
fn vertex_lighting_interpolates_the_corner_lighting() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    // One 20 m quad, a short point light above its middle: per pixel the
    // middle is bright, per vertex (the corners are out of range) it is dark.
    let material = h.materials.add(Material {
        base_color: Color::WHITE,
        ..Default::default()
    });
    let plane = h.meshes.add(MeshData::plane(20.0));
    let mut world = World::new();
    world.spawn((
        Transform::IDENTITY,
        MeshRenderer::new(plane, Color::WHITE).with_material(material),
    ));
    world.spawn(top_down_camera(6.0));
    world.spawn((
        Transform::from_xyz(0.0, 1.5, 0.0),
        Light::point([1.0; 3], 6.0, 5.0),
    ));
    let base = RenderSettings {
        light_color: Color::BLACK,
        ambient: Color::BLACK,
        ground_ambient: Color::BLACK,
        tonemapping: Tonemapping::None,
        ..Default::default()
    };
    let (_, per_pixel) = h.render(&world, &base);
    let gouraud = RenderSettings {
        retro: RetroSettings {
            vertex_lighting: true,
            ..only()
        },
        ..base.clone()
    };
    let (_, per_vertex) = h.render(&world, &gouraud);
    save_shot("lighting_pixel", SIZE, &per_pixel);
    save_shot("lighting_vertex", SIZE, &per_vertex);
    let mid = (SIZE / 2, SIZE / 2);
    let (p, v) = (per_pixel(mid.0, mid.1), per_vertex(mid.0, mid.1));
    assert!(p[0] > 120, "per-pixel light shows in the middle: {p:?}");
    assert!(v[0] < 20, "no vertex near the light: {v:?}");
}

#[test]
fn draw_distance_cuts_geometry_into_the_fog_colour() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    let cube = h.meshes.add(MeshData::cube(2.0));
    let material = h.materials.add(unlit_material(None, None));
    let mut world = World::new();
    for (x, z) in [(-1.2, -8.0), (1.2, -24.0)] {
        world.spawn((
            Transform::from_xyz(x, 0.0, z),
            MeshRenderer::new(cube, Color::WHITE).with_material(material),
        ));
    }
    world.spawn((Transform::IDENTITY, Camera::default()));
    let settings = |distance: f32| RenderSettings {
        retro: RetroSettings {
            draw_distance: distance,
            ..only()
        },
        clear_color: Color::rgb(0.0, 0.0, 1.0),
        ..flat_settings()
    };
    let (_, all) = h.render(&world, &settings(0.0));
    let (stats, cut) = h.render(&world, &settings(16.0));
    let right = |at: &dyn Fn(u32, u32) -> [u8; 3]| {
        (SIZE / 2..SIZE).any(|x| (0..SIZE).any(|y| at(x, y)[0] > 100))
    };
    assert!(right(&all), "the far cube is drawn without a cut");
    assert!(!right(&cut), "the far cube is gone past the draw distance");
    assert!(stats.culled >= 1, "culled on the CPU: {stats:?}");
    // The near cube keeps its fog-free colour.
    assert!(cut(SIZE / 2 - 6, SIZE / 2)[0] > 200);
}

#[test]
fn forced_nearest_removes_the_texture_blending() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
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
    let material = h.materials.add(unlit_material(Some(texture), None));
    let plane = h.meshes.add(MeshData::plane(4.0));
    let mut world = World::new();
    world.spawn((
        Transform::IDENTITY,
        MeshRenderer::new(plane, Color::WHITE).with_material(material),
    ));
    world.spawn(top_down_camera(8.0));
    let pure = |c: [u8; 3]| c.iter().all(|&v| v < 12 || v > 243);
    let (_, linear) = h.render(&world, &with(only()));
    let (_, nearest) = h.render(
        &world,
        &with(RetroSettings {
            force_nearest: true,
            ..only()
        }),
    );
    let edge = |at: &dyn Fn(u32, u32) -> [u8; 3]| at(column(0.0, 8.0), column(-1.0, 8.0));
    assert!(
        !pure(edge(&linear)),
        "linear blends across texels: {:?}",
        edge(&linear)
    );
    assert!(
        pure(edge(&nearest)),
        "nearest keeps one texel: {:?}",
        edge(&nearest)
    );
}

#[test]
fn low_internal_resolution_upscales_in_blocks_on_the_post_chain() {
    // A tilted white square on black; the 16x16 scene is drawn on 64 pixels.
    let build = |meshes: &mut Meshes, materials: &mut Materials, world: &mut World| {
        let quad = meshes.add(MeshData::plane(3.0));
        let white = materials.add(Material {
            base_color: Color::BLACK,
            emissive: [1.0; 3],
            unlit: true,
            ..Default::default()
        });
        world.spawn((
            Transform {
                rotation: glam::Quat::from_rotation_y(0.4),
                ..Transform::IDENTITY
            },
            MeshRenderer::new(quad, Color::WHITE).with_material(white),
        ));
        world.spawn(top_down_camera(6.0));
    };
    let settings = RenderSettings {
        clear_color: Color::BLACK,
        retro: RetroSettings {
            internal_resolution: [16, 16],
            ..only()
        },
        post: crate::PostSettings {
            fxaa: false,
            ..Default::default()
        },
        ..Default::default()
    };
    let Some(frame) = post::post_world_sized(SIZE, &settings, None, build) else {
        return;
    };
    let at = |x, y| frame.at(x, y);
    save_shot("lowres", SIZE, at);
    // Every 4x4 block is one colour.
    for by in 0..16 {
        for bx in 0..16 {
            let first = at(bx * 4, by * 4);
            for (dx, dy) in [(1, 0), (3, 3), (2, 1)] {
                assert_eq!(at(bx * 4 + dx, by * 4 + dy), first, "block {bx},{by}");
            }
        }
    }
    // The picture itself is there, and a full-resolution render isn't blocky.
    assert!(at(SIZE / 2, SIZE / 2)[0] > 100);
    let sharp = RenderSettings {
        retro: RetroSettings {
            enabled: false,
            ..settings.retro
        },
        ..settings.clone()
    };
    let full = post::post_world_sized(SIZE, &sharp, None, build).unwrap();
    let blocky = |f: &dyn Fn(u32, u32) -> [u8; 3]| {
        (0..SIZE - 1).filter(|&x| f(x, 20) != f(x + 1, 20)).count()
    };
    assert!(blocky(&|x, y| full.at(x, y)) > 0);
}

#[test]
fn the_post_chain_quantises_once_and_the_mesh_shader_leaves_hdr_alone() {
    let build = |meshes: &mut Meshes, materials: &mut Materials, world: &mut World| {
        let quad = meshes.add(MeshData::plane(20.0));
        let grey = materials.add(Material {
            base_color: Color::BLACK,
            emissive: [0.2, 0.31, 0.47],
            unlit: true,
            ..Default::default()
        });
        world.spawn((
            Transform::IDENTITY,
            MeshRenderer::new(quad, Color::WHITE).with_material(grey),
        ));
        world.spawn(top_down_camera(6.0));
    };
    let settings = RenderSettings {
        tonemapping: Tonemapping::None,
        retro: RetroSettings {
            color_bits: 5,
            dither_strength: 0.0,
            ..only()
        },
        ..Default::default()
    };
    let Some(frame) = post::post_world_sized(SIZE, &settings, None, build) else {
        return;
    };
    let step = 255.0 / 31.0;
    for c in frame.at(10, 10) {
        let k = (f32::from(c) / step).round();
        assert!(
            (f32::from(c) - k * step).abs() <= 1.6,
            "on the 5-bit lattice: {c}"
        );
    }
}
