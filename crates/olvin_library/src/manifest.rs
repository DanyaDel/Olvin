//! `pack.ron`: the manifest of a pack, and its validation rules.

use serde::{Deserialize, Serialize};

/// Where a pack shows up in the library window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Category {
    Characters,
    Gameplay,
    Shaders,
    Materials,
    Looks,
    Prefabs,
    Audio,
    Misc,
}

impl Category {
    pub const ALL: [Category; 8] = [
        Category::Characters,
        Category::Gameplay,
        Category::Shaders,
        Category::Materials,
        Category::Looks,
        Category::Prefabs,
        Category::Audio,
        Category::Misc,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::Characters => "Characters",
            Category::Gameplay => "Gameplay",
            Category::Shaders => "Shaders",
            Category::Materials => "Materials",
            Category::Looks => "Looks",
            Category::Prefabs => "Prefabs",
            Category::Audio => "Audio",
            Category::Misc => "Misc",
        }
    }

    /// Case-insensitive, for the command line.
    pub fn parse(text: &str) -> Option<Category> {
        Self::ALL
            .into_iter()
            .find(|c| c.label().eq_ignore_ascii_case(text.trim()))
    }
}

impl std::fmt::Display for Category {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// The contents of `pack.ron`. Unknown fields are an error: a typo should not
/// silently drop `requires`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// kebab-case, unique, equal to the folder name.
    pub id: String,
    pub name: String,
    /// `major.minor.patch`.
    pub version: String,
    pub category: Category,
    /// English, one to three sentences.
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub author: String,
    /// SPDX expression, e.g. `MIT OR Apache-2.0` or `CC0-1.0`.
    pub license: String,
    /// Other pack ids, imported first.
    #[serde(default)]
    pub requires: Vec<String>,
    /// Headline assets (paths relative to `assets/`), shown in the UI. The first
    /// `*.prefab.ron` is what "Add to scene" instantiates.
    #[serde(default)]
    pub provides: Vec<String>,
    /// How to use the pack after the import.
    #[serde(default)]
    pub notes: Option<String>,
}

impl Manifest {
    /// Parses `pack.ron` text. `Some("..")` and a plain string both work for `notes`.
    pub fn parse(text: &str) -> Result<Self, String> {
        ron::Options::default()
            .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME)
            .from_str(text)
            .map_err(|e| format!("pack.ron: {e}"))
    }

    /// Every rule that can be checked on the manifest alone. `folder` is the
    /// pack's directory name.
    pub fn validate(&self, folder: &str) -> Vec<String> {
        let mut errors = Vec::new();
        if !is_kebab(&self.id) {
            errors.push(format!("id `{}` is not kebab-case", self.id));
        }
        if self.id != folder {
            errors.push(format!(
                "id `{}` differs from the folder name `{folder}`",
                self.id
            ));
        }
        if self.name.trim().is_empty() {
            errors.push("name is empty".into());
        }
        if !is_version(&self.version) {
            errors.push(format!(
                "version `{}` is not major.minor.patch",
                self.version
            ));
        }
        if self.description.trim().is_empty() {
            errors.push("description is empty".into());
        }
        if self.author.trim().is_empty() {
            errors.push("author is empty".into());
        }
        if self.license.trim().is_empty() {
            errors.push("license is empty".into());
        }
        if self.tags.len() > MAX_TAGS {
            errors.push(format!("more than {MAX_TAGS} tags"));
        }
        for dep in &self.requires {
            if !is_kebab(dep) {
                errors.push(format!("requires `{dep}`: not a kebab-case id"));
            }
            if *dep == self.id {
                errors.push("a pack cannot require itself".into());
            }
        }
        for path in &self.provides {
            if let Err(e) = check_path(path) {
                errors.push(format!("provides `{path}`: {e}"));
            }
        }
        errors
    }
}

pub const MAX_TAGS: usize = 12;
/// One file of a pack.
pub const MAX_FILE_BYTES: usize = 16 << 20;
/// All files of a pack together.
pub const MAX_PACK_BYTES: usize = 64 << 20;
pub const MAX_FILES: usize = 1024;
pub const MAX_PREVIEW_BYTES: usize = 256 << 10;

/// Extensions a pack may ship: data, scripts, shaders, images, sounds, models.
/// No executables, no Rust (a pack must not change what the game compiles), and
/// no `.meta` (GUIDs are made by the import).
pub const ALLOWED_EXTENSIONS: &[&str] = &[
    "ron", "lua", "wgsl", "png", "jpg", "jpeg", "tga", "webp", "hdr", "exr", "ogg", "wav", "flac",
    "mp3", "gltf", "glb", "bin", "json", "toml", "txt", "md",
];

pub fn is_kebab(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 48
        && s.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

fn is_version(s: &str) -> bool {
    let core = s.split(['-', '+']).next().unwrap_or("");
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// A path inside `assets/`: relative, `/`-separated, no `..`, no hidden parts,
/// plain characters.
pub fn check_path(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("empty path".into());
    }
    if path.starts_with('/') || path.contains('\\') || path.contains(':') {
        return Err("must be relative and use `/`".into());
    }
    for part in path.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err("contains an empty, `.` or `..` part".into());
        }
        if part.starts_with('.') {
            return Err("hidden files are not allowed".into());
        }
        if part.len() > 100
            || !part
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ' ' | '(' | ')'))
        {
            return Err(format!(
                "`{part}` has characters outside letters, digits and `-_. ()`"
            ));
        }
    }
    Ok(())
}

/// Extension check for one file of a pack (lowercase compare).
pub fn check_extension(path: &str) -> Result<(), String> {
    let ext = path.rsplit_once('.').map_or("", |(_, e)| e).to_lowercase();
    if ALLOWED_EXTENSIONS.contains(&ext.as_str()) {
        Ok(())
    } else {
        Err(format!("extension `.{ext}` is not allowed in a pack"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kebab_ids() {
        for ok in ["a", "fps-controller", "pbr-2"] {
            assert!(is_kebab(ok), "{ok}");
        }
        for bad in ["", "Fps", "a_b", "-a", "a-", "a--b", "a b"] {
            assert!(!is_kebab(bad), "{bad}");
        }
    }

    #[test]
    fn paths_stay_inside_assets() {
        assert!(check_path("scripts/fps/player.lua").is_ok());
        for bad in [
            "../x.lua", "a/../b", "/abs", "a\\b", "C:/x", "a//b", ".hidden", "a/.b", "",
        ] {
            assert!(check_path(bad).is_err(), "{bad}");
        }
        assert!(check_extension("a.PNG").is_ok());
        assert!(check_extension("a.exe").is_err());
        assert!(check_extension("a.rs").is_err());
        assert!(check_extension("a.meta").is_err());
        assert!(check_extension("noext").is_err());
    }

    #[test]
    fn parses_the_contract_example() {
        let m = Manifest::parse(
            r#"(
                id: "fps-controller",
                name: "First Person Controller",
                version: "1.0.0",
                category: Characters,
                description: "Mouse look.",
                tags: ["player", "fps"],
                author: "NLPA Collab",
                license: "MIT OR Apache-2.0",
                requires: [],
                provides: ["prefabs/fps-controller/Player.prefab.ron"],
                notes: "Press Play.",
            )"#,
        )
        .unwrap();
        assert_eq!(m.notes.as_deref(), Some("Press Play."));
        assert!(m.validate("fps-controller").is_empty());
        assert!(!m.validate("other").is_empty());
        assert!(Manifest::parse("(id: \"x\", bogus: 1)").is_err());
        let with_some = Manifest::parse(
            r#"(id: "a", name: "A", version: "1.0.0", category: Misc, description: "d",
                author: "x", license: "MIT", notes: Some("n"))"#,
        )
        .unwrap();
        assert_eq!(with_some.notes.as_deref(), Some("n"));
    }

    #[test]
    fn categories_parse() {
        assert_eq!(Category::parse("looks"), Some(Category::Looks));
        assert_eq!(Category::parse("nope"), None);
    }
}
