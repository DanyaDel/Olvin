//! Lights, the sun and its cascaded shadows.

use super::*;

#[test]
fn renders_a_lit_cube_offscreen() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    let cube = h.meshes.add(MeshData::cube(1.0));
    let mut world = World::new();
    world.spawn((
        Transform::IDENTITY,
        MeshRenderer::new(cube, Color::rgb(1.0, 0.0, 0.0)),
    ));
    world.spawn((Transform::from_xyz(0.0, 0.0, 3.0), Camera::default()));
    let mut settings = RenderSettings {
        light_dir: Vec3::new(0.0, 0.0, -1.0),
        clear_color: Color::rgb(0.0, 0.0, 1.0),
        ..Default::default()
    };
    let (stats, at) = h.render(&world, &settings);
    assert_eq!((stats.draw_calls, stats.instances), (1, 1));
    assert_eq!(stats.triangles, 12, "one cube");
    let lit = at(SIZE / 2, SIZE / 2);
    // Red dominates; the dielectric highlight may tint green/blue a little.
    assert!(
        lit[0] > 200 && lit[1] < 120 && lit[2] < 120 && lit[0] > lit[1] + 100,
        "lit red cube in the middle: {lit:?}"
    );
    let corner = at(1, 1);
    assert!(
        corner[2] > 200 && corner[0] < 30,
        "clear colour in the corner: {corner:?}"
    );

    // Light from behind: only ambient on the visible face.
    settings.light_dir = Vec3::new(0.0, 0.0, 1.0);
    let (_, at) = h.render(&world, &settings);
    let unlit = at(SIZE / 2, SIZE / 2);
    assert!(
        unlit[0] > 20 && unlit[0] + 80 < lit[0],
        "unlit side is dimmer but not black: {unlit:?} vs {lit:?}"
    );
}

#[test]
fn red_point_light_brightens_the_floor_near_it() {
    for editor_like in [false, true] {
        let Some(mut h) = Harness::new(editor_like) else {
            return;
        };
        let plane = h.meshes.add(MeshData::plane(20.0));
        let mut world = World::new();
        world.spawn((Transform::IDENTITY, MeshRenderer::new(plane, Color::WHITE)));
        world.spawn(top_down_camera(8.0));
        world.spawn((
            Transform::from_xyz(-2.0, 0.7, 0.0),
            Light::point([1.0, 0.0, 0.0], 4.0, 4.0),
        ));
        // A dark sun so only the point light paints the floor red.
        let settings = RenderSettings {
            light_color: Color::BLACK,
            ..Default::default()
        };
        let (_, at) = h.render(&world, &settings);
        let (y, near, far) = (
            SIZE / 2,
            at(column(-2.0, 8.0), SIZE / 2),
            at(column(3.0, 8.0), SIZE / 2),
        );
        assert!(
            near[0] > far[0] + 60 && near[0] > near[1] + 60,
            "editor_like={editor_like}: near {near:?} far {far:?} (row {y})"
        );
        assert!(luma(far) > 0, "ambient still lights the far floor");
    }
}

#[test]
fn directional_light_casts_a_shadow() {
    for editor_like in [false, true] {
        let Some(mut h) = Harness::new(editor_like) else {
            return;
        };
        let plane = h.meshes.add(MeshData::plane(20.0));
        let cube = h.meshes.add(MeshData::cube(1.0));
        let mut world = World::new();
        world.spawn((Transform::IDENTITY, MeshRenderer::new(plane, Color::WHITE)));
        world.spawn((
            Transform::from_xyz(0.0, 1.5, 0.0),
            MeshRenderer::new(cube, Color::WHITE),
        ));
        world.spawn(top_down_camera(8.0));
        // Travels +X and down at 45°: the cube's shadow lands on x ∈ [0.5, 2.5].
        let sun_dir = Vec3::new(1.0, -1.0, 0.0).normalize();
        let sun = world.spawn((
            Transform::IDENTITY.looking_at(sun_dir, Vec3::Y),
            Light::directional([1.0; 3], 1.0),
        ));
        let settings = RenderSettings::default();
        let (shadowed, open) = ((column(1.5, 8.0), SIZE / 2), (column(-2.5, 8.0), SIZE / 2));

        let (_, at) = h.render(&world, &settings);
        assert!(h.view.shadows_active());
        let (s, o) = (at(shadowed.0, shadowed.1), at(open.0, open.1));
        assert!(
            luma(s) + 150 < luma(o),
            "editor_like={editor_like}: shadowed {s:?} vs open {o:?}"
        );

        // Shadows off: the same pixel is lit like the rest of the floor.
        world.get::<&mut Light>(sun).unwrap().shadows = false;
        let (_, at) = h.render(&world, &settings);
        assert!(!h.view.shadows_active());
        let (s, o) = (at(shadowed.0, shadowed.1), at(open.0, open.1));
        assert!(
            luma(s).abs_diff(luma(o)) < 30,
            "editor_like={editor_like}: no shadow {s:?} vs {o:?}"
        );
    }
}

/// The sun's shadow test scene: a cube casting a shadow onto x ∈ [0.5, 2.5];
/// returns (shadowed, open) pixel columns.
fn shadow_scene(h: &mut Harness) -> (World, (u32, u32), (u32, u32)) {
    let plane = h.meshes.add(MeshData::plane(20.0));
    let cube = h.meshes.add(MeshData::cube(1.0));
    let mut world = World::new();
    world.spawn((Transform::IDENTITY, MeshRenderer::new(plane, Color::WHITE)));
    world.spawn((
        Transform::from_xyz(0.0, 1.5, 0.0),
        MeshRenderer::new(cube, Color::WHITE),
    ));
    world.spawn(top_down_camera(8.0));
    let sun_dir = Vec3::new(1.0, -1.0, 0.0).normalize();
    world.spawn((
        Transform::IDENTITY.looking_at(sun_dir, Vec3::Y),
        Light::directional([1.0; 3], 1.0),
    ));
    let (shadowed, open) = ((column(1.5, 8.0), SIZE / 2), (column(-2.5, 8.0), SIZE / 2));
    (world, shadowed, open)
}

#[test]
fn cascades_and_filters_all_shadow_the_cube() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    let (world, shadowed, open) = shadow_scene(&mut h);
    for tier in [RenderTier::Compat, RenderTier::Full] {
        h.core.set_tier(tier);
        for cascades in 1..=4 {
            for filter in [ShadowFilter::Hard, ShadowFilter::Pcf, ShadowFilter::Soft] {
                let settings = RenderSettings {
                    shadow_cascades: cascades,
                    shadow_filter: filter,
                    shadow_distance: 20.0,
                    ..Default::default()
                };
                let (_, at) = h.render(&world, &settings);
                let cap = if tier == RenderTier::Full { 4 } else { 2 };
                assert_eq!(h.view.shadow_cascades(), (cascades as usize).min(cap));
                let (s, o) = (at(shadowed.0, shadowed.1), at(open.0, open.1));
                assert!(
                    luma(s) + 150 < luma(o),
                    "{tier:?} {cascades} cascades {filter:?}: shadowed {s:?} vs open {o:?}"
                );
            }
        }
    }
}

#[test]
fn first_directional_light_replaces_the_settings_sun() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    let plane = h.meshes.add(MeshData::plane(20.0));
    let mut world = World::new();
    world.spawn((Transform::IDENTITY, MeshRenderer::new(plane, Color::WHITE)));
    world.spawn(top_down_camera(8.0));
    let settings = RenderSettings {
        light_dir: Vec3::NEG_Y,
        light_color: Color::WHITE,
        ..Default::default()
    };
    let (_, at) = h.render(&world, &settings);
    let bright = at(SIZE / 2, SIZE / 2);
    // A green sun overrides the white settings sun.
    world.spawn((
        Transform::IDENTITY.looking_at(Vec3::NEG_Y, Vec3::Z),
        Light::directional([0.0, 1.0, 0.0], 1.0),
    ));
    let (_, at) = h.render(&world, &settings);
    let green = at(SIZE / 2, SIZE / 2);
    assert!(
        green[1] > green[0] + 60 && green[0] + 60 < bright[0],
        "{green:?} vs {bright:?}"
    );
    // Solid view: scene lights off, the settings sun again.
    let solid = RenderSettings {
        scene_lights: false,
        ..settings
    };
    let (_, at) = h.render(&world, &solid);
    assert_eq!(at(SIZE / 2, SIZE / 2), bright);
}
