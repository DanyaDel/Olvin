//! Alpha blending.

use super::*;

/// Flat colour with alpha, drawn in the blend pass when `blend`.
fn tinted(rgb: [f32; 3], alpha: f32, blend: bool) -> Material {
    Material {
        base_color: Color([rgb[0], rgb[1], rgb[2], alpha]),
        unlit: true,
        blend: if blend {
            BlendMode::Alpha
        } else {
            BlendMode::Opaque
        },
        ..Default::default()
    }
}

fn layer(h: &mut Harness, world: &mut World, y: f32, material: Material) {
    let plane = h.meshes.add(MeshData::plane(4.0));
    let material = h.materials.add(material);
    world.spawn((
        Transform::from_xyz(0.0, y, 0.0),
        MeshRenderer::new(plane, Color::WHITE).with_material(material),
    ));
}

#[test]
fn blended_material_mixes_with_what_is_behind() {
    for editor_like in [false, true] {
        let Some(mut h) = Harness::new(editor_like) else {
            return;
        };
        let mut world = World::new();
        world.spawn(top_down_camera(8.0));
        layer(&mut h, &mut world, 0.0, tinted([1.0, 0.0, 0.0], 1.0, false));
        layer(&mut h, &mut world, 1.0, tinted([0.0, 1.0, 0.0], 0.5, true));
        let (stats, at) = h.render(&world, &flat_settings());
        assert_eq!(stats.draw_calls, 2);
        // Half green over red: blended in linear light on an sRGB target
        // (188 = sRGB of 0.5), in encoded values on the editor's non-sRGB one.
        let mid = at(SIZE / 2, SIZE / 2);
        if editor_like {
            assert!(
                mid[0].abs_diff(mid[1]) < 12 && mid[0] > 100 && mid[2] < 10,
                "{mid:?}"
            );
        } else {
            assert!(close(mid, [188, 188, 0]), "{mid:?}");
        }

        // Behind an opaque surface the blended one is hidden (depth test).
        let mut world = World::new();
        world.spawn(top_down_camera(8.0));
        layer(&mut h, &mut world, 0.0, tinted([1.0, 0.0, 0.0], 1.0, false));
        layer(&mut h, &mut world, -1.0, tinted([0.0, 1.0, 0.0], 0.5, true));
        let (_, at) = h.render(&world, &flat_settings());
        let hidden = at(SIZE / 2, SIZE / 2);
        assert!(hidden[0] > 200 && hidden[1] < 10, "{hidden:?}");
    }
}

#[test]
fn blended_material_with_a_surface_shader_gets_its_own_blend_pipeline() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    let shader = h.shaders.add(
        "fn surface(in: SurfaceInput) -> SurfaceOutput {\n    var out = default_surface(in);\n    \
         out.color = vec4<f32>(0.0, 1.0, 0.0, 0.5);\n    return out;\n}\n"
            .into(),
    );
    let mut world = World::new();
    world.spawn(top_down_camera(8.0));
    layer(&mut h, &mut world, 0.0, tinted([1.0, 0.0, 0.0], 1.0, false));
    layer(
        &mut h,
        &mut world,
        1.0,
        Material {
            shader: Some(shader),
            unlit: false,
            ..tinted([1.0; 3], 1.0, true)
        },
    );
    let settings = RenderSettings {
        // Lit by a white sun straight down: green stays green, half-covered.
        light_dir: Vec3::NEG_Y,
        light_color: Color::WHITE,
        ..flat_settings()
    };
    let (_, at) = h.render(&world, &settings);
    let mid = at(SIZE / 2, SIZE / 2);
    assert_eq!(h.shaders.error(shader), None);
    assert!(
        mid[0] > 40 && mid[1] > 40,
        "red below, green above: {mid:?}"
    );
}

#[test]
fn blended_layers_sort_back_to_front_whatever_the_spawn_order() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    let red = tinted([1.0, 0.0, 0.0], 0.5, true);
    let green = tinted([0.0, 1.0, 0.0], 0.5, true);
    let mut pixels = Vec::new();
    // Near layer (y = 2, green) over far layer (y = 1, red), both orders.
    for near_first in [true, false] {
        let mut world = World::new();
        world.spawn(top_down_camera(8.0));
        if near_first {
            layer(&mut h, &mut world, 2.0, green);
            layer(&mut h, &mut world, 1.0, red);
        } else {
            layer(&mut h, &mut world, 1.0, red);
            layer(&mut h, &mut world, 2.0, green);
        }
        let (_, at) = h.render(&world, &flat_settings());
        pixels.push(at(SIZE / 2, SIZE / 2));
    }
    assert_eq!(pixels[0], pixels[1]);
    // The near green dominates the far red: g = 0.5, r = 0.25 in linear light.
    let [r, g, _] = pixels[0];
    assert!(g > r + 40, "{:?}", pixels[0]);
}
