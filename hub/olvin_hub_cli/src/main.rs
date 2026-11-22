//! `olvin-hub`: creates Olvin games and installs modules into them.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use olvin_hub::{
    ArchiveFormat, BuildOptions, Catalog, Category, EditorInstall, EngineSource, HubConfig,
    OnConflict, Project, Status,
};

/// Text logo for the help header: `Olvin_` with a green underscore cursor
/// (assets/logo/README.md). clap drops the colour itself under NO_COLOR or
/// when the output is not a terminal.
fn logo() -> clap::builder::StyledStr {
    use clap::builder::styling::{AnsiColor, Style};
    let cursor = Style::new()
        .fg_color(Some(AnsiColor::BrightGreen.into()))
        .bold();
    format!("Olvin{cursor}_{cursor:#}").into()
}

#[derive(Parser)]
#[command(
    name = "olvin-hub",
    version,
    about = "Olvin hub: game projects and modules",
    before_help = logo(),
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create a game project
    New {
        name: String,
        /// Folder to create the project in (default: the current one)
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Module catalog; [x] marks modules installed in the current project
    List {
        #[arg(short, long)]
        project: Option<PathBuf>,
    },
    /// Install modules (with what they require)
    Add {
        #[arg(required = true)]
        modules: Vec<String>,
        #[arg(short, long)]
        project: Option<PathBuf>,
    },
    /// Remove modules
    Remove {
        #[arg(required = true)]
        modules: Vec<String>,
        #[arg(short, long)]
        project: Option<PathBuf>,
        /// Remove even if other modules require it
        #[arg(long)]
        force: bool,
    },
    /// Build and run the game
    Run {
        #[arg(short, long)]
        project: Option<PathBuf>,
        #[arg(long)]
        release: bool,
    },
    /// Build the game and lay out a folder (or archive) for players: executable + assets
    Build {
        #[arg(short, long)]
        project: Option<PathBuf>,
        /// Optimized build (cargo build --release)
        #[arg(long)]
        release: bool,
        /// Output folder (default: <project>/dist/<name>)
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Also pack the folder: tar.gz or zip
        #[arg(short, long, value_name = "FORMAT")]
        archive: Option<String>,
    },
    /// Projects the hub knows about
    Projects,
    /// Add an existing project to the list
    Register { path: PathBuf },
    /// Remove a project from the list (files stay)
    Forget { path: PathBuf },
    /// Editor: install, elements, launch
    Editor {
        #[command(subcommand)]
        command: EditorCmd,
    },
    /// Preset library: ready prefabs, scripts, shaders and looks
    Library {
        #[command(subcommand)]
        command: LibraryCmd,
    },
    /// Show or change the engine source for new projects
    Engine {
        #[arg(long, conflicts_with = "git")]
        path: Option<PathBuf>,
        #[arg(long)]
        git: Option<String>,
    },
}

#[derive(Subcommand)]
enum LibraryCmd {
    /// Packs of the library; [x] marks the ones installed in the project
    List {
        /// Words that must appear in the id, name, description or tags
        query: Vec<String>,
        /// One category: characters, gameplay, shaders, materials, looks, prefabs, audio, misc
        #[arg(short, long)]
        category: Option<String>,
        #[arg(short, long)]
        project: Option<PathBuf>,
    },
    /// Everything about one pack
    Show {
        id: String,
        #[arg(short, long)]
        project: Option<PathBuf>,
    },
    /// Import a pack (and what it requires) into the project's assets/
    Add {
        id: String,
        #[arg(short, long)]
        project: Option<PathBuf>,
        /// On a clash with a different file: overwrite it
        #[arg(long, conflicts_with_all = ["keep_both", "skip"])]
        replace: bool,
        /// On a clash: write the pack's file next to it with a -2 suffix
        #[arg(long, conflicts_with = "skip")]
        keep_both: bool,
        /// On a clash: keep the project's file, import the rest
        #[arg(long)]
        skip: bool,
    },
    /// Packs installed in the project (library.lock.ron)
    Installed {
        #[arg(short, long)]
        project: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum EditorCmd {
    /// Build the editor (default elements on first install)
    Install,
    /// Editor elements; [x] marks installed ones
    List,
    /// Add elements and rebuild
    Add {
        #[arg(required = true)]
        elements: Vec<String>,
    },
    /// Remove elements and rebuild
    Remove {
        #[arg(required = true)]
        elements: Vec<String>,
    },
    /// Open a project in the editor (default: the project in the current folder)
    Open { project: Option<PathBuf> },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let catalog = Catalog::builtin();
    // Read only where it is needed: a damaged config must not block `build` or `add`.

    match cli.command {
        Cmd::New { name, dir } => {
            let mut config = HubConfig::load()?;
            let parent = match dir {
                Some(dir) => dir,
                None => std::env::current_dir()?,
            };
            let project = Project::create(&parent, &name, &config.engine, &catalog)?;
            config.register(project.root())?;
            config.save()?;
            let installed: Vec<_> = project
                .installed(&catalog)
                .iter()
                .map(|m| m.id.as_str())
                .collect();
            println!(
                "created {} (engine: {})",
                project.root().display(),
                config.engine
            );
            println!("modules: {}", installed.join(", "));
            println!(
                "run: cd {} && cargo run --release",
                project.root().display()
            );
        }
        Cmd::List { project } => {
            let project = open(project.as_deref()).ok();
            for category in catalog.categories() {
                println!("{category}");
                for m in catalog.modules.iter().filter(|m| m.category == category) {
                    let mark = match &project {
                        Some(p) if p.is_installed(m) => "[x]",
                        Some(_) => "[ ]",
                        None => "  -",
                    };
                    println!("  {mark} {:<9} {} — {}", m.id, m.name, m.description);
                }
            }
            match project {
                Some(p) => println!("\nproject: {} ({})", p.name(), p.root().display()),
                None => println!("\n(outside a project: install marks not shown)"),
            }
        }
        Cmd::Add { modules, project } => {
            let mut project = open(project.as_deref())?;
            // Nothing is saved or printed if one of them fails.
            let mut lines = Vec::new();
            for id in &modules {
                let added = project.add(&catalog, id)?;
                lines.push(if added.is_empty() {
                    format!("{id}: already installed")
                } else {
                    format!("+ {}", added.join(", "))
                });
            }
            project.save()?;
            for line in lines {
                println!("{line}");
            }
        }
        Cmd::Remove {
            modules,
            project,
            force,
        } => {
            let mut project = open(project.as_deref())?;
            let removed = project.remove_many(&catalog, &modules, force)?;
            project.save()?;
            for id in &modules {
                if removed.contains(id) {
                    println!("- {id}");
                } else {
                    println!("{id}: was not installed");
                }
            }
        }
        Cmd::Run { project, release } => {
            let project = open(project.as_deref())?;
            let mut cmd = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
            cmd.arg("run").current_dir(project.root());
            if release {
                cmd.arg("--release");
            }
            let status = cmd.status().context("could not start cargo")?;
            if !status.success() {
                bail!("cargo exited with {status}");
            }
        }
        Cmd::Build {
            project,
            release,
            out,
            archive,
        } => {
            let project = open(project.as_deref())?;
            let archive = archive.as_deref().map(ArchiveFormat::parse).transpose()?;
            println!(
                "building {} ({})…",
                project.name(),
                if release { "release" } else { "debug" }
            );
            let report = project.build(&BuildOptions {
                release,
                out,
                archive,
                target_dir: None,
            })?;
            println!("game:    {}", report.binary.display());
            println!(
                "folder:  {} ({} asset files)",
                report.out_dir.display(),
                report.assets
            );
            if report.licenses > 0 {
                println!("licenses: {} files in licenses/", report.licenses);
            }
            if let Some(archive) = report.archive {
                println!("archive: {}", archive.display());
            }
            if report.gitignore_updated {
                println!("added /dist to .gitignore");
            }
        }
        Cmd::Projects => {
            let config = HubConfig::load()?;
            if config.projects.is_empty() {
                println!("No projects yet. Create one: olvin-hub new <name>");
            }
            for root in &config.projects {
                match Project::open(root) {
                    Ok(p) => println!("{:<20} {}", p.name(), root.display()),
                    Err(_) => println!("{:<20} {} (not found)", "?", root.display()),
                }
            }
        }
        Cmd::Register { path } => {
            let mut config = HubConfig::load()?;
            let project = Project::open(&path)?;
            if config.register(project.root())? {
                config.save()?;
                println!("added {}", project.name());
            } else {
                println!("{} is already in the list", project.name());
            }
        }
        Cmd::Forget { path } => {
            let mut config = HubConfig::load()?;
            if config.forget(&path) {
                config.save()?;
                println!("removed {} from the list", path.display());
            } else {
                println!("{} was not in the list", path.display());
            }
        }
        Cmd::Library { command } => library(command)?,
        Cmd::Editor { command } => editor(command, &catalog, &HubConfig::load()?)?,
        Cmd::Engine { path, git } => {
            let mut config = HubConfig::load()?;
            let new = match (path, git) {
                (Some(path), _) => Some(EngineSource::Path(path.canonicalize()?)),
                (_, Some(url)) => Some(EngineSource::Git(url)),
                _ => None,
            };
            if let Some(engine) = new {
                engine.validate()?;
                config.engine = engine;
                config.save()?;
            }
            println!("engine: {}", config.engine);
        }
    }
    Ok(())
}

fn library(command: LibraryCmd) -> Result<()> {
    match command {
        LibraryCmd::List {
            query,
            category,
            project,
        } => {
            let project = open(project.as_deref()).ok();
            let category = category
                .as_deref()
                .map(|c| Category::parse(c).with_context(|| format!("unknown category `{c}`")))
                .transpose()?;
            let library = olvin_hub::library_for(project.as_ref());
            for problem in library.problems() {
                eprintln!("warning: {problem}");
            }
            let rows =
                olvin_hub::library_rows(&library, project.as_ref(), &query.join(" "), category);
            let mut last = None;
            for row in &rows {
                if last != Some(row.category) {
                    println!("{}", row.category);
                    last = Some(row.category);
                }
                let mark = match &row.status {
                    Some(Status::NotInstalled) => "[ ]",
                    Some(_) => "[x]",
                    None => "  -",
                };
                println!(
                    "  {mark} {:<24} {:<7} {} — {}",
                    row.id, row.version, row.name, row.description
                );
            }
            if rows.is_empty() {
                println!("No packs match.");
            }
            if project.is_none() {
                println!("\n(outside a project: install marks not shown)");
            }
        }
        LibraryCmd::Show { id, project } => {
            let project = open(project.as_deref()).ok();
            let library = olvin_hub::library_for(project.as_ref());
            print!(
                "{}",
                olvin_hub::library_describe(&library, &id, project.as_ref())?
            );
        }
        LibraryCmd::Add {
            id,
            project,
            replace,
            keep_both,
            skip,
        } => {
            let project = open(project.as_deref())?;
            let library = olvin_hub::library_for(Some(&project));
            let policy = if replace {
                OnConflict::Replace
            } else if keep_both {
                OnConflict::KeepBoth
            } else if skip {
                OnConflict::Skip
            } else {
                OnConflict::Abort
            };
            let report = olvin_hub::library_add(&library, &project, &id, policy)?;
            for pack in &report.packs {
                println!(
                    "{} {}: {} added, {} replaced, {} renamed, {} identical, {} kept",
                    pack.id,
                    pack.version,
                    pack.added.len(),
                    pack.replaced.len(),
                    pack.renamed.len(),
                    pack.identical.len(),
                    pack.skipped.len()
                );
                for (from, to) in &pack.renamed {
                    println!("  {from} -> {to}");
                }
            }
            for w in &report.warnings {
                eprintln!("warning: {w}");
            }
            for (pack, notes) in &report.notes {
                println!("\n{pack}: {notes}");
            }
        }
        LibraryCmd::Installed { project } => {
            let project = open(project.as_deref())?;
            let packs = olvin_hub::library_installed(&project)?;
            if packs.is_empty() {
                println!("No packs installed. Browse them: olvin-hub library list");
            }
            for (id, version, explicit, files) in packs {
                let how = if explicit { "" } else { " (dependency)" };
                println!("{id:<24} {version:<7} {files} files{how}");
            }
        }
    }
    Ok(())
}

fn editor(command: EditorCmd, catalog: &Catalog, config: &HubConfig) -> Result<()> {
    let install = EditorInstall::locate()?;
    let current = install.initial_selection(catalog);
    let mut selection = current.clone();
    match command {
        EditorCmd::Install => {}
        EditorCmd::List => {
            println!("Core (always): scene, hierarchy, inspector, assets");
            for m in &catalog.editor_modules {
                let mark = if install.is_generated() && current.contains(&m.id) {
                    "[x]"
                } else {
                    "[ ]"
                };
                println!("  {mark} {:<9} {} — {}", m.id, m.name, m.description);
            }
            if install.is_stale(&config.engine) {
                println!(
                    "\n⚠ the engine changed after the build. Rebuild: olvin-hub editor install"
                );
            }
            match install.is_built() {
                true => println!("\ninstalled: {}", install.binary().display()),
                false => println!("\nnot installed. Install: olvin-hub editor install"),
            }
            return Ok(());
        }
        EditorCmd::Add { elements } => {
            for id in elements {
                if catalog.editor_module(&id).is_none() {
                    bail!("no editor element `{id}`. See the list: olvin-hub editor list");
                }
                if !selection.contains(&id) {
                    selection.push(id);
                }
            }
        }
        EditorCmd::Remove { elements } => {
            for id in &elements {
                if catalog.editor_module(id).is_none() {
                    bail!("no editor element `{id}`. See the list: olvin-hub editor list");
                }
            }
            selection.retain(|id| !elements.contains(id));
        }
        EditorCmd::Open { project } => {
            let project = match project {
                Some(path) => Some(Project::open(&path)?),
                None => Project::discover(&std::env::current_dir()?).ok(),
            };
            if !install.is_built() {
                build_editor(&install, catalog, config, &selection)?;
            } else if install.is_stale(&config.engine) {
                println!("the engine changed after the editor build, rebuilding");
                build_editor(&install, catalog, config, &selection)?;
            }
            install.launch(project.as_ref().map(Project::root))?;
            match &project {
                Some(p) => println!("editor opened: {}", p.name()),
                None => println!("editor opened without a project"),
            }
            return Ok(());
        }
    }
    build_editor(&install, catalog, config, &selection)
}

fn build_editor(
    install: &EditorInstall,
    catalog: &Catalog,
    config: &HubConfig,
    ids: &[String],
) -> Result<()> {
    install.write(&config.engine, catalog, ids)?;
    let names: Vec<_> = catalog
        .editor_modules
        .iter()
        .filter(|m| ids.contains(&m.id))
        .map(|m| m.name.as_str())
        .collect();
    println!(
        "building the editor (elements: {})…",
        if names.is_empty() {
            "core only".into()
        } else {
            names.join(", ")
        }
    );
    let status = install
        .build_command()
        .status()
        .context("could not start cargo")?;
    if !status.success() {
        bail!("editor build failed ({status})");
    }
    install.mark_built(&config.engine);
    println!("done: {}", install.binary().display());
    Ok(())
}

fn open(project: Option<&Path>) -> Result<Project> {
    match project {
        Some(path) => Project::open(path),
        None => Project::discover(&std::env::current_dir()?),
    }
}
