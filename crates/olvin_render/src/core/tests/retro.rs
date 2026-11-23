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
