use std::collections::HashMap;

use glam::{Mat4, Quat, Vec3};

/// Local translation / rotation / scale of one joint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointPose {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl JointPose {
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }

    /// Lerp for translation and scale, shortest-arc slerp for rotation.
    pub fn lerp(&self, other: &Self, t: f32) -> Self {
        Self {
            translation: self.translation.lerp(other.translation, t),
            rotation: self.rotation.slerp(other.rotation, t),
            scale: self.scale.lerp(other.scale, t),
        }
    }
}

impl From<Mat4> for JointPose {
    fn from(m: Mat4) -> Self {
        let (scale, rotation, translation) = m.to_scale_rotation_translation();
        Self {
            translation,
            rotation,
            scale,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Joint {
    pub name: String,
    /// Index into [`Skeleton::joints`]; `None` for a root. Any order is fine.
    pub parent: Option<usize>,
    /// What [`Channel::target`](crate::Channel) refers to (a node id in the
    /// source file); several skeletons of one model share clips through it.
    pub target: usize,
    /// Local pose in the bind (rest) state. Roots include the transform of
    /// everything above the skeleton, so poses are model-space.
    pub rest: JointPose,
    /// Model space (bind pose) to joint space.
    pub inverse_bind: Mat4,
}

/// Local poses of every joint, parallel to [`Skeleton::joints`].
#[derive(Clone, Debug, PartialEq)]
pub struct Pose {
    pub joints: Vec<JointPose>,
}

impl Pose {
    /// Moves towards `other` by `weight` (0 = unchanged, 1 = `other`).
    pub fn blend(&mut self, other: &Pose, weight: f32) {
        for (a, b) in self.joints.iter_mut().zip(&other.joints) {
            *a = a.lerp(b, weight);
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Skeleton {
    pub joints: Vec<Joint>,
    by_target: HashMap<usize, usize>,
}

impl Skeleton {
    /// Fails when a parent index is out of range or the parents form a cycle.
    pub fn new(joints: Vec<Joint>) -> Result<Self, String> {
        let n = joints.len();
        for (i, j) in joints.iter().enumerate() {
            if j.parent.is_some_and(|p| p >= n) {
                return Err(format!("joint {} ({}): parent out of range", i, j.name));
            }
            let (mut cur, mut steps) = (j.parent, 0);
            while let Some(p) = cur {
                steps += 1;
                if steps > n {
                    return Err(format!("joint {} ({}): parent cycle", i, j.name));
                }
                cur = joints[p].parent;
            }
        }
        let by_target = joints
            .iter()
            .enumerate()
            .map(|(i, j)| (j.target, i))
            .collect();
        Ok(Self { joints, by_target })
    }

    pub fn len(&self) -> usize {
        self.joints.len()
    }

    pub fn is_empty(&self) -> bool {
        self.joints.is_empty()
    }

    pub fn joint_for_target(&self, target: usize) -> Option<usize> {
        self.by_target.get(&target).copied()
    }

    pub fn rest_pose(&self) -> Pose {
        Pose {
            joints: self.joints.iter().map(|j| j.rest).collect(),
        }
    }

    /// Model-space matrix of every joint.
    pub fn globals(&self, pose: &Pose) -> Vec<Mat4> {
        let mut out: Vec<Option<Mat4>> = vec![None; self.joints.len()];
        let mut chain = Vec::new();
        for start in 0..self.joints.len() {
            // Walk up to the first resolved ancestor, then back down.
            let mut cur = Some(start);
            while let Some(i) = cur {
                if out[i].is_some() {
                    break;
                }
                chain.push(i);
                cur = self.joints[i].parent;
            }
            while let Some(i) = chain.pop() {
                let local = pose.joints.get(i).map_or(Mat4::IDENTITY, JointPose::matrix);
                out[i] = Some(match self.joints[i].parent {
                    Some(p) => out[p].unwrap_or(Mat4::IDENTITY) * local,
                    None => local,
                });
            }
        }
        out.into_iter()
            .map(|m| m.unwrap_or(Mat4::IDENTITY))
            .collect()
    }

    /// Skinning matrices (`global * inverse_bind`): they take a bind-pose
    /// model-space vertex to its posed position.
    pub fn skin_matrices(&self, pose: &Pose) -> Vec<Mat4> {
        self.globals(pose)
            .into_iter()
            .zip(&self.joints)
            .map(|(g, j)| g * j.inverse_bind)
            .collect()
    }
}
