//! Exponential height fog.

use super::*;
use crate::HeightFog;

/// An unlit white box at `y`, `distance` metres down -Z from the camera.
fn white_box(h: &mut Harness, world: &mut World, y: f32, z: f32) {
    let cube = h.meshes.add(MeshData::cube(2.0));
    let white = h.materials.add(Material {
        base_color: Color::BLACK,
        emissive: [1.0; 3],
        unlit: true,
        ..Default::default()
    });
    world.spawn((
        Transform::from_xyz(0.0, y, z),
        MeshRenderer::new(cube, Color::WHITE).with_material(white),
    ));
}

fn fogged(height_fog: Option<HeightFog>) -> RenderSettings {
    RenderSettings {
        clear_color: Color::BLACK,
        tonemapping: Tonemapping::None,
        height_fog,
        ..Default::default()
    }
}

/// Linear brightness (0..1) of a pixel of the sRGB target.
fn brightness(at: &impl Fn(u32, u32) -> [u8; 3], x: u32, y: u32) -> f32 {
    let c = at(x, y);
    let lin = |v: u8| ((v as f32 / 255.0 + 0.055) / 1.055).powf(2.4);
    (lin(c[0]) + lin(c[1]) + lin(c[2])) / 3.0
}

#[test]
fn height_fog_grows_with_distance() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    let mut world = World::new();
    world.spawn((Transform::IDENTITY, Camera::default()));
    // Two boxes side by side, one near and one far (the far one smaller on
    // screen, so both are checked in their middle).
    let near = h.meshes.add(MeshData::cube(2.0));
    let far = h.meshes.add(MeshData::cube(8.0));
    let white = h.materials.add(Material {
        base_color: Color::BLACK,
        emissive: [1.0; 3],
        unlit: true,
        ..Default::default()
    });
    world.spawn((
        Transform::from_xyz(-1.5, 0.0, -6.0),
        MeshRenderer::new(near, Color::WHITE).with_material(white),
    ));
    world.spawn((
        Transform::from_xyz(8.0, 0.0, -60.0),
        MeshRenderer::new(far, Color::WHITE).with_material(white),
    ));
    let fog = HeightFog {
        color: Color::BLACK,
        density: 0.03,
        falloff: 0.0,
        height: 0.0,
    };
    let (_, clear) = h.render(&world, &fogged(None));
    let (_, misty) = h.render(&world, &fogged(Some(fog)));
    // Find a lit pixel of each box in the unfogged frame.
    let lit = |at: &dyn Fn(u32, u32) -> [u8; 3], range: std::ops::Range<u32>| {
        (0..SIZE)
            .flat_map(|y| range.clone().map(move |x| (x, y)))
            .find(|&(x, y)| luma(at(x, y)) > 700)
    };
    let (Some(a), Some(b)) = (lit(&clear, 0..SIZE / 2), lit(&clear, SIZE / 2..SIZE)) else {
        panic!("boxes not on screen");
    };
    let (near_px, far_px) = (a, b);
    let fog_at = |(x, y): (u32, u32)| 1.0 - brightness(&misty, x, y) / brightness(&clear, x, y);
    // About 5.5 m of uniform fog at 0.03: 1 - e^-0.165 = 15%; 58 m: 82%.
    let (near_fog, far_fog) = (fog_at(near_px), fog_at(far_px));
    assert!((near_fog - 0.15).abs() < 0.05, "near {near_fog}");
    assert!((far_fog - 0.82).abs() < 0.08, "far {far_fog}");
    // With no fog at all nothing changes.
    let (_, off) = h.render(
        &world,
        &fogged(Some(HeightFog {
            density: 0.0,
            ..fog
        })),
    );
    assert_eq!(off(near_px.0, near_px.1), clear(near_px.0, near_px.1));
}

#[test]
fn height_fog_is_thicker_low_than_high() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    let mut world = World::new();
    world.spawn((Transform::from_xyz(0.0, 10.0, 0.0), Camera::default()));
    // The same distance from the camera, one box down in the mist, one high.
    white_box(&mut h, &mut world, 3.0, -14.0);
    white_box(&mut h, &mut world, 17.0, -14.0);
    let fog = HeightFog {
        color: Color::BLACK,
        density: 0.2,
        falloff: 0.3,
        height: 0.0,
    };
    let (_, misty) = h.render(&world, &fogged(Some(fog)));
    let (_, clear) = h.render(&world, &fogged(None));
    // Rows of the two boxes: screen y of world y at 14 m is 10 + (y - 10) -> find lit rows.
    let column = SIZE / 2;
    let lit: Vec<u32> = (0..SIZE)
        .filter(|&y| luma(clear(column, y)) > 700)
        .collect();
    assert!(lit.len() > 6, "{lit:?}");
    // Top box (small y on screen) vs the bottom one.
    let top = lit[0];
    let bottom = *lit.last().unwrap();
    assert!(top < SIZE / 2 && bottom > SIZE / 2, "{lit:?}");
    let (high, low) = (
        brightness(&misty, column, top),
        brightness(&misty, column, bottom),
    );
    assert!(high > low + 0.2, "high {high}, low {low}");
    // Fog colour white on a dark box brightens it instead: the colour is used.
    let bright = HeightFog {
        color: Color::rgb(0.0, 0.0, 1.0),
        ..fog
    };
    let (_, blue) = h.render(&world, &fogged(Some(bright)));
    let c = blue(column, bottom);
    assert!(c[2] > c[0], "{c:?}");
}

#[test]
fn height_fog_works_on_the_compat_tier_too() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    h.core.set_tier(RenderTier::Compat);
    let mut world = World::new();
    world.spawn((Transform::IDENTITY, Camera::default()));
    white_box(&mut h, &mut world, 0.0, -20.0);
    let fog = HeightFog {
        color: Color::BLACK,
        density: 0.2,
        falloff: 0.0,
        height: 0.0,
    };
    let (_, misty) = h.render(&world, &fogged(Some(fog)));
    let c = brightness(&misty, SIZE / 2, SIZE / 2);
    assert!(c < 0.1, "{c}");
}
