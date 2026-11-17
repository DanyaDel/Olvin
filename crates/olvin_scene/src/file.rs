//! Scene files (`*.scene.ron`): what the editor saves and games load.
//! Plain descriptions, independent of the ECS and the renderer.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use glam::{EulerRot, Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::{Camera, Transform};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SceneFile {
    /// Parents always come before their children; sibling order is display order.
    pub entities: Vec<EntityDesc>,
    /// Data owned by editor plugins (e.g. animation clips), keyed by plugin id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntityDesc {
    /// Stable within the scene; references (parent, animation tracks) use it.
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub parent: Option<u64>,
    pub position: Vec3,
    /// Euler angles in degrees, applied Y, then X, then Z.
    pub rotation: Vec3,
    pub scale: Vec3,
    #[serde(default)]
    pub mesh: Option<MeshDesc>,
    #[serde(default)]
    pub camera: Option<CameraDesc>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Shape {
    Cube,
    Plane,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeshDesc {
    pub shape: Shape,
    /// Linear RGBA.
    pub color: [f32; 4],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CameraDesc {
    pub fov_deg: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for CameraDesc {
    /// Same as `Camera::default()`, in round degrees.
    fn default() -> Self {
        Self { fov_deg: 60.0, near: 0.1, far: 1000.0 }
    }
}

impl From<&CameraDesc> for Camera {
    fn from(c: &CameraDesc) -> Self {
        Camera { fov_y: c.fov_deg.to_radians(), near: c.near, far: c.far }
    }
}

pub fn euler_to_quat(degrees: Vec3) -> Quat {
    Quat::from_euler(
        EulerRot::YXZ,
        degrees.y.to_radians(),
        degrees.x.to_radians(),
        degrees.z.to_radians(),
    )
}

impl EntityDesc {
    pub fn new(id: u64, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            parent: None,
            position: Vec3::ZERO,
            rotation: Vec3::ZERO,
            scale: Vec3::ONE,
            mesh: None,
            camera: None,
        }
    }

    pub fn local_transform(&self) -> Transform {
        Transform {
            translation: self.position,
            rotation: euler_to_quat(self.rotation),
            scale: self.scale,
        }
    }
}

impl SceneFile {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        ron::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let path = path.as_ref();
        let config = ron::ser::PrettyConfig::new().struct_names(false);
        let text = ron::ser::to_string_pretty(self, config).map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn get(&self, id: u64) -> Option<&EntityDesc> {
        self.entities.iter().find(|e| e.id == id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut EntityDesc> {
        self.entities.iter_mut().find(|e| e.id == id)
    }

    /// World matrices of all entities, parents resolved.
    pub fn world_matrices(&self) -> HashMap<u64, Mat4> {
        self.world_matrices_with(|e| e.local_transform())
    }

    /// Same, with a custom local transform (e.g. an animation preview pose).
    pub fn world_matrices_with(&self, local: impl Fn(&EntityDesc) -> Transform) -> HashMap<u64, Mat4> {
        let mut out = HashMap::with_capacity(self.entities.len());
        // Parents precede children, but tolerate hand-edited files with one extra pass.
        for _ in 0..2 {
            for e in &self.entities {
                let parent = match e.parent {
                    None => Mat4::IDENTITY,
                    Some(p) => match out.get(&p) {
                        Some(m) => *m,
                        None => continue,
                    },
                };
                out.insert(e.id, parent * local(e).matrix());
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_parenting() {
        let mut scene = SceneFile::default();
        let mut parent = EntityDesc::new(1, "parent");
        parent.position = Vec3::new(10.0, 0.0, 0.0);
        parent.rotation = Vec3::new(0.0, 90.0, 0.0);
        let mut child = EntityDesc::new(2, "child");
        child.parent = Some(1);
        child.position = Vec3::new(0.0, 0.0, -1.0);
        child.mesh = Some(MeshDesc { shape: Shape::Cube, color: [1.0; 4] });
        scene.entities = vec![parent, child];

        let text = ron::to_string(&scene).unwrap();
        let back: SceneFile = ron::from_str(&text).unwrap();
        assert_eq!(back, scene);

        // Child sits 1 unit "forward" of a parent turned 90° left: at x = 9.
        let world = scene.world_matrices()[&2].transform_point3(Vec3::ZERO);
        assert!(world.abs_diff_eq(Vec3::new(9.0, 0.0, 0.0), 1e-5), "{world}");
    }
}
