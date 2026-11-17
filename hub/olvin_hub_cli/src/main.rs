//! `olvin-hub`: creates Olvin games and installs modules into them.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use olvin_hub::{Catalog, EngineSource, HubConfig, Project};

#[derive(Parser)]
#[command(name = "olvin-hub", version, about = "Хаб Olvin: проекты игр и модули")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Создать проект игры
    New {
        name: String,
        /// Папка, в которой создать проект (по умолчанию текущая)
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Каталог модулей; [x] — установлен в текущий проект
    List {
        #[arg(short, long)]
        project: Option<PathBuf>,
    },
    /// Установить модули (вместе с зависимостями)
    Add {
        #[arg(required = true)]
        modules: Vec<String>,
        #[arg(short, long)]
        project: Option<PathBuf>,
    },
    /// Удалить модули
    Remove {
        #[arg(required = true)]
        modules: Vec<String>,
        #[arg(short, long)]
        project: Option<PathBuf>,
        /// Удалить, даже если от модуля зависят другие
        #[arg(long)]
        force: bool,
    },
    /// Собрать и запустить игру
    Run {
        #[arg(short, long)]
        project: Option<PathBuf>,
        #[arg(long)]
        release: bool,
    },
    /// Известные хабу проекты
    Projects,
    /// Запомнить существующий проект
    Register { path: PathBuf },
    /// Забыть проект (файлы не трогаются)
    Forget { path: PathBuf },
    /// Показать или сменить источник движка для новых проектов
    Engine {
        #[arg(long, conflicts_with = "git")]
        path: Option<PathBuf>,
        #[arg(long)]
        git: Option<String>,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("ошибка: {err:#}");
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
            let installed: Vec<_> =
                project.installed(&catalog).iter().map(|m| m.id.as_str()).collect();
            println!("создан {} (движок: {})", project.root().display(), config.engine);
            println!("модули: {}", installed.join(", "));
            println!("запуск: cd {} && cargo run --release", project.root().display());
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
                Some(p) => println!("\nпроект: {} ({})", p.name(), p.root().display()),
                None => println!("\n(вне проекта: отметки установки не показаны)"),
            }
        }
        Cmd::Add { modules, project } => {
            let mut project = open(project.as_deref())?;
            for id in &modules {
                let added = project.add(&catalog, id)?;
                if added.is_empty() {
                    println!("{id}: уже установлен");
                } else {
                    println!("+ {}", added.join(", "));
                }
            }
            project.save()?;
        }
        Cmd::Remove { modules, project, force } => {
            let mut project = open(project.as_deref())?;
            for id in &modules {
                if project.remove(&catalog, id, force)? {
                    println!("- {id}");
                } else {
                    println!("{id}: не был установлен");
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
            let status = cmd.status().context("не удалось запустить cargo")?;
            if !status.success() {
                bail!("cargo завершился с {status}");
            }
        }
        Cmd::Projects => {
            if config.projects.is_empty() {
                println!("проектов нет: olvin-hub new <имя>");
            }
            for root in &config.projects {
                match Project::open(root) {
                    Ok(p) => println!("{:<20} {}", p.name(), root.display()),
                    Err(_) => println!("{:<20} {} (не найден)", "?", root.display()),
                }
            }
        }
        Cmd::Register { path } => {
            let project = Project::open(&path)?;
            if config.register(project.root())? {
                config.save()?;
                println!("добавлен {}", project.name());
            } else {
                println!("{} уже в списке", project.name());
            }
        }
        Cmd::Forget { path } => {
            if config.forget(&path) {
                config.save()?;
                println!("забыт {}", path.display());
            } else {
                println!("{} не было в списке", path.display());
            }
        }
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
            println!("движок: {}", config.engine);
        }
    }
    Ok(())
}

fn open(project: Option<&Path>) -> Result<Project> {
    match project {
        Some(path) => Project::open(path),
        None => Project::discover(&std::env::current_dir()?),
    }
}
