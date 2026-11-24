use glam::Vec3;
use olvin_scene::Transform;

use super::*;
use crate::{MeshHandle, Vertex};

/// A flat strip of two vertices facing +Z.
fn strip() -> MeshData {
    MeshData {
        vertices: vec![
            Vertex::new([0.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            Vertex::new([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ],
        indices: vec![0, 1, 1],
    }
}

fn smile() -> MorphTarget {
    MorphTarget {
        name: "smile".into(),
        positions: vec![Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 2.0, 0.0)],
        normals: Vec::new(),
    }
}

fn tilt() -> MorphTarget {
    MorphTarget {
        name: "tilt".into(),
        positions: vec![Vec3::ZERO; 2],
        normals: vec![Vec3::new(0.0, 1.0, -1.0); 2],
    }
}

fn y(mesh: &MeshData, i: usize) -> f32 {
    mesh.vertices[i].position[1]
}

#[test]
fn weights_blend_the_offsets_in() {
    let mut m = MorphMesh::new(strip(), vec![smile(), tilt()], &[0.5]);
    assert_eq!(m.len(), 2);
    assert_eq!(m.weights, [0.5, 0.0]);
    assert_eq!((y(&m.morphed(), 0), y(&m.morphed(), 1)), (0.5, 1.0));
    assert!(m.set("smile", 1.0));
    assert!(!m.set("frown", 1.0), "no such target");
    assert!(!m.set_index(9, 1.0));
    assert!(!m.set_index(0, f32::NAN), "not a number is refused");
    assert_eq!(m.weight("smile"), Some(1.0));
    assert_eq!((y(&m.morphed(), 0), y(&m.morphed(), 1)), (1.0, 2.0));
    // Weights outside 0..1 are allowed: -1 pulls the other way.
    m.set("smile", -1.0);
    assert_eq!(y(&m.morphed(), 1), -2.0);
}

#[test]
fn normals_follow_their_offsets_and_stay_unit_length() {
    let mut m = MorphMesh::new(strip(), vec![smile(), tilt()], &[]);
    let untouched = m.morphed().vertices[0].normal;
    assert_eq!(untouched, [0.0, 0.0, 1.0]);
    m.set("tilt", 1.0);
    let n = Vec3::from(m.morphed().vertices[0].normal);
    // (0, 0, 1) + (0, 1, -1) = (0, 1, 0).
    assert!(n.abs_diff_eq(Vec3::Y, 1e-6), "{n:?}");
    m.set("tilt", 0.0);
    m.set("smile", 1.0);
    assert_eq!(
        m.morphed().vertices[0].normal,
        untouched,
        "a target without normals leaves them"
    );
}

#[test]
fn a_target_that_does_not_fit_is_dropped_and_the_weights_follow() {
    let bad = MorphTarget {
        name: "bad".into(),
        positions: vec![Vec3::ONE],
        normals: Vec::new(),
    };
    let m = MorphMesh::new(strip(), vec![bad, smile()], &[1.0, 0.25]);
    assert_eq!(m.names().collect::<Vec<_>>(), ["smile"]);
    assert_eq!(
        m.weights,
        [0.25],
        "the weight of the dropped target went with it"
    );
    assert_eq!(m.index_of("smile"), Some(0));
}

#[test]
fn tracks_blend_hold_and_clamp() {
    let linear = MorphTrack {
        target: 0,
        times: vec![0.0, 2.0],
        values: vec![0.0, 1.0],
        step: false,
    };
    assert_eq!(linear.sample(1.0), Some(0.5));
    assert_eq!(linear.sample(-3.0), Some(0.0));
    assert_eq!(linear.sample(9.0), Some(1.0));
    let step = MorphTrack {
        step: true,
        ..linear.clone()
    };
    assert_eq!(step.sample(1.9), Some(0.0));
    assert_eq!(step.sample(2.0), Some(1.0));
    let broken = MorphTrack {
        values: vec![1.0],
        ..linear
    };
    assert_eq!(broken.sample(1.0), None, "keys and values must match");
}

#[test]
fn a_player_loops_or_stops_at_the_end() {
    let clip = Arc::new(MorphClip::new(
        "blink",
        vec![MorphTrack {
            target: 1,
            times: vec![0.0, 1.0],
            values: vec![0.0, 1.0],
            step: false,
        }],
    ));
    assert_eq!(clip.duration, 1.0);
    let mut weights = [0.5, 0.0, 0.7];
    clip.sample(0.25, &mut weights);
    assert_eq!(weights, [0.5, 0.25, 0.7], "only tracks write");
    let mut once = MorphPlayer::new(clip.clone(), false);
    once.advance(0.75);
    once.advance(0.75);
    assert!(!once.playing && once.time == 1.0);
    let mut looped = MorphPlayer::new(clip, true);
    looped.advance(1.25);
    assert!(looped.playing && (looped.time - 0.25).abs() < 1e-6);
}

fn world_with(morph: MorphMesh) -> (Resources, Entity, MeshHandle) {
    let mut res = Resources::default();
    let mut world = World::new();
    let mut meshes = Meshes::default();
    let handle = meshes.add(strip());
    meshes.pending.clear();
    let e = world.spawn((
        Transform::IDENTITY,
        MeshRenderer::new(handle, crate::Color::WHITE),
        morph,
    ));
    res.insert(world);
    res.insert(meshes);
    (res, e, handle)
}

#[test]
fn update_reuploads_only_when_the_weights_changed() {
    let (mut res, e, handle) = world_with(MorphMesh::new(strip(), vec![smile()], &[]));
    update(&mut res);
    {
        let meshes = res.fetch::<Meshes>();
        assert_eq!(meshes.pending.len(), 1, "built once at the start");
        assert_eq!(meshes.pending[0].0, handle);
    }
    res.fetch_mut::<Meshes>().pending.clear();
    update(&mut res);
    assert!(res.fetch::<Meshes>().pending.is_empty(), "nothing changed");

    res.fetch::<World>()
        .get::<&mut MorphMesh>(e)
        .unwrap()
        .set("smile", 1.0);
    update(&mut res);
    let meshes = res.fetch::<Meshes>();
    assert_eq!(meshes.pending.len(), 1);
    assert_eq!(y(&meshes.pending[0].1, 1), 2.0);
}

#[test]
fn a_player_drives_the_weights_through_update() {
    let (mut res, e, _) = world_with(MorphMesh::new(strip(), vec![smile()], &[]));
    res.insert(Time::with_fixed_delta(0.5));
    let clip = Arc::new(MorphClip::new(
        "grin",
        vec![MorphTrack {
            target: 0,
            times: vec![0.0, 1.0],
            values: vec![0.0, 1.0],
            step: false,
        }],
    ));
    res.fetch_mut::<World>()
        .insert_one(e, MorphPlayer::new(clip, false))
        .unwrap();
    res.fetch_mut::<Time>().advance(0.5);
    update(&mut res);
    let w = res.fetch::<World>().get::<&MorphMesh>(e).unwrap().weights[0];
    assert!((w - 0.5).abs() < 1e-6, "{w}");
}

#[test]
fn a_skinned_mesh_gets_the_blended_bind_pose() {
    let skin = SkinnedMesh::new(
        strip(),
        vec![[0, 0, 0, 0]; 2],
        vec![[1.0, 0.0, 0.0, 0.0]; 2],
        Entity::DANGLING,
    )
    .unwrap();
    let (mut res, e, _) = world_with(MorphMesh::new(strip(), vec![smile()], &[1.0]));
    res.fetch_mut::<World>().insert_one(e, skin).unwrap();
    update(&mut res);
    let world = res.fetch::<World>();
    let bind = world.get::<&SkinnedMesh>(e).unwrap().bind_mesh();
    assert_eq!((y(&bind, 0), y(&bind, 1)), (1.0, 2.0));
    assert!(
        res.fetch::<Meshes>().pending.is_empty(),
        "the pose uploads it, not the morph"
    );
}
