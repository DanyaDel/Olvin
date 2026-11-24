//! Levels of detail: the farther an object, the fewer triangles it draws.

use super::*;
use crate::{DEFAULT_LODS, Vertex};

fn sphere(rings: u32, segments: u32) -> MeshData {
    let mut mesh = MeshData::default();
    for r in 0..=rings {
        let theta = std::f32::consts::PI * r as f32 / rings as f32;
        for s in 0..=segments {
            let phi = std::f32::consts::TAU * s as f32 / segments as f32;
            let n = Vec3::new(
                theta.sin() * phi.cos(),
                theta.cos(),
                theta.sin() * phi.sin(),
            );
            mesh.vertices.push(Vertex {
                position: n.into(),
                normal: n.into(),
                uv: [0.0, 0.0],
                tangent: [1.0, 0.0, 0.0, 1.0],
            });
        }
    }
    for r in 0..rings {
        for s in 0..segments {
            let a = r * (segments + 1) + s;
            let b = a + segments + 1;
            mesh.indices.extend([a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    mesh
}

fn triangles_at(h: &mut Harness, mesh: crate::MeshHandle, distance: f32, bias: f32) -> u64 {
    let mut world = World::new();
    world.spawn((
        Transform::IDENTITY,
        MeshRenderer::new(mesh, Color::rgb(1.0, 1.0, 1.0)),
    ));
    world.spawn((Transform::from_xyz(0.0, 0.0, distance), Camera::default()));
    let settings = RenderSettings {
        lod_bias: bias,
        ..Default::default()
    };
    h.render(&world, &settings).0.triangles
}

#[test]
fn far_objects_draw_a_simpler_copy() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    let full = sphere(32, 48);
    let full_triangles = (full.indices.len() / 3) as u64;
    let mesh = h.meshes.add_with_lods(full, &DEFAULT_LODS);

    let near = triangles_at(&mut h, mesh, 3.0, 1.0);
    let middle = triangles_at(&mut h, mesh, 10.0, 1.0);
    let far = triangles_at(&mut h, mesh, 40.0, 1.0);
    assert_eq!(near, full_triangles, "close up: everything");
    assert!(middle < near && middle > far, "{near} > {middle} > {far}");
    assert!(far < near / 3, "far is a fraction of near: {far} vs {near}");

    // Bias: infinity keeps the full mesh at any distance, 0 forces the lowest.
    assert_eq!(
        triangles_at(&mut h, mesh, 40.0, f32::INFINITY),
        full_triangles
    );
    assert_eq!(triangles_at(&mut h, mesh, 3.0, 0.0), far);
}

#[test]
fn replacing_the_geometry_drops_the_levels() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    let mesh = h.meshes.add_with_lods(sphere(32, 48), &DEFAULT_LODS);
    let before = triangles_at(&mut h, mesh, 40.0, 1.0);
    let other = sphere(24, 36);
    let other_triangles = (other.indices.len() / 3) as u64;
    h.meshes.set(mesh, other);
    let after = triangles_at(&mut h, mesh, 40.0, 1.0);
    assert!(before < other_triangles);
    assert_eq!(after, other_triangles, "the old copies no longer apply");
}
