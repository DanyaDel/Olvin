//! `.meta` sidecar files: a stable GUID plus importer settings, like Unity's.
//! A file `assets/a/b.png` has its meta in `assets/a/b.png.meta` (RON).

use std::fmt;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::AssetKind;

/// Extension of sidecar files (appended to the asset's full file name).
pub const META_EXT: &str = "meta";

/// Globally unique asset id; survives renames and moves. Written as 32 hex digits.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Guid(pub u128);

impl Guid {
    /// A fresh random id (std's randomly keyed hasher over time, pid and a counter).
    pub fn new_random() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let half = |salt: u64| {
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u128(nanos);
            h.write_u64(n);
            h.write_u32(std::process::id());
            h.write_u64(salt);
            h.finish()
        };
        let guid = ((half(0x9e37_79b9) as u128) << 64) | half(0x85eb_ca6b) as u128;
        // Zero is reserved for "no asset".
        Self(guid.max(1))
    }

    pub fn parse(s: &str) -> Option<Self> {
        if s.len() != 32 {
            return None;
        }
        u128::from_str_radix(s, 16).ok().map(Self)
    }
}

impl fmt::Display for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

impl fmt::Debug for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Guid({self})")
    }
}

impl Serialize for Guid {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Guid {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Guid::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("bad guid `{s}`")))
    }
}

/// How a texture is turned into GPU data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextureImport {
    /// Colour data (albedo, emissive); off for normal maps, ORM, masks.
    pub srgb: bool,
    /// Larger images are downscaled to fit (0 = no limit).
    pub max_size: u32,
    pub generate_mips: bool,
    /// Nearest filtering (pixel art).
    pub point_filter: bool,
}

impl Default for TextureImport {
    fn default() -> Self {
        Self {
            srgb: true,
            max_size: 4096,
            generate_mips: true,
            point_filter: false,
        }
    }
}

/// How a model (glTF/GLB) is imported.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelImport {
    /// Uniform scale baked into vertices and node translations.
    pub scale: f32,
    /// Reserved: no lightmapper yet.
    pub generate_lightmap_uvs: bool,
}

impl Default for ModelImport {
    fn default() -> Self {
        Self {
            scale: 1.0,
            generate_lightmap_uvs: false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Importer {
    #[default]
    None,
    Texture(TextureImport),
    Model(ModelImport),
}

impl Importer {
    pub fn is_none(&self) -> bool {
        matches!(self, Importer::None)
    }

    /// Default importer settings for an asset kind.
    pub fn for_kind(kind: AssetKind) -> Self {
        match kind {
            AssetKind::Texture => Importer::Texture(TextureImport::default()),
            AssetKind::Model => Importer::Model(ModelImport::default()),
            _ => Importer::None,
        }
    }
}

/// Contents of a `.meta` file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    pub guid: Guid,
    #[serde(default, skip_serializing_if = "Importer::is_none")]
    pub importer: Importer,
}

impl Meta {
    pub fn new(kind: AssetKind) -> Self {
        Self {
            guid: Guid::new_random(),
            importer: Importer::for_kind(kind),
        }
    }

    pub fn texture(&self) -> Option<&TextureImport> {
        match &self.importer {
            Importer::Texture(t) => Some(t),
            _ => None,
        }
    }

    pub fn model(&self) -> Option<&ModelImport> {
        match &self.importer {
            Importer::Model(m) => Some(m),
            _ => None,
        }
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        ron::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let path = path.as_ref();
        let text =
            ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::new().struct_names(false))
                .map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// `foo/bar.png` → `foo/bar.png.meta`.
pub fn meta_path(asset: impl AsRef<Path>) -> PathBuf {
    let mut s = asset.as_ref().as_os_str().to_owned();
    s.push(".");
    s.push(META_EXT);
    PathBuf::from(s)
}

/// Sidecar or editor/OS temp file that is never an asset.
pub fn is_ignored_name(name: &str) -> bool {
    name.starts_with('.')
        || name.ends_with('~')
        || name.ends_with(".meta")
        || [".tmp", ".swp", ".swx", ".part", ".crdownload", ".bak"]
            .iter()
            .any(|ext| name.ends_with(ext))
        || name.starts_with("~$")
        || name == "4913" // vim's write probe
}
