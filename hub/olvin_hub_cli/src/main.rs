//! `olvin-hub`: creates Olvin games and installs modules into them.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use olvin_hub::{Catalog, EditorInstall, EngineSource, HubConfig, Project};

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
    /// Show or change the engine source for new projects
    Engine {
        #[arg(long, conflicts_with = "git")]
        path: Option<PathBuf>,
        #[arg(long)]
        git: Option<String>,
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
    let mut config = HubConfig::load()?;

    match cli.command {
        Cmd::New { name, dir } => {
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
            for id in &modules {
                let added = project.add(&catalog, id)?;
                if added.is_empty() {
                    println!("{id}: already installed");
                } else {
                    println!("+ {}", added.join(", "));
                }
            }
            project.save()?;
        }
        Cmd::Remove {
            modules,
            project,
            force,
        } => {
            let mut project = open(project.as_deref())?;
            for id in &modules {
                if project.remove(&catalog, id, force)? {
                    println!("- {id}");
                } else {
                    println!("{id}: was not installed");
                }
            }
            project.save()?;
        }
        Cmd::Run { project, release } => {
            let project = open(project.as_deref())?;
            let mut cmd = Command::new("cargo");
            cmd.arg("run").current_dir(project.root());
            if release {
                cmd.arg("--release");
            }
            let status = cmd.status().context("could not start cargo")?;
            if !status.success() {
                bail!("cargo exited with {status}");
            }
        }
        Cmd::Projects => {
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
            let project = Project::open(&path)?;
            if config.register(project.root())? {
                config.save()?;
                println!("added {}", project.name());
            } else {
                println!("{} is already in the list", project.name());
            }
        }
        Cmd::Forget { path } => {
            if config.forget(&path) {
                config.save()?;
                println!("removed {} from the list", path.display());
            } else {
                println!("{} was not in the list", path.display());
            }
        }
        Cmd::Editor { command } => editor(command, &catalog, &config)?,
        Cmd::Engine { path, git } => {
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
        EditorCmd::Remove { elements } => selection.retain(|id| !elements.contains(id)),
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
