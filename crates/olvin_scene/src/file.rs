//! Scene files (`*.scene.ron`): what the editor saves and games load.
//! Plain descriptions, independent of the ECS and the renderer.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use glam::{EulerRot, Mat4, Quat, Vec3};
pub use olvin_csg::{BrushShape, CsgOp};
pub use olvin_mesh::PolyMesh;
use serde::{Deserialize, Serialize};

pub use crate::primitives::Shape;
use crate::{Camera, Transform};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SceneFile {
    /// Parents always come before their children; sibling order is display order.
    pub entities: Vec<EntityDesc>,
    /// Data owned by editor plugins (e.g. animation clips), keyed by plugin id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, String>,
}

fn yes() -> bool {
    true
}

fn is_true(v: &bool) -> bool {
    *v
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntityDesc {
    /// Stable within the scene; references (parent, animation tracks) use it.
    pub id: u64,
    pub name: String,
    /// Inactive objects (and their children) are not spawned by games.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub active: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tag: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub layer: u8,
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
    /// Editable polygon mesh (Edit Mode); replaces `mesh.shape` when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poly: Option<PolyMesh>,
    /// CSG brush: its geometry is the unit cube scaled by the transform.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brush: Option<BrushDesc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light: Option<LightDesc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collider: Option<ColliderDesc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rigidbody: Option<RigidbodyDesc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character_controller: Option<CharacterControllerDesc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_source: Option<AudioSourceDesc>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub audio_listener: bool,
    /// Game-side behaviours by name, with string parameters.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scripts: Vec<ScriptDesc>,
}

fn is_zero(v: &u8) -> bool {
    *v == 0
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BrushDesc {
    pub shape: BrushShape,
    pub op: CsgOp,
    /// Linear RGBA of the faces this brush contributes.
    pub color: [f32; 4],
    /// Edited polygons: the brush is the convex hull of these local points
    /// (unit-cube space) instead of `shape`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertices: Option<Vec<Vec3>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LightKind {
    Directional,
    Point,
    Spot,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LightDesc {
    pub kind: LightKind,
    /// Linear RGB.
    pub color: [f32; 3],
    pub intensity: f32,
    pub range: f32,
    pub spot_angle: f32,
    pub shadows: bool,
}

impl LightDesc {
    pub fn new(kind: LightKind) -> Self {
        Self { kind, color: [1.0, 0.96, 0.9], intensity: 1.0, range: 10.0, spot_angle: 30.0, shadows: true }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ColliderShape {
    Box { size: Vec3 },
    Sphere { radius: f32 },
    /// Along local Y; `height` includes the caps (like Unity).
    Capsule { radius: f32, height: f32 },
    /// The object's mesh (or the CSG level for brushes).
    Mesh,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColliderDesc {
    pub shape: ColliderShape,
    pub center: Vec3,
    pub is_trigger: bool,
    pub friction: f32,
    pub restitution: f32,
}

impl ColliderDesc {
    pub fn new(shape: ColliderShape) -> Self {
        Self { shape, center: Vec3::ZERO, is_trigger: false, friction: 0.6, restitution: 0.0 }
    }

    /// The collider Unity adds for a primitive.
    pub fn for_shape(shape: Shape) -> Self {
        Self::new(match shape {
            Shape::Cube => ColliderShape::Box { size: Vec3::ONE },
            Shape::Sphere => ColliderShape::Sphere { radius: 0.5 },
            Shape::Capsule | Shape::Cylinder => ColliderShape::Capsule { radius: 0.5, height: 2.0 },
            Shape::Plane | Shape::Quad => ColliderShape::Mesh,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BodyKind {
    Dynamic,
    Kinematic,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RigidbodyDesc {
    pub kind: BodyKind,
    pub mass: f32,
    pub drag: f32,
    pub angular_drag: f32,
    pub gravity_scale: f32,
    pub freeze_rotation: [bool; 3],
    /// Continuous collision detection for fast objects.
    pub ccd: bool,
}

impl Default for RigidbodyDesc {
    fn default() -> Self {
        Self {
            kind: BodyKind::Dynamic,
            mass: 1.0,
            drag: 0.0,
            angular_drag: 0.05,
            gravity_scale: 1.0,
            freeze_rotation: [false; 3],
            ccd: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CharacterControllerDesc {
    pub height: f32,
    pub radius: f32,
    pub step_offset: f32,
    pub slope_limit_deg: f32,
}

impl Default for CharacterControllerDesc {
    fn default() -> Self {
        Self { height: 2.0, radius: 0.5, step_offset: 0.3, slope_limit_deg: 45.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioSourceDesc {
    /// Path inside `assets/`.
    pub clip: String,
    pub volume: f32,
    pub pitch: f32,
    pub looping: bool,
    pub play_on_awake: bool,
    /// 3D attenuation between the distances; 2D when off.
    pub spatial: bool,
    pub min_distance: f32,
    pub max_distance: f32,
}

impl Default for AudioSourceDesc {
    fn default() -> Self {
        Self {
            clip: String::new(),
            volume: 1.0,
            pitch: 1.0,
            looping: false,
            play_on_awake: true,
            spatial: true,
            min_distance: 1.0,
            max_distance: 50.0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ScriptDesc {
    pub name: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, String>,
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
            active: true,
            tag: String::new(),
            layer: 0,
            parent: None,
            position: Vec3::ZERO,
            rotation: Vec3::ZERO,
            scale: Vec3::ONE,
            mesh: None,
            camera: None,
            brush: None,
            poly: None,
            light: None,
            collider: None,
            rigidbody: None,
            character_controller: None,
            audio_source: None,
            audio_listener: false,
            scripts: Vec::new(),
        }
    }

    /// Forward (-Z) in world space for a world matrix of this entity.
    pub fn forward(world: Mat4) -> Vec3 {
        world.transform_vector3(Vec3::NEG_Z).normalize_or_zero()
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

    /// Entity indices in hierarchy (depth-first) order: the order the
    /// hierarchy panel shows and CSG operations apply in.
    pub fn hierarchy_order(&self) -> Vec<usize> {
        fn visit(scene: &SceneFile, parent: Option<u64>, out: &mut Vec<usize>) {
            for (i, e) in scene.entities.iter().enumerate() {
                if e.parent == parent {
                    out.push(i);
                    visit(scene, Some(e.id), out);
                }
            }
        }
        let mut out = Vec::with_capacity(self.entities.len());
        visit(self, None, &mut out);
        out
    }

    /// CSG brushes in operation order, placed in the world.
    pub fn csg_brushes(&self) -> Vec<olvin_csg::BrushInput> {
        let worlds = self.world_matrices();
        self.hierarchy_order()
            .into_iter()
            .filter_map(|i| {
                let e = &self.entities[i];
                let b = e.brush.as_ref()?;
                Some(olvin_csg::BrushInput {
                    id: e.id,
                    shape: b.shape,
                    op: b.op,
                    transform: *worlds.get(&e.id)?,
                    color: b.color,
                    vertices: b.vertices.clone(),
                })
            })
            .collect()
    }

    /// Whether an entity and all its ancestors are active.
    pub fn active_in_hierarchy(&self, id: u64) -> bool {
        let mut current = self.get(id);
        while let Some(e) = current {
            if !e.active {
                return false;
            }
            current = e.parent.and_then(|p| self.get(p));
        }
        true
    }

    /// The first active directional light's direction (where light travels) and colour.
    pub fn sun(&self) -> Option<(Vec3, [f32; 3], f32)> {
        let worlds = self.world_matrices();
        self.entities.iter().find_map(|e| {
            let l = e.light.as_ref().filter(|l| l.kind == LightKind::Directional)?;
            if !self.active_in_hierarchy(e.id) {
                return None;
            }
            Some((EntityDesc::forward(*worlds.get(&e.id)?), l.color, l.intensity))
        })
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
