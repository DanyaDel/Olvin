//! The preset library as the CLI and the hub window use it: the packs
//! compiled into the binary plus `$OLVIN_LIBRARY` and `<project>/library/`
//! (see `olvin_library`).

use anyhow::{Result, anyhow, bail};
use olvin_library::{
    Category, ImportOptions, ImportReport, Library, OnConflict, Pack, Source, Status,
};

use crate::Project;

/// One line of the pack list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryRow {
    pub id: String,
    pub name: String,
    pub version: String,
    pub category: Category,
    /// `None` outside a project.
    pub status: Option<Status>,
    pub description: String,
}

/// The library as seen from `project` (its `library/` folder adds packs).
pub fn library_for(project: Option<&Project>) -> Library {
    Library::for_project(project.map(Project::root))
}

/// Packs matching `query` and `category`, with their install state in `project`.
pub fn library_rows(
    library: &Library,
    project: Option<&Project>,
    query: &str,
    category: Option<Category>,
) -> Vec<LibraryRow> {
    library
        .search(query, category)
        .into_iter()
        .map(|p| LibraryRow {
            id: p.manifest.id.clone(),
            name: p.manifest.name.clone(),
            version: p.manifest.version.clone(),
            category: p.manifest.category,
            status: project.map(|pr| library.status(p.id(), pr.root())),
            description: p.manifest.description.clone(),
        })
        .collect()
}

/// A multi-line description of one pack: what `library show` prints.
pub fn library_describe(library: &Library, id: &str, project: Option<&Project>) -> Result<String> {
    let pack: &Pack = library
        .get(id)
        .ok_or_else(|| anyhow!("no pack `{id}` in the library (olvin-hub library list)"))?;
    let m = &pack.manifest;
    let mut out = format!("{} {} ({})\n{}\n\n", m.name, m.version, m.id, m.description);
    out += &format!("category: {}\n", m.category);
    if !m.tags.is_empty() {
        out += &format!("tags:     {}\n", m.tags.join(", "));
    }
    out += &format!("author:   {}\nlicense:  {}\n", m.author, m.license);
    out += &match &pack.source {
        Source::Builtin => "source:   built in\n".to_owned(),
        Source::Folder(dir) => format!("source:   {}\n", dir.display()),
    };
    if !m.requires.is_empty() {
        out += &format!("requires: {}\n", m.requires.join(", "));
    }
    if let Some(project) = project {
        out += &format!(
            "status:   {}\n",
            match library.status(id, project.root()) {
                Status::NotInstalled => "not installed".to_owned(),
                Status::Installed(v) => format!("installed ({v})"),
                Status::Outdated {
                    installed,
                    available,
                } => format!("installed {installed}, library has {available}"),
            }
        );
    }
    out += &format!(
        "files:    {} ({} KB)\n",
        pack.files.len(),
        pack.total_bytes().div_ceil(1024)
    );
    if !m.provides.is_empty() {
        out += "provides:\n";
        for p in &m.provides {
            out += &format!("  {p}\n");
        }
    }
    if let Some(notes) = m.notes.as_deref().filter(|n| !n.trim().is_empty()) {
        out += &format!("\n{notes}\n");
    }
    Ok(out)
}

/// Imports `id` (and what it requires) into `project`. With the default
/// [`OnConflict::Abort`] a clash with the project's own files imports nothing
/// and the report lists them; the command line turns that into an error.
pub fn library_add(
    library: &Library,
    project: &Project,
    id: &str,
    on_conflict: OnConflict,
) -> Result<ImportReport> {
    let report = library
        .import(id, project.root(), &ImportOptions { on_conflict })
        .map_err(|e| anyhow!("{e}"))?;
    if !report.completed {
        let list: Vec<String> = report
            .conflicts
            .iter()
            .map(|(pack, path)| format!("  {path} ({pack})"))
            .collect();
        bail!(
            "{} file(s) already exist with other content, nothing was imported:\n{}\n\
             rerun with --replace, --keep-both or --skip",
            report.conflicts.len(),
            list.join("\n")
        );
    }
    Ok(report)
}

/// `(id, version, explicit, file count)` of the packs installed in `project`.
pub fn library_installed(project: &Project) -> Result<Vec<(String, String, bool, usize)>> {
    olvin_library::installed(project.root())
        .map_err(|e| anyhow!("{e}"))
        .map(|packs| {
            packs
                .into_iter()
                .map(|p| (p.id, p.version, p.explicit, p.files.len()))
                .collect()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Catalog, EngineSource};

    fn project(test: &str) -> (std::path::PathBuf, Project) {
        let dir = std::env::temp_dir().join(format!("olvin-hub-lib-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = Project::create(&dir, "g", &EngineSource::default(), &Catalog::builtin()).unwrap();
        (dir.join("g"), p)
    }

    /// A project that ships its own pack under `library/`.
    fn with_user_pack(root: &std::path::Path) {
        let pack = root.join("library/my-pack");
        std::fs::create_dir_all(pack.join("assets/scripts/my-pack")).unwrap();
        std::fs::write(
            pack.join("pack.ron"),
            r#"(id: "my-pack", name: "Mine", version: "0.2.0", category: Gameplay,
                description: "A user pack.", author: "me", license: "CC0-1.0",
                provides: ["scripts/my-pack/a.lua"], notes: "Use it.")"#,
        )
        .unwrap();
        std::fs::write(pack.join("assets/scripts/my-pack/a.lua"), "return {}").unwrap();
    }

    #[test]
    fn lists_shows_adds_and_remembers() {
        let (root, project) = project("flow");
        with_user_pack(&root);
        let lib = library_for(Some(&project));
        assert!(lib.problems().is_empty(), "{:?}", lib.problems());

        let rows = library_rows(&lib, Some(&project), "user", None);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, Some(Status::NotInstalled));
        assert!(library_rows(&lib, Some(&project), "user", Some(Category::Shaders)).is_empty());

        let text = library_describe(&lib, "my-pack", Some(&project)).unwrap();
        assert!(
            text.contains("Mine 0.2.0") && text.contains("not installed"),
            "{text}"
        );
        assert!(library_describe(&lib, "nope", None).is_err());

        let report = library_add(&lib, &project, "my-pack", OnConflict::Abort).unwrap();
        assert_eq!(report.written(), 1);
        assert!(root.join("assets/scripts/my-pack/a.lua").is_file());
        assert_eq!(
            library_installed(&project).unwrap(),
            [("my-pack".to_owned(), "0.2.0".to_owned(), true, 1)]
        );
        assert_eq!(
            library_rows(&lib, Some(&project), "", None)
                .iter()
                .find(|r| r.id == "my-pack")
                .unwrap()
                .status,
            Some(Status::Installed("0.2.0".into()))
        );
    }

    #[test]
    fn a_clash_is_an_error_until_a_policy_is_chosen() {
        let (root, project) = project("clash");
        with_user_pack(&root);
        let lib = library_for(Some(&project));
        let mine = root.join("assets/scripts/my-pack/a.lua");
        std::fs::create_dir_all(mine.parent().unwrap()).unwrap();
        std::fs::write(&mine, "-- mine").unwrap();
        let err = library_add(&lib, &project, "my-pack", OnConflict::Abort).unwrap_err();
        assert!(err.to_string().contains("--keep-both"), "{err}");
        assert_eq!(std::fs::read_to_string(&mine).unwrap(), "-- mine");
        let report = library_add(&lib, &project, "my-pack", OnConflict::KeepBoth).unwrap();
        assert_eq!(report.packs[0].renamed.len(), 1);
        assert_eq!(std::fs::read_to_string(&mine).unwrap(), "-- mine");
    }
}
