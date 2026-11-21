//! Skinned meshes. Skinning runs on the CPU every time the [`Animator`]
//! changes and the result goes into the mesh slot with [`Meshes::set`]. That
//! works on every tier, including WebGL2 / GLES3 (no storage buffers or
//! bone textures needed), and the shadow and blend passes draw the posed
//! mesh like any other. Keep skinned meshes modest; one mesh slot per
//! skinned instance (they are not shared, because each is posed on its own).

use glam::{Mat4, Vec3, Vec4};
use olvin_anim::Animator;
use olvin_core::Resources;
use olvin_ecs::{Entity, World};

use crate::{MeshData, MeshRenderer, Meshes, Vertex};

/// Marks the entity's mesh (its [`MeshRenderer`]) as skinned by `animator`.
/// Put it on a unique mesh whose bind-pose data it keeps; the entity sits at
/// the model's root (glTF skinned nodes ignore their own transform).
#[derive(Clone, Debug)]
pub struct SkinnedMesh {
    /// Entity with the [`Animator`] (may be this one).
    pub animator: Entity,
    bind: MeshData,
    joints: Vec<[u16; 4]>,
    weights: Vec<[f32; 4]>,
    last_version: Option<u64>,
}

impl SkinnedMesh {
    /// `bind` is the mesh in bind pose, `joints` / `weights` hold four
    /// influences per vertex (joint indices of the animator's skeleton).
    pub fn new(
        bind: MeshData,
        joints: Vec<[u16; 4]>,
        weights: Vec<[f32; 4]>,
        animator: Entity,
    ) -> Result<Self, String> {
        let n = bind.vertices.len();
        if joints.len() != n || weights.len() != n {
            return Err(format!(
                "{n} vertices but {} joint sets and {} weight sets",
                joints.len(),
                weights.len()
            ));
        }
        Ok(Self {
            animator,
            bind,
            joints,
            weights,
            last_version: None,
        })
    }

    /// The bind-pose mesh to register with [`Meshes::add`].
    pub fn bind_mesh(&self) -> MeshData {
        self.bind.clone()
    }

    /// The mesh posed by `matrices` (`Skeleton::skin_matrices`). Influences
    /// of joints that do not exist are dropped; a vertex without any keeps
    /// its bind position.
    pub fn skin(&self, matrices: &[Mat4]) -> MeshData {
        let vertices = self
            .bind
            .vertices
            .iter()
            .zip(self.joints.iter().zip(&self.weights))
            .map(|(v, (joints, weights))| {
                let mut m = Mat4::ZERO;
                let mut total = 0.0;
                for (&j, &w) in joints.iter().zip(weights) {
                    if let (Some(jm), true) = (matrices.get(j as usize), w > 0.0) {
                        m += *jm * w;
                        total += w;
                    }
                }
                if total <= 1e-6 {
                    return *v;
                }
                let m = m * (1.0 / total);
                let dir = |d: [f32; 3]| m.transform_vector3(Vec3::from(d)).normalize_or_zero();
                Vertex {
                    position: m.transform_point3(Vec3::from(v.position)).into(),
                    normal: dir(v.normal).into(),
                    uv: v.uv,
                    tangent: Vec4::from((
                        dir([v.tangent[0], v.tangent[1], v.tangent[2]]),
                        v.tangent[3],
                    ))
                    .into(),
                }
            })
            .collect();
        MeshData {
            vertices,
            indices: self.bind.indices.clone(),
        }
    }
}

/// Re-poses every skinned mesh whose animator changed since last time
/// (`RenderDataPlugin` runs it in `PostUpdate`).
pub fn skin_meshes(res: &mut Resources) {
    let world = res.fetch::<World>();
    let mut meshes = res.fetch_mut::<Meshes>();
    let jobs: Vec<(Entity, crate::MeshHandle, Entity)> = world
        .query::<(Entity, (&SkinnedMesh, &MeshRenderer))>()
        .iter()
        .map(|(e, (s, r))| (e, r.mesh, s.animator))
        .collect();
    for (e, handle, animator) in jobs {
        let Ok(anim) = world.get::<&Animator>(animator) else {
            continue;
        };
        let Ok(skin) = world.get::<&SkinnedMesh>(e) else {
            continue;
        };
        if skin.last_version == Some(anim.version) {
            continue;
        }
        meshes.set(handle, skin.skin(&anim.matrices));
        let version = anim.version;
        drop((skin, anim));
        if let Ok(mut skin) = world.get::<&mut SkinnedMesh>(e) {
            skin.last_version = Some(version);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use glam::Quat;
    use olvin_anim::{Joint, JointPose, Skeleton};

    use super::*;

    /// Root at the origin with Tip above it; a vertical strip skinned to them.
    fn bar() -> (Skeleton, SkinnedMesh) {
        let joint = |name: &str, parent, target, y| Joint {
            name: name.into(),
            parent,
            target,
            rest: JointPose {
                translation: Vec3::new(0.0, y, 0.0),
                ..JointPose::IDENTITY
            },
            inverse_bind: Mat4::from_translation(Vec3::new(0.0, -y, 0.0)),
        };
        let skeleton = Skeleton::new(vec![
            joint("root", None, 0, 0.0),
            joint("tip", Some(0), 1, 1.0),
        ])
        .unwrap();
        let v = |x: f32, y: f32| Vertex::new([x, y, 0.0], [0.0, 0.0, 1.0]);
        let bind = MeshData {
            vertices: vec![v(0.0, 0.0), v(1.0, 0.0), v(0.0, 1.0), v(1.0, 1.0)],
            indices: vec![0, 1, 2, 1, 3, 2],
        };
        let joints = vec![[0; 4], [0; 4], [1, 0, 0, 0], [1, 0, 0, 0]];
        let weights = vec![[1.0, 0.0, 0.0, 0.0]; 4];
        let mesh = SkinnedMesh::new(bind, joints, weights, Entity::DANGLING).unwrap();
        (skeleton, mesh)
    }

    #[test]
    fn rest_pose_keeps_the_bind_mesh() {
        let (sk, mesh) = bar();
        let posed = mesh.skin(&sk.skin_matrices(&sk.rest_pose()));
        for (a, b) in posed.vertices.iter().zip(&mesh.bind.vertices) {
            assert!(Vec3::from(a.position).abs_diff_eq(Vec3::from(b.position), 1e-6));
        }
    }

    #[test]
    fn bending_the_tip_moves_only_its_vertices_and_turns_normals() {
        let (sk, mesh) = bar();
        let mut pose = sk.rest_pose();
        pose.joints[1].rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let posed = mesh.skin(&sk.skin_matrices(&pose));
        let p = |i: usize| Vec3::from(posed.vertices[i].position);
        assert!(p(0).abs_diff_eq(Vec3::ZERO, 1e-6) && p(1).abs_diff_eq(Vec3::X, 1e-6));
        // The tip's vertices turn about (0, 1, 0): (1, 1) goes to (0, 2).
        assert!(p(2).abs_diff_eq(Vec3::Y, 1e-5), "{:?}", p(2));
        assert!(
            p(3).abs_diff_eq(Vec3::new(0.0, 2.0, 0.0), 1e-5),
            "{:?}",
            p(3)
        );
        // Normals of the tip stay unit and (rotation about Z) keep pointing at +Z.
        let n = Vec3::from(posed.vertices[3].normal);
        assert!((n.length() - 1.0).abs() < 1e-5 && n.abs_diff_eq(Vec3::Z, 1e-5));
    }

    #[test]
    fn weights_blend_and_bad_joints_fall_back() {
        let (sk, mut mesh) = bar();
        let mut pose = sk.rest_pose();
        pose.joints[0].translation = Vec3::new(2.0, 0.0, 0.0);
        let m = sk.skin_matrices(&pose);
        // Vertex 0: half on the moved root, half on the still tip joint... which
        // moves with its parent, so both influences shift it by 2.
        mesh.joints[0] = [0, 1, 0, 0];
        mesh.weights[0] = [0.5, 0.5, 0.0, 0.0];
        let p = Vec3::from(mesh.skin(&m).vertices[0].position);
        assert!(p.abs_diff_eq(Vec3::new(2.0, 0.0, 0.0), 1e-5), "{p:?}");
        // Influence of a missing joint is dropped; with nothing left: bind pose.
        mesh.joints[1] = [9, 0, 0, 0];
        mesh.weights[1] = [1.0, 0.0, 0.0, 0.0];
        let q = Vec3::from(mesh.skin(&m).vertices[1].position);
        assert!(q.abs_diff_eq(Vec3::X, 1e-6), "{q:?}");
    }

    #[test]
    fn length_mismatch_is_an_error() {
        let (_, mesh) = bar();
        let err = SkinnedMesh::new(mesh.bind.clone(), vec![[0; 4]], vec![], Entity::DANGLING);
        assert!(err.is_err());
    }

    #[test]
    fn system_reposes_the_mesh_once_per_animator_change() {
        use olvin_anim::{Channel, Clip, Interpolation, Property};
        let (sk, mesh) = bar();
        let clip = Clip::new(
            "slide",
            vec![Channel {
                target: 0,
                property: Property::Translation,
                interpolation: Interpolation::Linear,
                times: vec![0.0, 1.0],
                values: vec![Vec4::ZERO, Vec4::new(1.0, 0.0, 0.0, 0.0)],
            }],
        );
        let mut animator = Animator::looping(Arc::new(sk), vec![clip].into(), 0);
        animator.update(0.5);
        let mut res = Resources::default();
        let mut world = World::new();
        let mut meshes = Meshes::default();
        let handle = meshes.add(mesh.bind_mesh());
        let a = world.spawn((animator,));
        let skin = SkinnedMesh {
            animator: a,
            ..mesh
        };
        let e = world.spawn((MeshRenderer::new(handle, crate::Color::WHITE), skin));
        meshes.pending.clear();
        res.insert(world);
        res.insert(meshes);

        skin_meshes(&mut res);
        {
            let meshes = res.fetch::<Meshes>();
            assert_eq!(meshes.pending.len(), 1);
            // Half a metre of slide at t = 0.5.
            let x = meshes.pending[0].1.vertices[0].position[0];
            assert!((x - 0.5).abs() < 1e-5, "{x}");
        }
        res.fetch_mut::<Meshes>().pending.clear();
        skin_meshes(&mut res);
        assert!(
            res.fetch::<Meshes>().pending.is_empty(),
            "unchanged animator: no re-skin"
        );

        res.fetch_mut::<World>()
            .get::<&mut Animator>(a)
            .unwrap()
            .update(0.25);
        skin_meshes(&mut res);
        assert_eq!(res.fetch::<Meshes>().pending.len(), 1);
        assert!(res.fetch::<World>().get::<&SkinnedMesh>(e).is_ok());
    }
}
