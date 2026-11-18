use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind")]
pub enum Source {
    /// A feature of the `olvin` facade crate.
    #[serde(rename = "olvin")]
    Olvin { feature: String },
    /// A plain crates.io dependency.
    #[serde(rename = "crate")]
    Crate {
        #[serde(rename = "crate")]
        name: String,
        version: String,
        #[serde(default)]
        features: Vec<String>,
    },
}

#[derive(Clone, Debug, Deserialize)]
pub struct Module {
    pub id: String,
    pub name: String,
    pub category: String,
    pub description: String,
    #[serde(flatten)]
    pub source: Source,
    #[serde(default)]
    pub requires: Vec<String>,
    /// Installed into new projects.
    #[serde(default)]
    pub default: bool,
}

/// An element of the editor itself (a plugin crate of olvin-editor).
#[derive(Clone, Debug, Deserialize)]
pub struct EditorModule {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Package name.
    #[serde(rename = "crate")]
    pub package: String,
    /// Location inside the Olvin repository.
    pub path: String,
    /// Path of the type implementing `EditorPlugin`.
    pub plugin: String,
    #[serde(default)]
    pub default: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Catalog {
    #[serde(rename = "module")]
    pub modules: Vec<Module>,
    #[serde(rename = "editor_module", default)]
    pub editor_modules: Vec<EditorModule>,
}

impl Catalog {
    pub fn builtin() -> Self {
        Self::parse(include_str!("../catalog.toml")).expect("built-in catalog is valid")
    }

    pub fn parse(text: &str) -> Result<Self> {
        let catalog: Self = toml::from_str(text).context("module catalog is damaged")?;
        for m in &catalog.modules {
            for r in &m.requires {
                if catalog.get(r).is_none() {
                    bail!("module `{}` requires unknown module `{r}`", m.id);
                }
            }
        }
        Ok(catalog)
    }

    pub fn get(&self, id: &str) -> Option<&Module> {
        self.modules.iter().find(|m| m.id == id)
    }

    pub fn editor_module(&self, id: &str) -> Option<&EditorModule> {
        self.editor_modules.iter().find(|m| m.id == id)
    }

    /// `id` and everything it requires, dependencies first.
    pub fn resolve(&self, id: &str) -> Result<Vec<&Module>> {
        let mut order = Vec::new();
        self.visit(id, &mut order, &mut Vec::new())?;
        Ok(order)
    }

    fn visit<'a>(
        &'a self,
        id: &str,
        order: &mut Vec<&'a Module>,
        stack: &mut Vec<String>,
    ) -> Result<()> {
        let Some(module) = self.get(id) else {
            bail!("no module `{id}`. See the list: olvin-hub list")
        };
        if order.iter().any(|m| m.id == id) {
            return Ok(());
        }
        if stack.iter().any(|s| s == id) {
            bail!("circular dependency: {} -> {id}", stack.join(" -> "));
        }
        stack.push(id.to_owned());
        for r in &module.requires {
            self.visit(r, order, stack)?;
        }
        stack.pop();
        order.push(module);
        Ok(())
    }

    /// Categories in first-appearance order.
    pub fn categories(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for m in &self.modules {
            if !out.contains(&m.category.as_str()) {
                out.push(&m.category);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_parses_and_resolves_requires_first() {
        let c = Catalog::builtin();
        let ids: Vec<_> = c
            .resolve("render")
            .unwrap()
            .iter()
            .map(|m| m.id.as_str())
            .collect();
        assert_eq!(ids, ["window", "render"]);
        assert!(c.resolve("nope").is_err());
    }

    #[test]
    fn rejects_cycles_and_unknown_requires() {
        let cyclic = r#"
            [[module]]
            id = "a"
            name = "A"
            category = "x"
            description = ""
            kind = "olvin"
            feature = "a"
            requires = ["b"]
            [[module]]
            id = "b"
            name = "B"
            category = "x"
            description = ""
            kind = "olvin"
            feature = "b"
            requires = ["a"]
        "#;
        assert!(Catalog::parse(cyclic).unwrap().resolve("a").is_err());
        assert!(Catalog::parse(&cyclic.replace("[\"a\"]", "[\"zzz\"]")).is_err());
    }
}
