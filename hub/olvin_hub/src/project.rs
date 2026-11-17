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
        let text =
            fs::read_to_string(&path).with_context(|| format!("не удалось прочитать {}", path.display()))?;
        let doc: DocumentMut =
            text.parse().with_context(|| format!("не удалось разобрать {}", path.display()))?;
        let project = Self { root, doc };
        if project.olvin().is_none() {
            bail!("{} не проект Olvin: нет зависимости `olvin`", path.display());
        }
        Ok(project)
    }

    /// The nearest Olvin project at or above `start`.
    pub fn discover(start: &Path) -> Result<Self> {
        start
            .ancestors()
            .filter(|dir| dir.join("Cargo.toml").is_file())
            .find_map(|dir| Self::open(dir).ok())
            .with_context(|| format!("в {} и выше нет проекта Olvin", start.display()))
    }

    /// Creates `<parent>/<name>` from the template and installs default modules.
    pub fn create(
        parent: &Path,
        name: &str,
        engine: &EngineSource,
        catalog: &Catalog,
    ) -> Result<Self> {
        validate_name(name)?;
        engine.validate()?;
        let root = parent.join(name);
        if root.exists() {
            bail!("{} уже существует", root.display());
        }
        fs::create_dir_all(root.join("src"))?;
        for (file, content) in template::files(name) {
            fs::write(root.join(file), content)?;
        }

        let mut doc: DocumentMut = template::cargo_toml(name).parse()?;
        doc["dependencies"]
            .as_table_like_mut()
            .unwrap()
            .insert("olvin", Item::Value(Value::InlineTable(engine.dependency())));
        let mut project = Self { root, doc };
        for module in catalog.modules.iter().filter(|m| m.default) {
            project.add(catalog, &module.id)?;
        }
        project.save()?;
        Ok(project)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn name(&self) -> &str {
        self.doc.get("package").and_then(|p| p.get("name")).and_then(Item::as_str).unwrap_or("?")
    }

    pub fn is_installed(&self, module: &Module) -> bool {
        match &module.source {
            Source::Olvin { feature } => self.olvin_features().iter().any(|f| f == feature),
            Source::Crate { name, .. } => self.deps().is_some_and(|d| d.contains_key(name)),
        }
    }

    pub fn installed<'c>(&self, catalog: &'c Catalog) -> Vec<&'c Module> {
        catalog.modules.iter().filter(|m| self.is_installed(m)).collect()
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
        let Some(module) = catalog.get(id) else { bail!("нет модуля `{id}`") };
        if !self.is_installed(module) {
            return Ok(false);
        }
        let dependents: Vec<_> = self
            .installed(catalog)
            .into_iter()
            .filter(|m| m.requires.iter().any(|r| r == id))
            .map(|m| m.id.as_str())
            .collect();
        if !dependents.is_empty() && !force {
            bail!("`{id}` нужен модулям: {}", dependents.join(", "));
        }
        match &module.source {
            Source::Olvin { feature } => {
                if let Some(features) = self.olvin_mut()?.get_mut("features").and_then(Item::as_array_mut) {
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
        fs::write(&path, self.doc.to_string()).with_context(|| format!("не удалось записать {}", path.display()))
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
                    .context("`olvin.features` не массив")?;
                features.push(feature.as_str());
                features.fmt();
            }
            Source::Crate { name, version, features } => {
                let item = if features.is_empty() {
                    toml_edit::value(version.as_str())
                } else {
                    let mut dep = InlineTable::new();
                    dep.insert("version", version.as_str().into());
                    dep.insert("features", features.iter().map(String::as_str).collect::<Array>().into());
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
        if self.doc.get("dependencies").is_none() {
            self.doc["dependencies"] = toml_edit::table();
        }
        self.doc["dependencies"].as_table_like_mut().expect("[dependencies] is a table")
    }

    fn olvin(&self) -> Option<&dyn TableLike> {
        self.deps()?.get("olvin")?.as_table_like()
    }

    fn olvin_mut(&mut self) -> Result<&mut dyn TableLike> {
        self.deps_mut()
            .get_mut("olvin")
            .and_then(Item::as_table_like_mut)
            .context("зависимость `olvin` отсутствует или записана не таблицей")
    }

    fn olvin_features(&self) -> Vec<String> {
        self.olvin()
            .and_then(|o| o.get("features"))
            .and_then(Item::as_array)
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
            .unwrap_or_default()
    }
}

fn validate_name(name: &str) -> Result<()> {
    let ok = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !ok {
        bail!("`{name}`: только латиница, цифры, `_` и `-`, начинаться с буквы");
    }
    Ok(())
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

        let ids = |p: &Project| p.installed(&catalog).iter().map(|m| m.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&p), ["window", "render", "scenes"]);

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
        assert_eq!(ids(&reopened), ["render", "scenes", "serde"]);
        let text = fs::read_to_string(p.root().join("Cargo.toml")).unwrap();
        assert!(text.contains(r#"serde = { version = "1", features = ["derive"] }"#), "{text}");

        assert!(Project::create(&dir, "my_game", &engine, &catalog).is_err());
        assert!(Project::create(&dir, "1bad name", &engine, &catalog).is_err());
        fs::remove_dir_all(dir).unwrap();
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
