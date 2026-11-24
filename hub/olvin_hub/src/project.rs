use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use toml_edit::{Array, DocumentMut, InlineTable, Item, TableLike, Value};

use crate::{Catalog, EngineSource, Module, Source, template};

/// A game that uses Olvin. Its `Cargo.toml` is the only source of truth for
/// installed modules; edits preserve the user's formatting and comments.
pub struct Project {
    root: PathBuf,
    doc: DocumentMut,
}

impl Project {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let path = root.join("Cargo.toml");
        let text = fs::read_to_string(&path)
            .with_context(|| format!("could not read {}", path.display()))?;
        let doc: DocumentMut = text
            .parse()
            .with_context(|| format!("could not parse {}", path.display()))?;
        let project = Self { root, doc };
        if project.olvin().is_none() {
            bail!(
                "{} is not an Olvin project: it has no `olvin` dependency",
                path.display()
            );
        }
        Ok(project)
    }

    /// The nearest Olvin project at or above `start`.
    pub fn discover(start: &Path) -> Result<Self> {
        start
            .ancestors()
            .filter(|dir| dir.join("Cargo.toml").is_file())
            .find_map(|dir| Self::open(dir).ok())
            .with_context(|| format!("no Olvin project in {} or above", start.display()))
    }

    /// Creates `<parent>/<name>` with the catalog's default modules.
    pub fn create(
        parent: &Path,
        name: &str,
        engine: &EngineSource,
        catalog: &Catalog,
    ) -> Result<Self> {
        let defaults: Vec<&str> = catalog
            .modules
            .iter()
            .filter(|m| m.default)
            .map(|m| m.id.as_str())
            .collect();
        Self::create_with(parent, name, engine, catalog, &defaults)
    }

    /// Creates `<parent>/<name>` with exactly these modules (plus what they require).
    pub fn create_with(
        parent: &Path,
        name: &str,
        engine: &EngineSource,
        catalog: &Catalog,
        modules: &[&str],
    ) -> Result<Self> {
        // Requirements count: `scenes` alone brings `render` and `window`.
        let mut wanted: Vec<&str> = Vec::new();
        for id in modules {
            for m in catalog.resolve(id)? {
                if !wanted.contains(&m.id.as_str()) {
                    wanted.push(&m.id);
                }
            }
        }
        validate_name(name)?;
        engine.validate()?;
        let root = parent.join(name);
        if root.exists() {
            bail!("{} already exists. Pick another name", root.display());
        }
        let graphical = ["window", "render", "scenes"]
            .iter()
            .all(|m| wanted.contains(m));
        let integrity = wanted.contains(&"integrity");
        let made = Self::write_new(&root, name, engine, catalog, modules, graphical, integrity);
        if made.is_err() {
            // No half-made project left behind (the folder did not exist before).
            let _ = fs::remove_dir_all(&root);
        }
        made
    }

    fn write_new(
        root: &Path,
        name: &str,
        engine: &EngineSource,
        catalog: &Catalog,
        modules: &[&str],
        graphical: bool,
        integrity: bool,
    ) -> Result<Self> {
        fs::create_dir_all(root.join("src"))?;
        for (file, content) in template::files(name, graphical, integrity) {
            fs::write(root.join(file), content)?;
        }
        let mut doc: DocumentMut = template::cargo_toml(name).parse()?;
        doc["dependencies"]
            .as_table_like_mut()
            .context("the project template has no [dependencies] table")?
            .insert(
                "olvin",
                Item::Value(Value::InlineTable(engine.dependency())),
            );
        let mut project = Self {
            root: root.to_path_buf(),
            doc,
        };
        for id in modules {
            project.add(catalog, id)?;
        }
        project.save()?;
        Ok(project)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Last time the manifest or the main scene changed.
    pub fn modified(&self) -> Option<std::time::SystemTime> {
        ["Cargo.toml", "assets/scenes/main.scene.ron", "src/main.rs"]
            .iter()
            .filter_map(|f| {
                fs::metadata(self.root.join(f))
                    .and_then(|m| m.modified())
                    .ok()
            })
            .max()
    }

    pub fn name(&self) -> &str {
        self.doc
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(Item::as_str)
            .unwrap_or("?")
    }

    pub fn is_installed(&self, module: &Module) -> bool {
        match &module.source {
            Source::Olvin { feature } => self.olvin_features().iter().any(|f| f == feature),
            Source::Crate { name, .. } => self.deps().is_some_and(|d| d.contains_key(name)),
        }
    }

    pub fn installed<'c>(&self, catalog: &'c Catalog) -> Vec<&'c Module> {
        catalog
            .modules
            .iter()
            .filter(|m| self.is_installed(m))
            .collect()
    }

    /// Installs a module and whatever it requires. Returns ids actually added.
    pub fn add(&mut self, catalog: &Catalog, id: &str) -> Result<Vec<String>> {
        let mut added = Vec::new();
        for module in catalog.resolve(id)? {
            if !self.is_installed(module) {
                self.install(module)?;
                added.push(module.id.clone());
            }
        }
        Ok(added)
    }

    /// Uninstalls a module. Refuses if installed modules require it, unless `force`.
    pub fn remove(&mut self, catalog: &Catalog, id: &str, force: bool) -> Result<bool> {
        self.remove_among(catalog, id, force, &[])
    }

    /// Uninstalls several modules at once: one that another module of the
    /// same list requires may come in any order (`remove serde saves`).
    /// Returns the ids that were installed and are gone now. All or nothing:
    /// on an error the project is left as it was.
    pub fn remove_many(
        &mut self,
        catalog: &Catalog,
        ids: &[String],
        force: bool,
    ) -> Result<Vec<String>> {
        for id in ids {
            if catalog.get(id).is_none() {
                bail!("no module `{id}`. See the list: olvin-hub list");
            }
        }
        let before = self.doc.clone();
        let mut removed = Vec::new();
        for id in ids {
            match self.remove_among(catalog, id, force, ids) {
                Ok(true) => removed.push(id.clone()),
                Ok(false) => {}
                Err(e) => {
                    self.doc = before;
                    return Err(e);
                }
            }
        }
        Ok(removed)
    }

    /// `remove`, not counting the modules in `also_removed` as dependents.
    fn remove_among(
        &mut self,
        catalog: &Catalog,
        id: &str,
        force: bool,
        also_removed: &[String],
    ) -> Result<bool> {
        let Some(module) = catalog.get(id) else {
            bail!("no module `{id}`. See the list: olvin-hub list")
        };
        if !self.is_installed(module) {
            return Ok(false);
        }
        let dependents: Vec<_> = self
            .installed(catalog)
            .into_iter()
            .filter(|m| m.requires.iter().any(|r| r == id))
            .filter(|m| !also_removed.contains(&m.id))
            .map(|m| m.id.as_str())
            .collect();
        if !dependents.is_empty() && !force {
            bail!(
                "`{id}` is required by {}. Remove those first",
                dependents.join(", ")
            );
        }
        match &module.source {
            Source::Olvin { feature } => {
                if let Some(features) = self
                    .olvin_mut()?
                    .get_mut("features")
                    .and_then(Item::as_array_mut)
                {
                    features.retain(|v| v.as_str() != Some(feature));
                    features.fmt();
                }
            }
            Source::Crate { name, .. } => {
                self.deps_mut().remove(name);
            }
        }
        Ok(true)
    }

    pub fn save(&self) -> Result<()> {
        let path = self.root.join("Cargo.toml");
        fs::write(&path, self.doc.to_string())
            .with_context(|| format!("could not write {}", path.display()))
    }

    fn install(&mut self, module: &Module) -> Result<()> {
        match &module.source {
            Source::Olvin { feature } => {
                let olvin = self.olvin_mut()?;
                if olvin.get("features").is_none() {
                    olvin.insert("features", Item::Value(Array::new().into()));
                }
                let features = olvin
                    .get_mut("features")
                    .and_then(Item::as_array_mut)
                    .context("`olvin.features` is not an array")?;
                features.push(feature.as_str());
                features.fmt();
            }
            Source::Crate {
                name,
                version,
                features,
            } => {
                let item = if features.is_empty() {
                    toml_edit::value(version.as_str())
                } else {
                    let mut dep = InlineTable::new();
                    dep.insert("version", version.as_str().into());
                    dep.insert(
                        "features",
                        features
                            .iter()
                            .map(String::as_str)
                            .collect::<Array>()
                            .into(),
                    );
                    Item::Value(dep.into())
                };
                self.deps_mut().insert(name, item);
            }
        }
        Ok(())
    }

    fn deps(&self) -> Option<&dyn TableLike> {
        self.doc.get("dependencies")?.as_table_like()
    }

    fn deps_mut(&mut self) -> &mut dyn TableLike {
        // A hand-edited `dependencies = 5` is not a table: start a fresh one.
        if !self
            .doc
            .get("dependencies")
            .is_some_and(Item::is_table_like)
        {
            self.doc["dependencies"] = toml_edit::table();
        }
        self.doc["dependencies"]
            .as_table_like_mut()
            // invariant: made a table just above.
            .expect("[dependencies] is a table")
    }

    fn olvin(&self) -> Option<&dyn TableLike> {
        self.deps()?.get("olvin")?.as_table_like()
    }

    fn olvin_mut(&mut self) -> Result<&mut dyn TableLike> {
        self.deps_mut()
            .get_mut("olvin")
            .and_then(Item::as_table_like_mut)
            .context("the `olvin` dependency is missing or not a table")
    }

    pub(crate) fn olvin_features(&self) -> Vec<String> {
        self.olvin()
            .and_then(|o| o.get("features"))
            .and_then(Item::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }
}

const RESERVED: &[&str] = &[
    "olvin",
    "log",
    "test",
    "core",
    "std",
    "alloc",
    "proc_macro",
    "as",
    "break",
    "const",
    "continue",
    "crate",
    "else",
    "enum",
    "extern",
    "false",
    "fn",
    "for",
    "if",
    "impl",
    "in",
    "let",
    "loop",
    "match",
    "mod",
    "move",
    "mut",
    "pub",
    "ref",
    "return",
    "self",
    "static",
    "struct",
    "super",
    "trait",
    "true",
    "type",
    "unsafe",
    "use",
    "where",
    "while",
    "async",
    "await",
    "dyn",
];

/// Why `name` cannot be a project name (and a package name), or `None`.
/// Empty is not a problem yet: nothing has been typed.
pub fn name_problem(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let ok = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !ok {
        return Some("Use Latin letters, digits, _ and -, starting with a letter.".into());
    }
    // Cargo refuses these as package names, or the project would depend on itself.
    let crate_name = name.replace('-', "_");
    RESERVED
        .contains(&crate_name.as_str())
        .then(|| "Reserved (a Rust keyword, a std crate or a dependency of the project).".into())
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("the project name is empty");
    }
    if name != name.trim() {
        bail!("`{name}`: the name has spaces at the ends");
    }
    match name_problem(name) {
        Some(problem) => bail!("`{name}`: {problem}"),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("olvin-hub-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn create_add_remove_roundtrip() {
        let dir = temp_dir("roundtrip");
        let catalog = Catalog::builtin();
        let engine = EngineSource::default();
        let mut p = Project::create(&dir, "my_game", &engine, &catalog).unwrap();

        let ids = |p: &Project| {
            p.installed(&catalog)
                .iter()
                .map(|m| m.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids(&p),
            [
                "window",
                "render",
                "scenes",
                "scripts",
                "input",
                "integrity"
            ]
        );

        // requires pulls in serde; re-adding is a no-op.
        assert_eq!(p.add(&catalog, "saves").unwrap(), ["serde", "saves"]);
        assert!(p.add(&catalog, "saves").unwrap().is_empty());
        // serde is needed by saves.
        assert!(p.remove(&catalog, "serde", false).is_err());
        assert!(p.remove(&catalog, "saves", false).unwrap());
        assert!(p.remove(&catalog, "window", true).unwrap());
        p.save().unwrap();

        let reopened = Project::open(p.root()).unwrap();
        assert_eq!(reopened.name(), "my_game");
        assert_eq!(
            ids(&reopened),
            ["render", "scenes", "scripts", "input", "integrity", "serde"]
        );
        let text = fs::read_to_string(p.root().join("Cargo.toml")).unwrap();
        assert!(
            text.contains(r#"serde = { version = "1", features = ["derive"] }"#),
            "{text}"
        );

        assert!(Project::create(&dir, "my_game", &engine, &catalog).is_err());
        assert!(Project::create(&dir, "1bad name", &engine, &catalog).is_err());
        // A project named like its own dependencies or a keyword would not build.
        for bad in ["olvin", "log", "fn", "Test", "std", "my-fn"] {
            let expect_err = !matches!(bad, "Test" | "my-fn");
            assert_eq!(
                Project::create(&dir, bad, &engine, &catalog).is_err(),
                expect_err,
                "{bad}"
            );
        }
        fs::remove_dir_all(dir).unwrap();
    }

    /// `scenes` alone pulls in `render` and `window`: the graphical template, not the headless one.
    #[test]
    fn requirements_count_for_the_graphical_template() {
        let dir = temp_dir("graphical");
        let catalog = Catalog::builtin();
        let engine = EngineSource::default();
        let p = Project::create_with(&dir, "g", &engine, &catalog, &["scenes"]).unwrap();
        let main = fs::read_to_string(p.root().join("src/main.rs")).unwrap();
        assert!(main.contains("DefaultPlugins"), "{main}");
        fs::remove_dir_all(dir).unwrap();
    }

    /// A failed create leaves no folder behind, and removal of a module together
    /// with its dependent works in any order and is all or nothing.
    #[test]
    fn failed_create_cleans_up_and_remove_many_is_atomic() {
        let dir = temp_dir("atomic");
        let catalog = Catalog::builtin();
        let engine = EngineSource::default();
        assert!(Project::create_with(&dir, "g", &engine, &catalog, &["nope"]).is_err());
        assert!(!dir.join("g").exists());

        let mut p = Project::create_with(&dir, "g", &engine, &catalog, &["saves"]).unwrap();
        let ids = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let before = p.doc.to_string();
        assert!(
            p.remove_many(&catalog, &ids(&["serde", "nope"]), false)
                .is_err()
        );
        assert_eq!(p.doc.to_string(), before);
        assert!(p.remove_many(&catalog, &ids(&["serde"]), false).is_err());
        assert_eq!(p.doc.to_string(), before);
        assert_eq!(
            p.remove_many(&catalog, &ids(&["serde", "saves", "rand"]), false)
                .unwrap(),
            ["serde", "saves"]
        );
        fs::remove_dir_all(dir).unwrap();
    }

    /// `lua` and `game` are facade features: they land in `olvin.features`
    /// (lua with the `script` it needs), and the template ignores build output.
    #[test]
    fn lua_and_game_modules_wire_facade_features() {
        let dir = temp_dir("lua-game");
        let catalog = Catalog::builtin();
        let engine = EngineSource::default();
        let mut p = Project::create_with(&dir, "g", &engine, &catalog, &["physics"]).unwrap();
        assert_eq!(p.add(&catalog, "lua").unwrap(), ["scripts", "lua"]);
        assert_eq!(p.add(&catalog, "game").unwrap(), ["game"]);
        assert!(
            p.remove(&catalog, "scripts", false).is_err(),
            "lua needs it"
        );
        p.save().unwrap();

        let reopened = Project::open(p.root()).unwrap();
        assert_eq!(
            reopened.olvin_features(),
            ["physics", "script", "lua", "game"]
        );
        let text = fs::read_to_string(p.root().join("Cargo.toml")).unwrap();
        assert!(
            text.contains(r#"features = ["physics", "script", "lua", "game"]"#),
            "{text}"
        );
        let ignore = fs::read_to_string(p.root().join(".gitignore")).unwrap();
        assert!(ignore.lines().any(|l| l == "/dist"), "{ignore}");
        // Headless template (no window/render/scenes), still not tied to the working dir.
        let main = fs::read_to_string(p.root().join("src/main.rs")).unwrap();
        assert!(main.contains("HeadlessPlugins"), "{main}");
        fs::remove_dir_all(dir).unwrap();
    }

    /// Every `kind = "olvin"` module names a real feature of the facade.
    #[test]
    fn olvin_modules_are_facade_features() {
        let manifest: toml::Table = include_str!("../../../crates/olvin/Cargo.toml")
            .parse()
            .unwrap();
        let features = manifest["features"].as_table().unwrap();
        for m in &Catalog::builtin().modules {
            if let Source::Olvin { feature } = &m.source {
                assert!(features.contains_key(feature), "{}: no `{feature}`", m.id);
            }
        }
    }

    #[test]
    fn edits_preserve_user_formatting() {
        let dir = temp_dir("format");
        fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"g\"\n\n[dependencies]\n# my engine\nolvin = { path = \"x\", features = [\"window\"] }\nfoo = \"1\" # keep me\n",
        )
        .unwrap();
        let catalog = Catalog::builtin();
        let mut p = Project::open(&dir).unwrap();
        p.add(&catalog, "rand").unwrap();
        p.save().unwrap();
        let text = fs::read_to_string(dir.join("Cargo.toml")).unwrap();
        assert!(text.contains("# my engine"));
        assert!(text.contains("foo = \"1\" # keep me"));
        assert!(text.contains("rand = \"0.10\""));
        fs::remove_dir_all(dir).unwrap();
    }
}
