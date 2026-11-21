//! Scene files (`*.scene.ron`): what the editor saves and games load.
//! Plain descriptions, independent of the ECS and the renderer.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use glam::{EulerRot, Mat4, Quat, Vec3};
pub use olvin_csg::{BrushShape, CsgOp};
pub use olvin_mesh::PolyMesh;
use serde::{Deserialize, Serialize};

pub use crate::anim::{
    AnimGraphDesc, AnimStateDesc, AnimationDesc, MotionDesc, ParamDesc, TestDesc, TransitionDesc,
};
pub use crate::primitives::Shape;
use crate::{Camera, Transform};

/// The version of the format this build writes. Bump it together with a new entry
/// in [`MIGRATIONS`] whenever a change cannot be absorbed by serde defaults/aliases,
/// or when an older build would silently lose data it does not know (it refuses
/// files newer than its own version instead).
pub const FORMAT_VERSION: u32 = 2;

/// Upgrades a scene one version up: `MIGRATIONS[n]` takes version `n` to `n + 1`.
/// It runs on the parsed scene, so renamed or new fields are handled by `#[serde(alias)]`
/// and `#[serde(default)]`, and the migration fixes up the meaning of the data.
pub type Migration = fn(&mut SceneFile);

/// The migration chain, oldest first. Its length must equal [`FORMAT_VERSION`].
pub const MIGRATIONS: &[Migration] = &[v0_to_v1, v1_to_v2];

/// Version 0 is a file from before versioning: the same data without the `version` field.
fn v0_to_v1(_: &mut SceneFile) {}

/// Version 2 adds `EntityDesc::model` (glTF models with animation). Older
/// files have no models, so the data stays as it is; the bump makes version-1
/// builds refuse new files instead of spawning empty objects where models were.
fn v1_to_v2(_: &mut SceneFile) {}

fn legacy_version() -> u32 {
    0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneFile {
    /// Format version, see [`FORMAT_VERSION`]. A file without the field is version 0.
    #[serde(default = "legacy_version")]
    pub version: u32,
    /// Parents always come before their children; sibling order is display order.
    pub entities: Vec<EntityDesc>,
    /// Lighting of the whole scene (Unity's Lighting window).
    #[serde(default, skip_serializing_if = "Environment::is_default")]
    pub environment: Environment,
    /// Data owned by editor plugins (e.g. animation clips), keyed by plugin id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, String>,
}

impl Default for SceneFile {
    fn default() -> Self {
        Self {
            version: FORMAT_VERSION,
            entities: Vec::new(),
            environment: Environment::default(),
            extensions: BTreeMap::new(),
        }
    }
}

/// A named [`Environment`] maker.
pub type Preset = (&'static str, fn() -> Environment);

/// Scene-wide lighting: ambient, camera exposure, fog and shadows. Colours are linear.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Environment {
    /// Ambient light from above (the sky) and from below (bounce off the ground).
    pub sky: [f32; 3],
    pub ground: [f32; 3],
    pub ambient_intensity: f32,
    /// Background where nothing is drawn.
    pub background: [f32; 3],
    pub exposure: f32,
    /// ACES filmic tonemapping; off = plain clamp.
    pub filmic: bool,
    pub fog: Option<FogDesc>,
    /// What's drawn behind everything.
    pub skybox: Sky,
    /// Shadows are drawn this far from the camera.
    pub shadow_distance: f32,
    /// Shadow map resolution (512..4096).
    pub shadow_resolution: u32,
}

/// The background of the scene (Unity's skybox).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Sky {
    /// Flat `Environment::background`.
    #[default]
    Color,
    /// Gradient sky with a sun disc at the first directional light. Linear colours.
    Procedural {
        zenith: [f32; 3],
        horizon: [f32; 3],
        ground: [f32; 3],
        /// Angular radius of the sun disc, degrees (0 = no disc).
        sun_size: f32,
    },
    /// Equirectangular panorama (`.hdr`, `.exr`, `.png`, `.jpg`), path relative
    /// to the project's `assets/`.
    Texture {
        path: String,
        rotation_deg: f32,
        intensity: f32,
    },
}

impl Sky {
    pub fn procedural() -> Self {
        Sky::Procedural {
            zenith: [0.18, 0.32, 0.62],
            horizon: [0.62, 0.7, 0.8],
            ground: [0.12, 0.11, 0.1],
            sun_size: 1.5,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FogDesc {
    pub color: [f32; 3],
    pub start: f32,
    pub end: f32,
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            sky: [0.15, 0.15, 0.18],
            ground: [0.07, 0.065, 0.06],
            ambient_intensity: 1.0,
            background: [0.02, 0.02, 0.03],
            exposure: 1.0,
            filmic: true,
            fog: None,
            skybox: Sky::Color,
            shadow_distance: 60.0,
            shadow_resolution: 2048,
        }
    }
}

impl Environment {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Ready-made moods, like quick lighting presets in DCC tools.
    pub const PRESETS: [Preset; 4] = [
        ("Day", || Environment {
            sky: [0.32, 0.4, 0.55],
            ground: [0.12, 0.1, 0.08],
            background: [0.36, 0.5, 0.72],
            skybox: Sky::procedural(),
            ..Default::default()
        }),
        ("Sunset", || Environment {
            sky: [0.35, 0.22, 0.25],
            ground: [0.1, 0.06, 0.05],
            background: [0.55, 0.3, 0.22],
            skybox: Sky::Procedural {
                zenith: [0.16, 0.18, 0.36],
                horizon: [0.9, 0.45, 0.24],
                ground: [0.1, 0.06, 0.05],
                sun_size: 2.5,
            },
            exposure: 1.1,
            fog: Some(FogDesc {
                color: [0.5, 0.3, 0.22],
                start: 20.0,
                end: 140.0,
            }),
            ..Default::default()
        }),
        ("Night", || Environment {
            sky: [0.03, 0.04, 0.08],
            ground: [0.01, 0.01, 0.015],
            background: [0.005, 0.008, 0.02],
            skybox: Sky::Procedural {
                zenith: [0.002, 0.004, 0.012],
                horizon: [0.02, 0.03, 0.06],
                ground: [0.005, 0.005, 0.008],
                sun_size: 0.0,
            },
            exposure: 1.4,
            fog: Some(FogDesc {
                color: [0.01, 0.015, 0.03],
                start: 10.0,
                end: 80.0,
            }),
            ..Default::default()
        }),
        ("Studio", || Environment {
            sky: [0.4, 0.4, 0.42],
            ground: [0.2, 0.2, 0.2],
            background: [0.18, 0.18, 0.2],
            filmic: false,
            ..Default::default()
        }),
    ];
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
    /// Material asset (`*.mat.ron`, path relative to the project's `assets/`)
    /// for the mesh, the edited mesh or the faces this brush contributes.
    /// `None`: plain `color` (mesh/brush colour), no textures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    /// glTF model instanced under this object at load (format version 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelDesc>,
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
        Self {
            kind,
            color: [1.0, 0.96, 0.9],
            intensity: 1.0,
            range: 10.0,
            spot_angle: 30.0,
            shadows: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ColliderShape {
    Box {
        size: Vec3,
    },
    Sphere {
        radius: f32,
    },
    /// Along local Y; `height` includes the caps (like Unity).
    Capsule {
        radius: f32,
        height: f32,
    },
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
        Self {
            shape,
            center: Vec3::ZERO,
            is_trigger: false,
            friction: 0.6,
            restitution: 0.0,
        }
    }

    /// The collider Unity adds for a primitive.
    pub fn for_shape(shape: Shape) -> Self {
        Self::new(match shape {
            Shape::Cube => ColliderShape::Box { size: Vec3::ONE },
            Shape::Sphere => ColliderShape::Sphere { radius: 0.5 },
            Shape::Capsule | Shape::Cylinder => ColliderShape::Capsule {
                radius: 0.5,
                height: 2.0,
            },
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
        Self {
            height: 2.0,
            radius: 0.5,
            step_offset: 0.3,
            slope_limit_deg: 45.0,
        }
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

/// A model asset placed on an object: its meshes and materials, and for a
/// skinned model the skeleton with an animator. The object is the model's
/// root; `collider: Mesh` on the object uses the model's triangles.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelDesc {
    /// `.gltf` / `.glb`, relative to the project's `assets/` (like materials).
    pub path: String,
    /// What the skeleton plays; `None`: the rest pose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animation: Option<AnimationDesc>,
}

impl ModelDesc {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            animation: None,
        }
    }
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
        Self {
            fov_deg: 60.0,
            near: 0.1,
            far: 1000.0,
        }
    }
}

impl From<&CameraDesc> for Camera {
    fn from(c: &CameraDesc) -> Self {
        Camera {
            fov_y: c.fov_deg.to_radians(),
            near: c.near,
            far: c.far,
        }
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
            material: None,
            model: None,
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

impl EntityDesc {
    /// This brush as CSG input. A modelled brush (`poly`) sends its faces:
    /// convex ones as they are, concave ones as triangles.
    pub fn brush_input(&self, transform: Mat4) -> Option<olvin_csg::BrushInput> {
        let b = self.brush.as_ref()?;
        let faces = self
            .poly
            .as_ref()
            .filter(|p| !p.faces.is_empty())
            .map(|poly| {
                let mut out = Vec::with_capacity(poly.faces.len());
                for f in 0..poly.faces.len() {
                    let points: Vec<Vec3> = poly.face_positions(f).collect();
                    if convex_polygon(&points, poly.face_normal(f)) {
                        out.push(points);
                    } else {
                        out.extend(
                            poly.face_triangles(f)
                                .iter()
                                .map(|t| t.iter().map(|&i| poly.positions[i as usize]).collect()),
                        );
                    }
                }
                out
            });
        Some(olvin_csg::BrushInput {
            id: self.id,
            shape: b.shape,
            op: b.op,
            transform,
            color: b.color,
            vertices: b.vertices.clone(),
            faces,
        })
    }
}

fn convex_polygon(points: &[Vec3], normal: Vec3) -> bool {
    let n = points.len();
    (0..n).all(|i| {
        let (a, b, c) = (points[i], points[(i + 1) % n], points[(i + 2) % n]);
        (b - a).cross(c - b).dot(normal) >= -1e-6
    })
}

impl SceneFile {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Reads a scene from text of any supported version and migrates it to
    /// [`FORMAT_VERSION`]. A file from a newer build is an error, not a silent downgrade.
    pub fn parse(text: &str) -> Result<Self, String> {
        Self::parse_with(text, FORMAT_VERSION, MIGRATIONS)
    }

    /// [`SceneFile::parse`] with an explicit target version and migration chain.
    pub fn parse_with(text: &str, target: u32, migrations: &[Migration]) -> Result<Self, String> {
        #[derive(Deserialize)]
        struct Header {
            #[serde(default = "legacy_version")]
            version: u32,
        }
        let header: Header = ron::from_str(text).map_err(|e| e.to_string())?;
        if header.version > target {
            return Err(format!(
                "scene format version {} is newer than supported {target}",
                header.version
            ));
        }
        let mut scene: SceneFile = ron::from_str(text).map_err(|e| e.to_string())?;
        scene.migrate_with(target, migrations);
        Ok(scene)
    }

    /// Runs the chain from `self.version` up to `target`; `version` is stamped after each step.
    fn migrate_with(&mut self, target: u32, migrations: &[Migration]) {
        while self.version < target {
            let Some(step) = migrations.get(self.version as usize) else {
                break;
            };
            step(self);
            self.version += 1;
        }
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
                e.brush_input(*worlds.get(&e.id)?)
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
            let l = e
                .light
                .as_ref()
                .filter(|l| l.kind == LightKind::Directional)?;
            if !self.active_in_hierarchy(e.id) {
                return None;
            }
            Some((
                EntityDesc::forward(*worlds.get(&e.id)?),
                l.color,
                l.intensity,
            ))
        })
    }

    /// World matrices of all entities, parents resolved.
    pub fn world_matrices(&self) -> HashMap<u64, Mat4> {
        self.world_matrices_with(|e| e.local_transform())
    }

    /// Same, with a custom local transform (e.g. an animation preview pose).
    pub fn world_matrices_with(
        &self,
        local: impl Fn(&EntityDesc) -> Transform,
    ) -> HashMap<u64, Mat4> {
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
        child.mesh = Some(MeshDesc {
            shape: Shape::Cube,
            color: [1.0; 4],
        });
        scene.entities = vec![parent, child];

        let text = ron::to_string(&scene).unwrap();
        let back: SceneFile = ron::from_str(&text).unwrap();
        assert_eq!(back, scene);

        // Child sits 1 unit "forward" of a parent turned 90° left: at x = 9.
        let world = scene.world_matrices()[&2].transform_point3(Vec3::ZERO);
        assert!(world.abs_diff_eq(Vec3::new(9.0, 0.0, 0.0), 1e-5), "{world}");
    }

    /// Сцена, где заполнено всё, что умеет формат: компоненты, окружение, расширения.
    fn full_scene() -> SceneFile {
        let mut root = EntityDesc::new(1, "Уровень");
        root.tag = "Level".into();
        root.layer = 3;
        root.brush = Some(BrushDesc {
            shape: BrushShape::Cylinder { sides: 12 },
            op: CsgOp::Subtract,
            color: [0.2, 0.4, 0.6, 1.0],
            vertices: None,
        });

        let mut cam = EntityDesc::new(2, "Camera");
        cam.parent = Some(1);
        cam.position = Vec3::new(0.25, 1.75, -3.5);
        cam.rotation = Vec3::new(-15.0, 30.0, 0.0);
        cam.camera = Some(CameraDesc::default());
        cam.audio_listener = true;

        let mut sun = EntityDesc::new(3, "Sun");
        sun.rotation = Vec3::new(-50.0, 0.0, 0.0);
        sun.light = Some(LightDesc::new(LightKind::Directional));

        let mut player = EntityDesc::new(4, "Player");
        player.parent = Some(1);
        player.scale = Vec3::new(1.0, 2.0, 1.0);
        player.mesh = Some(MeshDesc {
            shape: Shape::Capsule,
            color: [0.9, 0.1, 0.1, 1.0],
        });
        player.material = Some("materials/player.mat.ron".into());
        player.character_controller = Some(CharacterControllerDesc::default());
        player.audio_source = Some(AudioSourceDesc {
            clip: "audio/steps.ogg".into(),
            looping: true,
            ..Default::default()
        });
        player.scripts = vec![ScriptDesc {
            name: "PlayerMove".into(),
            params: BTreeMap::from([("speed".into(), "4.5".into())]),
        }];

        let mut crate_ = EntityDesc::new(5, "Crate");
        crate_.active = false;
        crate_.poly = Some(PolyMesh::cube(1.0));
        let mut collider = ColliderDesc::new(ColliderShape::Box {
            size: Vec3::splat(1.0),
        });
        collider.center = Vec3::new(0.0, 0.5, 0.0);
        crate_.collider = Some(collider);
        crate_.rigidbody = Some(RigidbodyDesc {
            kind: BodyKind::Kinematic,
            freeze_rotation: [true, false, true],
            ccd: true,
            ..Default::default()
        });

        let mut trigger = EntityDesc::new(6, "Trigger");
        trigger.parent = Some(5);
        let mut zone = ColliderDesc::new(ColliderShape::Sphere { radius: 2.5 });
        zone.is_trigger = true;
        trigger.collider = Some(zone);

        let mut robot = EntityDesc::new(7, "Robot");
        robot.model = Some(ModelDesc {
            path: "models/robot.gltf".into(),
            animation: Some(AnimationDesc::Inline(AnimGraphDesc {
                states: vec![
                    AnimStateDesc::clip("Idle", "Idle"),
                    AnimStateDesc {
                        name: "Walk".into(),
                        motion: MotionDesc::Blend1D {
                            param: "speed".into(),
                            points: vec![(0.0, "Idle".into()), (2.0, "Walk".into())],
                        },
                        speed: 1.5,
                        looping: false,
                    },
                ],
                transitions: vec![TransitionDesc {
                    from: Some("Idle".into()),
                    to: "Walk".into(),
                    when: vec![("speed".into(), TestDesc::Greater(0.1))],
                    exit_time: Some(0.5),
                    duration: 0.25,
                }],
                initial: "Idle".into(),
                params: BTreeMap::from([
                    ("speed".into(), ParamDesc::Float(0.0)),
                    ("jump".into(), ParamDesc::Trigger),
                ]),
            })),
        });
        robot.collider = Some(ColliderDesc::new(ColliderShape::Mesh));
        let mut dancer = EntityDesc::new(8, "Dancer");
        dancer.model = Some(ModelDesc {
            path: "models/dancer.glb".into(),
            animation: Some(AnimationDesc::Graph("anim/dancer.animgraph.ron".into())),
        });
        let mut prop = EntityDesc::new(9, "Prop");
        prop.model = Some(ModelDesc::new("models/crate.gltf"));

        SceneFile {
            entities: vec![root, cam, player, sun, crate_, trigger, robot, dancer, prop],
            environment: Environment {
                fog: Some(FogDesc {
                    color: [0.5, 0.6, 0.7],
                    start: 10.0,
                    end: 80.0,
                }),
                skybox: Sky::Texture {
                    path: "sky/day.hdr".into(),
                    rotation_deg: 90.0,
                    intensity: 1.25,
                },
                filmic: false,
                shadow_resolution: 4096,
                ..Default::default()
            },
            extensions: BTreeMap::from([("animator".into(), "(clips: [])".into())]),
            ..Default::default()
        }
    }

    #[test]
    fn file_roundtrip_keeps_everything() {
        let dir = std::env::temp_dir().join(format!("olvin_scene_rt_{}", std::process::id()));
        let path = dir.join("scenes/main.scene.ron");
        let scene = full_scene();
        // save сам создаёт недостающие папки.
        scene.save(&path).unwrap();
        let back = SceneFile::load(&path).unwrap();
        assert_eq!(back, scene);

        // Второй круг даёт тот же текст: формат стабилен.
        let first = std::fs::read_to_string(&path).unwrap();
        back.save(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), first);

        // Значения по умолчанию не пишутся, но читаются обратно.
        let minimal = SceneFile {
            entities: vec![EntityDesc::new(7, "Empty")],
            ..Default::default()
        };
        minimal.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("environment") && !text.contains("extensions"));
        assert_eq!(SceneFile::load(&path).unwrap(), minimal);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_reports_missing_and_broken_files() {
        let dir = std::env::temp_dir().join(format!("olvin_scene_err_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("nope.scene.ron");
        let err = SceneFile::load(&missing).unwrap_err();
        assert!(err.contains("nope.scene.ron"), "{err}");

        let broken = dir.join("broken.scene.ron");
        std::fs::write(&broken, "(entities: [ (id: 1, name: ").unwrap();
        let err = SceneFile::load(&broken).unwrap_err();
        assert!(err.contains("broken.scene.ron"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hierarchy_order_activity_and_sun() {
        let scene = full_scene();
        let order: Vec<u64> = scene
            .hierarchy_order()
            .into_iter()
            .map(|i| scene.entities[i].id)
            .collect();
        // Обход в глубину, братья в порядке файла.
        assert_eq!(order, vec![1, 2, 4, 3, 5, 6, 7, 8, 9]);
        assert!(scene.active_in_hierarchy(4));
        assert!(!scene.active_in_hierarchy(5));
        assert!(
            !scene.active_in_hierarchy(6),
            "ребёнок выключенного объекта выключен"
        );
        let (dir, _, intensity) = scene.sun().expect("направленный свет");
        assert!(dir.y < -0.5, "солнце светит вниз: {dir}");
        assert!(intensity > 0.0);
        assert_eq!(scene.csg_brushes().len(), 1);
    }

    const V0: &str = include_str!("../tests/fixtures/v0_legacy.scene.ron");
    const V1: &str = include_str!("../tests/fixtures/v1.scene.ron");
    const V2: &str = include_str!("../tests/fixtures/v2.scene.ron");

    #[test]
    fn legacy_file_without_version_loads_as_current() {
        let scene = SceneFile::parse(V0).unwrap();
        assert_eq!(scene.version, FORMAT_VERSION);
        assert_eq!(scene.entities.len(), 3);
        let player = scene.get(2).unwrap();
        assert_eq!(player.name, "Player");
        assert_eq!(player.parent, Some(1));
        assert_eq!(player.position, Vec3::new(1.0, 0.0, 2.0));
        assert_eq!(player.scripts[0].name, "PlayerMove");
        assert!(scene.sun().is_some());
        assert_eq!(scene.environment.shadow_distance, 40.0);
        // Старый файл при сохранении становится файлом текущей версии.
        let text = ron::ser::to_string_pretty(&scene, Default::default()).unwrap();
        assert!(
            text.contains(&format!("version: {FORMAT_VERSION}")),
            "{text}"
        );
        assert_eq!(SceneFile::parse(&text).unwrap(), scene);
    }

    #[test]
    fn v1_file_migrates_to_v2_without_models() {
        let scene = SceneFile::parse(V1).unwrap();
        assert_eq!(scene.version, 2);
        assert!(scene.entities.iter().all(|e| e.model.is_none()));
        // Всё остальное — как в фикстуре v2 без нового объекта.
        let mut v2 = SceneFile::parse(V2).unwrap();
        v2.entities.retain(|e| e.model.is_none());
        assert_eq!(scene, v2);
    }

    #[test]
    fn v2_fixture_reads_models_and_animation() {
        let scene = SceneFile::parse(V2).unwrap();
        let robot = scene.get(4).unwrap().model.as_ref().unwrap();
        assert_eq!(robot.path, "models/robot.gltf");
        let Some(AnimationDesc::Inline(graph)) = &robot.animation else {
            panic!("inline graph expected: {:?}", robot.animation);
        };
        assert_eq!(graph.initial, "Idle");
        assert_eq!(graph.states.len(), 2);
        assert!(graph.states[1].looping && graph.states[1].speed == 1.0);
        assert_eq!(
            graph.transitions[0].when[0],
            ("near".into(), TestDesc::True)
        );
        assert_eq!(graph.params["near"], ParamDesc::Bool(false));
        let crate_ = scene.get(5).unwrap().model.as_ref().unwrap();
        assert_eq!(crate_.animation, None);
        // Сборка версии 1 такой файл не открывает, а не теряет модели молча.
        let err = SceneFile::parse_with(V2, 1, &MIGRATIONS[..1]).unwrap_err();
        assert!(err.contains("newer"), "{err}");
    }

    #[test]
    fn current_fixture_matches_legacy_content() {
        let v0 = SceneFile::parse(V0).unwrap();
        let v1 = SceneFile::parse(V1).unwrap();
        assert_eq!(v0, v1);
    }

    #[test]
    fn new_scenes_are_written_with_a_version() {
        let text = ron::to_string(&SceneFile::default()).unwrap();
        assert!(
            text.contains(&format!("version:{FORMAT_VERSION}")),
            "{text}"
        );
    }

    #[test]
    fn future_version_is_rejected() {
        let err = SceneFile::parse("(version: 999, entities: [])").unwrap_err();
        assert!(err.contains("999") && err.contains("newer"), "{err}");
    }

    #[test]
    fn broken_text_is_an_error() {
        assert!(SceneFile::parse("(version: 1, entities: [").is_err());
    }

    #[test]
    fn migration_chain_runs_in_order_from_the_file_version() {
        fn first(s: &mut SceneFile) {
            // 0 -> 1: переименование объектов.
            for e in &mut s.entities {
                e.name = format!("a_{}", e.name);
            }
        }
        fn second(s: &mut SceneFile) {
            // 1 -> 2: слой по умолчанию для всех.
            for e in &mut s.entities {
                e.layer = 7;
                e.name.push_str("_b");
            }
        }
        let chain: &[Migration] = &[first, second];
        let body = "entities: [(id: 1, name: \"x\", position: (0.0, 0.0, 0.0), rotation: (0.0, 0.0, 0.0), scale: (1.0, 1.0, 1.0))]";

        let from0 = SceneFile::parse_with(&format!("({body})"), 2, chain).unwrap();
        assert_eq!(from0.version, 2);
        assert_eq!(from0.entities[0].name, "a_x_b");
        assert_eq!(from0.entities[0].layer, 7);

        // Файл версии 1 пропускает первую миграцию.
        let from1 = SceneFile::parse_with(&format!("(version: 1, {body})"), 2, chain).unwrap();
        assert_eq!(from1.entities[0].name, "x_b");

        // Файл текущей версии не трогают.
        let same = SceneFile::parse_with(&format!("(version: 2, {body})"), 2, chain).unwrap();
        assert_eq!(same.entities[0].name, "x");
        assert_eq!(same.entities[0].layer, 0);
    }

    #[test]
    fn migration_chain_covers_every_version() {
        assert_eq!(MIGRATIONS.len(), FORMAT_VERSION as usize);
    }

    #[test]
    fn load_migrates_old_files_from_disk() {
        let dir = std::env::temp_dir().join(format!("olvin_scene_old_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("old.scene.ron");
        std::fs::write(&path, V0).unwrap();
        let scene = SceneFile::load(&path).unwrap();
        assert_eq!(scene.version, FORMAT_VERSION);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
