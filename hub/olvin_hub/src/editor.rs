//! The editor installation: a generated crate whose binary is olvin-editor
//! with the chosen elements linked in. Its `Cargo.toml` is the source of
//! truth for what is installed.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use anyhow::{Context, Result, bail};
use toml_edit::{DocumentMut, Item, Value};

use crate::{Catalog, EngineSource};

pub struct EditorInstall {
    root: PathBuf,
}

impl EditorInstall {
    /// `<data dir>/olvin-hub/editor`, e.g. `~/.local/share/olvin-hub/editor`.
    pub fn locate() -> Result<Self> {
        let data = dirs::data_dir().context("на этой платформе нет папки данных")?;
        Ok(Self::at(data.join("olvin-hub/editor")))
    }

    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn binary(&self) -> PathBuf {
        let name = if cfg!(windows) { "olvin-editor.exe" } else { "olvin-editor" };
        self.root.join("target/release").join(name)
    }

    pub fn is_generated(&self) -> bool {
        self.root.join("Cargo.toml").is_file()
    }

    pub fn is_built(&self) -> bool {
        self.binary().is_file()
    }

    /// Ids of installed editor elements, in catalog order.
    pub fn installed(&self, catalog: &Catalog) -> Vec<String> {
        let Ok(text) = std::fs::read_to_string(self.root.join("Cargo.toml")) else { return Vec::new() };
        let Ok(doc) = text.parse::<DocumentMut>() else { return Vec::new() };
        let Some(deps) = doc.get("dependencies").and_then(Item::as_table_like) else { return Vec::new() };
        catalog
            .editor_modules
            .iter()
            .filter(|m| deps.contains_key(&m.package))
            .map(|m| m.id.clone())
            .collect()
    }

    /// What a fresh install gets: catalog defaults, or what's there already.
    pub fn initial_selection(&self, catalog: &Catalog) -> Vec<String> {
        if self.is_generated() {
            self.installed(catalog)
        } else {
            catalog.editor_modules.iter().filter(|m| m.default).map(|m| m.id.clone()).collect()
        }
    }

    /// (Re)generates the installation crate for exactly these elements.
    pub fn write(&self, engine: &EngineSource, catalog: &Catalog, ids: &[String]) -> Result<()> {
        engine.validate()?;
        if let EngineSource::Path(path) = engine
            && !path.join("editor/olvin_editor/Cargo.toml").is_file()
        {
            bail!("{}: в этой версии Olvin нет редактора (editor/olvin_editor)", path.display());
        }
        for id in ids {
            if catalog.editor_module(id).is_none() {
                bail!("нет элемента редактора `{id}`");
            }
        }
        // Catalog order keeps the generated files stable.
        let modules: Vec<_> = catalog.editor_modules.iter().filter(|m| ids.contains(&m.id)).collect();

        let mut doc: DocumentMut = CARGO_TOML.parse().expect("template is valid TOML");
        let deps = doc["dependencies"].as_table_mut().unwrap();
        deps.insert("olvin_editor", Item::Value(Value::InlineTable(engine.crate_dependency("editor/olvin_editor"))));
        for m in &modules {
            deps.insert(&m.package, Item::Value(Value::InlineTable(engine.crate_dependency(&m.path))));
        }

        let plugins: String = modules.iter().map(|m| format!("        Box::new({}),\n", m.plugin)).collect();
        let names: Vec<_> = modules.iter().map(|m| m.name.as_str()).collect();
        let main = format!(
            "// Сгенерировано olvin-hub. Элементы: {}.\n// Не редактируй: файл перезаписывается командами `olvin-hub editor add/remove`.\n\nfn main() {{\n    olvin_editor::run(vec![\n{plugins}    ]);\n}}\n",
            if names.is_empty() { "только база".into() } else { names.join(", ") },
        );

        std::fs::create_dir_all(self.root.join("src"))?;
        std::fs::write(self.root.join("Cargo.toml"), doc.to_string())?;
        std::fs::write(self.root.join("src/main.rs"), main)?;
        Ok(())
    }

    /// `cargo build --release` of the installation. The caller decides how to run it.
    pub fn build_command(&self) -> Command {
        let mut cmd = Command::new("cargo");
        cmd.args(["build", "--release", "--manifest-path"]).arg(self.root.join("Cargo.toml"));
        cmd
    }

    pub fn launch(&self, project: Option<&Path>) -> Result<Child> {
        if !self.is_built() {
            bail!("редактор не собран: olvin-hub editor install");
        }
        let mut cmd = Command::new(self.binary());
        if let Some(project) = project {
            cmd.arg(project);
        }
        cmd.spawn().context("не удалось запустить olvin-editor")
    }
}

const CARGO_TOML: &str = r#"# Сгенерировано olvin-hub: установка редактора Olvin.
# Не редактируй вручную: файл перезаписывается командами `olvin-hub editor add/remove`.

[package]
name = "olvin-editor-install"
version = "0.1.0"
edition = "2024"
publish = false

[[bin]]
name = "olvin-editor"
path = "src/main.rs"

[dependencies]

[profile.release]
lto = "thin"

# Standalone: never part of an enclosing workspace.
[workspace]
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_install_with_selected_elements() {
        let dir = std::env::temp_dir().join(format!("olvin-hub-editor-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let install = EditorInstall::at(&dir);
        let catalog = Catalog::builtin();
        let engine = EngineSource::default();

        assert_eq!(install.initial_selection(&catalog), ["console", "csg"]);
        install.write(&engine, &catalog, &["animator".into(), "console".into()]).unwrap();
        assert_eq!(install.installed(&catalog), ["console", "animator"]);
        let main = std::fs::read_to_string(dir.join("src/main.rs")).unwrap();
        assert!(main.find("ConsolePlugin").unwrap() < main.find("AnimatorPlugin").unwrap());

        install.write(&engine, &catalog, &[]).unwrap();
        assert!(install.installed(&catalog).is_empty());
        assert!(std::fs::read_to_string(dir.join("src/main.rs")).unwrap().contains("vec![\n    ]"));
        assert!(install.write(&engine, &catalog, &["nope".into()]).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
