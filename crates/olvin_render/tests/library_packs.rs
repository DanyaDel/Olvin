//! Manifest sanity of every `library/*/pack.ron`. The struct mirrors the
//! pack format contract; the real parser lives in `olvin_library`, this test
//! can switch to it once it is merged.

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct Pack {
    id: String,
    name: String,
    version: String,
    category: Category,
    description: String,
    #[serde(default)]
    tags: Vec<String>,
    author: String,
    license: String,
    #[serde(default)]
    requires: Vec<String>,
    #[serde(default)]
    provides: Vec<String>,
    #[serde(default)]
    notes: String,
}

#[derive(Debug, Deserialize, PartialEq)]
enum Category {
    Characters,
    Gameplay,
    Shaders,
    Materials,
    Looks,
    Prefabs,
    Audio,
    Misc,
}

fn library() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library")
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            files(&p, out);
        } else {
            out.push(p);
        }
    }
}

/// The packs this branch ships.
const OURS: [&str; 7] = [
    "toon-shaders",
    "outline-shaders",
    "hologram-shaders",
    "dissolve-shaders",
    "water-shaders",
    "basic-materials",
    "looks-cinematic",
];

#[test]
fn every_pack_manifest_is_sane() {
    let mut seen = 0;
    for entry in std::fs::read_dir(library()).unwrap().flatten() {
        let dir = entry.path();
        let manifest = dir.join("pack.ron");
        if !manifest.is_file() {
            continue;
        }
        seen += 1;
        let folder = entry.file_name().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(&manifest).unwrap();
        let pack: Pack = ron::from_str(&text).unwrap_or_else(|e| panic!("{folder}: {e}"));
        assert_eq!(pack.id, folder, "id must equal the folder name");
        assert!(
            !pack.id.is_empty()
                && pack
                    .id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                && !pack.id.starts_with('-')
                && !pack.id.ends_with('-'),
            "{folder}: id is not kebab-case"
        );
        assert!(!pack.name.is_empty() && !pack.description.is_empty());
        assert!(
            pack.version.split('.').count() == 3
                && pack.version.split('.').all(|n| n.parse::<u32>().is_ok()),
            "{folder}: version {}",
            pack.version
        );
        assert!(!pack.license.is_empty() && !pack.author.is_empty());
        assert!(!pack.provides.is_empty(), "{folder}: provides nothing");
        // `provides` is exactly the files under assets/, namespaced by the pack id.
        let assets = dir.join("assets");
        let mut on_disk = Vec::new();
        files(&assets, &mut on_disk);
        let mut on_disk: Vec<String> = on_disk
            .iter()
            .map(|p| {
                p.strip_prefix(&assets)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        on_disk.sort();
        let mut provided = pack.provides.clone();
        provided.sort();
        assert_eq!(provided, on_disk, "{folder}: provides != assets/ contents");
        for p in &pack.provides {
            assert!(
                p.starts_with(&format!("{folder}/")),
                "{folder}: {p} is not namespaced"
            );
        }
        for req in &pack.requires {
            assert!(
                library().join(req).join("pack.ron").is_file(),
                "{folder}: requires {req}"
            );
        }
        // Optional preview: small.
        let preview = dir.join("preview.png");
        if preview.is_file() {
            let len = std::fs::metadata(&preview).unwrap().len();
            assert!(len <= 256 * 1024, "{folder}: preview is {len} bytes");
        }
    }
    assert!(seen > 0);
    for ours in OURS {
        assert!(
            library().join(ours).join("pack.ron").is_file(),
            "{ours} missing"
        );
        assert!(
            library().join(ours).join("preview.png").is_file(),
            "{ours}: preview.png missing (OLVIN_LIBRARY_PREVIEWS=<dir> cargo test -p olvin_render render_library_previews)"
        );
    }
}

#[test]
fn library_assets_parse() {
    use olvin_scene::look::LookDesc;
    use olvin_scene::material::MaterialDesc;
    let mut all = Vec::new();
    files(&library(), &mut all);
    for p in all {
        let name = p.to_string_lossy();
        if name.ends_with(".mat.ron") {
            MaterialDesc::load(&p).unwrap_or_else(|e| panic!("{e}"));
        } else if name.ends_with(".look.ron") {
            LookDesc::load(&p).unwrap_or_else(|e| panic!("{e}"));
        } else if name.contains("/assets/") && name.ends_with(".png") {
            assert!(
                std::fs::metadata(&p).unwrap().len() <= 64 * 1024,
                "{name}: too big"
            );
        }
    }
}
