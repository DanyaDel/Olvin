//! Skinned meshes. One [`SkinnedMesh`] API, two ways to pose it, picked by
//! [`RenderCore::tier`](crate::RenderCore::tier) when the mesh is uploaded:
//!
//! * **Full tier (native APIs):** a compute pass skins the vertices on the GPU
//!   (joint matrices go to a storage buffer, nothing is rewritten on the
//!   CPU) straight into the mesh's own vertex buffer, so the main, blend and
//!   shadow passes and custom surface shaders draw the posed mesh like any
//!   other. The culling bound comes from per-joint bounds of the bind pose
//!   (conservative, no vertex is touched on the CPU).
//! * **Compat tier (WebGL2 / GLES3):** CPU skinning, as before: the posed
//!   [`MeshData`] is re-uploaded into the same slot.
//!
//! [`skin_meshes`] only records the animator's matrices ([`Meshes::skin`]);
//! the renderer decides what to do with them. One mesh slot per skinned
//! instance (they are not shared, because each is posed on its own).

use std::sync::Arc;

use glam::{Mat4, Vec3, Vec4};
use olvin_anim::Animator;
use olvin_core::Resources;
use olvin_ecs::{Entity, World};

use crate::{MeshData, MeshHandle, MeshRenderer, Meshes, Vertex};

/// Bind-pose data shared by clones of a [`SkinnedMesh`] (and read by the
/// renderer's GPU skinning).
#[derive(Clone, Debug)]
pub(crate) struct SkinSource {
    pub(crate) bind: MeshData,
    pub(crate) joints: Vec<[u16; 4]>,
    pub(crate) weights: Vec<[f32; 4]>,
    /// Bind-pose bounding sphere (centre, radius) of the vertices each joint
    /// influences; `None` for a joint nothing uses.
    joint_bounds: Vec<Option<(Vec3, f32)>>,
    /// Bounds of the vertices without any influence (they never move).
    loose_bounds: Option<(Vec3, f32)>,
}

impl SkinSource {
    fn new(bind: MeshData, joints: Vec<[u16; 4]>, weights: Vec<[f32; 4]>) -> Self {
        let count = joints
            .iter()
            .zip(&weights)
            .flat_map(|(j, w)| j.iter().zip(w))
            .filter(|(_, w)| **w > 0.0)
            .map(|(j, _)| *j as usize + 1)
            .max()
            .unwrap_or(0);
        let mut boxes: Vec<Option<(Vec3, Vec3)>> = vec![None; count];
        let mut loose: Option<(Vec3, Vec3)> = None;
        let grow = |b: &mut Option<(Vec3, Vec3)>, p: Vec3| {
            *b = Some(b.map_or((p, p), |(lo, hi)| (lo.min(p), hi.max(p))));
        };
        for (v, (j, w)) in bind.vertices.iter().zip(joints.iter().zip(&weights)) {
            let p = Vec3::from(v.position);
            let mut any = false;
            for (&j, &w) in j.iter().zip(w) {
                if w > 0.0 {
                    grow(&mut boxes[j as usize], p);
                    any = true;
                }
            }
            if !any {
                grow(&mut loose, p);
            }
        }
        let sphere = |(lo, hi): (Vec3, Vec3)| ((lo + hi) * 0.5, (hi - lo).length() * 0.5);
        Self {
            bind,
            joints,
            weights,
            joint_bounds: boxes.into_iter().map(|b| b.map(sphere)).collect(),
            loose_bounds: loose.map(sphere),
        }
    }

    /// A sphere (centre, radius) around the mesh posed by `matrices`: every
    /// vertex is a convex blend of its joints' transformed vertices, so the
    /// hull of the per-joint bind spheres moved by their matrices holds it.
    /// Joints without a matrix keep their bind sphere.
    pub(crate) fn posed_bounds(&self, matrices: &[Mat4]) -> (Vec3, f32) {
        let mut spheres: Vec<(Vec3, f32)> = Vec::with_capacity(self.joint_bounds.len() + 1);
        spheres.extend(self.loose_bounds);
        for (j, b) in self.joint_bounds.iter().enumerate() {
            let Some((c, r)) = *b else { continue };
            spheres.push(match matrices.get(j) {
                Some(m) => {
                    let m3 = glam::Mat3::from_mat4(*m);
                    // Frobenius norm >= the largest scale of the matrix.
                    let scale = (m3.x_axis.length_squared()
                        + m3.y_axis.length_squared()
                        + m3.z_axis.length_squared())
                    .sqrt();
                    (m.transform_point3(c), r * scale)
                }
                None => (c, r),
            });
        }
        let Some(first) = spheres.first() else {
            return (Vec3::ZERO, 0.0);
        };
        let (lo, hi) = spheres.iter().fold(
            (
                first.0 - Vec3::splat(first.1),
                first.0 + Vec3::splat(first.1),
            ),
            |(lo, hi), (c, r)| (lo.min(*c - Vec3::splat(*r)), hi.max(*c + Vec3::splat(*r))),
        );
        let center = (lo + hi) * 0.5;
        let radius = spheres
            .iter()
            .map(|(c, r)| c.distance(center) + r)
            .fold(0.0, f32::max);
        (center, radius)
    }

    pub(crate) fn skin(&self, matrices: &[Mat4]) -> MeshData {
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

/// A pose waiting for the renderer: the matrices to skin `source` with into
/// mesh slot `handle` (see [`Meshes::skin`]).
pub(crate) struct SkinUpdate {
    pub(crate) handle: MeshHandle,
    pub(crate) source: Arc<SkinSource>,
    pub(crate) matrices: Vec<Mat4>,
}

/// Marks the entity's mesh (its [`MeshRenderer`]) as skinned by `animator`.
/// Put it on a unique mesh whose bind-pose data it keeps (register
/// [`bind_mesh`](Self::bind_mesh) with [`Meshes::add`]); the entity sits at
/// the model's root (glTF skinned nodes ignore their own transform).
#[derive(Clone, Debug)]
pub struct SkinnedMesh {
    /// Entity with the [`Animator`] (may be this one).
    pub animator: Entity,
    source: Arc<SkinSource>,
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
            source: Arc::new(SkinSource::new(bind, joints, weights)),
            last_version: None,
        })
    }

    /// The bind-pose mesh to register with [`Meshes::add`].
    pub fn bind_mesh(&self) -> MeshData {
        self.source.bind.clone()
    }

    /// The mesh posed by `matrices` (`Skeleton::skin_matrices`) on the CPU.
    /// Influences of joints that do not exist are dropped; a vertex without
    /// any keeps its bind position. This is also what the Compat tier does.
    pub fn skin(&self, matrices: &[Mat4]) -> MeshData {
        self.source.skin(matrices)
    }

    /// Poses mesh slot `handle` with `matrices` unless `version` (the
    /// animator's) was already applied; returns whether it queued a pose.
    /// [`skin_meshes`] does this for every skinned entity; callers that own
    /// their animators (the editor's preview) use it directly.
    pub fn pose(
        &mut self,
        meshes: &mut Meshes,
        handle: MeshHandle,
        version: u64,
        matrices: &[Mat4],
    ) -> bool {
        if self.last_version == Some(version) {
            return false;
        }
        meshes.skin(handle, self, matrices);
        self.last_version = Some(version);
        true
    }

    /// Replaces the bind-pose geometry (a morphed bind pose) and forgets the
    /// applied pose, so the next [`pose`](Self::pose) skins the new one.
    /// Joints, weights and the vertex count stay.
    pub(crate) fn set_bind(&mut self, bind: MeshData) {
        if bind.vertices.len() != self.source.bind.vertices.len() {
            return;
        }
        self.source = Arc::new(SkinSource::new(
            bind,
            self.source.joints.clone(),
            self.source.weights.clone(),
        ));
        self.last_version = None;
    }

    /// Forgets the applied animator version, so the next [`pose`](Self::pose)
    /// poses again (a new animator started counting from scratch).
    pub fn invalidate(&mut self) {
        self.last_version = None;
    }
}

impl Meshes {
    /// Queues a pose for the skinned mesh in slot `handle` (replacing an
    /// earlier one that was not uploaded yet). The renderer skins it on the
    /// GPU (Full tier) or on the CPU (Compat) at the next `upload`.
    pub fn skin(&mut self, handle: MeshHandle, skin: &SkinnedMesh, matrices: &[Mat4]) {
        self.skin_updates.retain(|u| u.handle != handle);
        self.skin_updates.push(SkinUpdate {
            handle,
            source: skin.source.clone(),
            matrices: matrices.to_vec(),
        });
    }
}

impl Meshes {
    /// Poses queued by [`Meshes::skin`] that the renderer has not taken yet.
    pub fn skin_updates_len(&self) -> usize {
        self.skin_updates.len()
    }

    /// Drops the queued poses (for tests that look at what a system queued).
    pub fn clear_skin_updates(&mut self) {
        self.skin_updates.clear();
    }
}

/// Queues the pose of every skinned mesh whose animator changed since last
/// time (`RenderDataPlugin` runs it in `PostUpdate`).
pub fn skin_meshes(res: &mut Resources) {
    // Morph targets first: a skinned mesh is posed from its blended bind pose.
    crate::morph::update(res);
    let world = res.fetch::<World>();
    let mut meshes = res.fetch_mut::<Meshes>();
    let jobs: Vec<(Entity, MeshHandle, Entity)> = world
        .query::<(Entity, (&SkinnedMesh, &MeshRenderer))>()
        .iter()
        .map(|(e, (s, r))| (e, r.mesh, s.animator))
        .collect();
    for (e, handle, animator) in jobs {
        let Ok(anim) = world.get::<&Animator>(animator) else {
            continue;
        };
        let Ok(mut skin) = world.get::<&mut SkinnedMesh>(e) else {
            continue;
        };
        skin.pose(&mut meshes, handle, anim.version, &anim.matrices);
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
        for (a, b) in posed.vertices.iter().zip(&mesh.source.bind.vertices) {
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
        let src = Arc::make_mut(&mut mesh.source);
        src.joints[0] = [0, 1, 0, 0];
        src.weights[0] = [0.5, 0.5, 0.0, 0.0];
        let p = Vec3::from(mesh.skin(&m).vertices[0].position);
        assert!(p.abs_diff_eq(Vec3::new(2.0, 0.0, 0.0), 1e-5), "{p:?}");
        // Influence of a missing joint is dropped; with nothing left: bind pose.
        let src = Arc::make_mut(&mut mesh.source);
        src.joints[1] = [9, 0, 0, 0];
        src.weights[1] = [1.0, 0.0, 0.0, 0.0];
        let q = Vec3::from(mesh.skin(&m).vertices[1].position);
        assert!(q.abs_diff_eq(Vec3::X, 1e-6), "{q:?}");
    }

    #[test]
    fn length_mismatch_is_an_error() {
        let (_, mesh) = bar();
        let err = SkinnedMesh::new(
            mesh.source.bind.clone(),
            vec![[0; 4]],
            vec![],
            Entity::DANGLING,
        );
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
            assert_eq!(meshes.skin_updates.len(), 1);
            // Half a metre of slide at t = 0.5.
            let u = &meshes.skin_updates[0];
            let x = u.source.skin(&u.matrices).vertices[0].position[0];
            assert!((x - 0.5).abs() < 1e-5, "{x}");
        }
        res.fetch_mut::<Meshes>().skin_updates.clear();
        skin_meshes(&mut res);
        assert!(
            res.fetch::<Meshes>().skin_updates.is_empty(),
            "unchanged animator: no re-skin"
        );

        res.fetch_mut::<World>()
            .get::<&mut Animator>(a)
            .unwrap()
            .update(0.25);
        skin_meshes(&mut res);
        assert_eq!(res.fetch::<Meshes>().skin_updates.len(), 1);
        assert!(res.fetch::<World>().get::<&SkinnedMesh>(e).is_ok());
    }
}
