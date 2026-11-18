//! Headless screenshots of the hub at the layout breakpoints, rendered with
//! wgpu through egui_kittest. Ignored by default (needs a GPU or llvmpipe):
//!
//! ```sh
//! __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json WGPU_BACKEND=gl \
//! OLVIN_HUB_SHOTS=/some/dir [OLVIN_HUB_SHOTS_ROOT=/tmp/olvin-demo] cargo test -p olvin_hub_gui -- --ignored screenshots
//! ```

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use eframe::egui;
use egui_kittest::Harness;
use olvin_hub::{Catalog, EditorInstall, EngineSource, HubConfig, Project};

use crate::{Hub, Modal, Page, View, ui::ThemeKind};

/// A few demo projects with different modules and ages, plus one missing.
fn demo_projects(dir: &std::path::Path) -> Vec<PathBuf> {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).unwrap();
    let catalog = Catalog::builtin();
    let engine = EngineSource::default();
    let specs: [(&str, &[&str], u64); 5] = [
        (
            "bright_tomorrow",
            &[
                "window", "render", "scenes", "scripts", "input", "physics", "audio",
            ],
            4 * 3600,
        ),
        (
            "lights_out",
            &["window", "render", "scenes", "scripts", "input"],
            26 * 3600,
        ),
        (
            "aurora_racer",
            &[
                "window", "render", "scenes", "physics", "gltf", "saves", "rand",
            ],
            9 * 86_400,
        ),
        ("dedicated_server", &[], 40 * 86_400),
        (
            "a_project_with_a_really_long_name_that_will_not_fit",
            &["window", "render"],
            400 * 86_400,
        ),
    ];
    let mut roots = Vec::new();
    for (name, modules, age) in specs {
        let p = Project::create_with(dir, name, &engine, &catalog, modules).unwrap();
        let when = SystemTime::now() - Duration::from_secs(age);
        for f in ["Cargo.toml", "src/main.rs"] {
            let file = fs::File::options()
                .write(true)
                .open(p.root().join(f))
                .unwrap();
            file.set_modified(when).unwrap();
        }
        roots.push(p.root().canonicalize().unwrap());
    }
    roots.push(PathBuf::from("/nonexistent/old_prototype"));
    roots
}

#[test]
#[ignore = "renders PNGs with wgpu; run with --ignored"]
fn screenshots() {
    let out = std::env::var_os("OLVIN_HUB_SHOTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/hub-shots")
        });
    fs::create_dir_all(&out).unwrap();
    // OLVIN_HUB_SHOTS_ROOT: where the demo projects live, so README shots can
    // show a tidy path instead of a pid-suffixed temp dir.
    let tmp = std::env::var_os("OLVIN_HUB_SHOTS_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("olvin-hub-shots-{}", std::process::id()))
        });
    let projects = demo_projects(&tmp.join("projects"));

    #[derive(Clone, Copy)]
    enum Extra {
        None,
        NewProject,
        Modules,
        Toast,
        Empty,
    }
    let sizes = [
        (470.0, 760.0),
        (800.0, 700.0),
        (1280.0, 800.0),
        (1920.0, 1080.0),
    ];
    let scenes: &[(&str, Page, View, ThemeKind, Extra)] = &[
        (
            "projects",
            Page::Projects,
            View::List,
            ThemeKind::Dark,
            Extra::Toast,
        ),
        (
            "grid",
            Page::Projects,
            View::Grid,
            ThemeKind::Dark,
            Extra::None,
        ),
        (
            "editor",
            Page::Editor,
            View::List,
            ThemeKind::Dark,
            Extra::None,
        ),
        (
            "settings",
            Page::Settings,
            View::List,
            ThemeKind::Dark,
            Extra::None,
        ),
        (
            "new-project",
            Page::Projects,
            View::List,
            ThemeKind::Dark,
            Extra::NewProject,
        ),
        (
            "modules",
            Page::Projects,
            View::List,
            ThemeKind::Dark,
            Extra::Modules,
        ),
        (
            "light",
            Page::Projects,
            View::List,
            ThemeKind::Light,
            Extra::None,
        ),
        (
            "graphite",
            Page::Projects,
            View::Grid,
            ThemeKind::Graphite,
            Extra::None,
        ),
        (
            "contrast",
            Page::Settings,
            View::List,
            ThemeKind::HighContrast,
            Extra::None,
        ),
        (
            "empty",
            Page::Projects,
            View::List,
            ThemeKind::Dark,
            Extra::Empty,
        ),
    ];
    for (w, h) in sizes {
        for &(tag, page, view, theme, extra) in scenes {
            let mut config = HubConfig::default();
            if !matches!(extra, Extra::Empty) {
                config.projects = projects.clone();
            }
            let mut hub = Hub::new(config, Some(EditorInstall::at(tmp.join("editor"))));
            hub.page = page;
            hub.view = view;
            hub.theme = theme;
            match extra {
                Extra::NewProject => hub.modal = Some(hub.new_project_modal()),
                Extra::Modules => {
                    hub.modal = Some(Modal::Modules {
                        root: projects[0].clone(),
                    })
                }
                Extra::Toast => {
                    hub.toast("Created bright_tomorrow");
                    hub.toast_error(
                        "Could not read /home/me/game/Cargo.toml. Check that the folder still exists.",
                    );
                }
                Extra::None | Extra::Empty => {}
            }
            let mut harness = Harness::builder()
                .with_size(egui::vec2(w, h))
                .with_pixels_per_point(1.0)
                .with_step_dt(1.0 / 60.0)
                .wgpu()
                // The real `App::ui` path: no test margin around the window.
                .build_eframe(move |cc| {
                    hub.sync_style(&cc.egui_ctx);
                    hub
                });
            // 1.5 s: intro typed, entrances and dialog pop finished.
            harness.run_steps(90);
            let image = harness.render().expect("wgpu render");
            image
                .save(out.join(format!("{tag}-{}.png", w as u32)))
                .unwrap();
        }
    }
    let _ = fs::remove_dir_all(&tmp);
}
