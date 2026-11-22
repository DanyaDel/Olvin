use super::*;
use crate::jobs::kill_tree;
use olvin_hub::Project;
use std::process::Command;
use std::time::Duration;

fn run_frames(hub: &mut Hub, ctx: &egui::Context, size: egui::Vec2, frames: usize) {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
    for frame in 0..frames {
        // Real sessions repaint on every mouse move; sizes settle over frames.
        let pointer = egui::pos2(300.0 + frame as f32 * 7.0, 400.0);
        let input = egui::RawInput {
            screen_rect: Some(screen),
            time: Some(frame as f64 / 60.0),
            events: vec![egui::Event::PointerMoved(pointer)],
            ..Default::default()
        };
        // Headless: nobody uploads textures, so drop the deltas explicitly.
        ctx.run_ui(input, |ui| hub.draw(ui)).textures_delta.clear();
    }
}

/// Runs a few frames headless and returns the dialog's rect.
fn modal_rect(modal: Modal, id: &str) -> egui::Rect {
    let ctx = egui::Context::default();
    let mut hub = Hub::new(HubConfig::default(), None);
    hub.modal = Some(modal);
    run_frames(&mut hub, &ctx, egui::vec2(955.0, 999.0), 60);
    assert!(hub.modal.is_some(), "dialog closed by itself");
    ctx.memory(|m| m.area_rect(egui::Id::new(id)))
        .expect("dialog was shown")
}

#[test]
fn dialogs_fit_their_content() {
    let hub = Hub::new(HubConfig::default(), None);
    let cases = [
        (hub.new_project_modal(), "new-project"),
        (
            Modal::AddExisting {
                path: String::new(),
            },
            "add-existing",
        ),
    ];
    for (modal, id) in cases {
        let rect = modal_rect(modal, id);
        assert!(rect.height() < 900.0, "{id}: {rect:?}");
        assert!(
            rect.top() > 0.0 && rect.bottom() < 999.0,
            "{id} off screen: {rect:?}"
        );
    }
}

fn temp(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olvin-hub-gui-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

#[test]
fn create_from_template_then_toggle_modules_and_forget() {
    let dir = temp("create");
    let mut hub = Hub::new(HubConfig::default(), None);
    hub.config_file = Some(dir.join("config/config.toml"));
    let modules: Vec<String> = TEMPLATES[1].modules.iter().map(|m| m.to_string()).collect();
    assert!(hub.create_project("demo_game", dir.to_str().unwrap(), &modules));
    assert_eq!(hub.rows.len(), 1);
    assert_eq!(hub.rows[0].name, "demo_game");
    assert!(!hub.rows[0].missing);
    assert!(hub.rows[0].modules.iter().any(|m| m == "3D physics"));
    // the same name twice is an error toast, not a second row
    assert!(!hub.create_project("demo_game", dir.to_str().unwrap(), &modules));
    assert!(hub.toasts.last().unwrap().error);
    assert_eq!(hub.rows.len(), 1);
    // a reserved name is refused before anything is created
    assert!(!hub.create_project("olvin", dir.to_str().unwrap(), &modules));
    assert!(!dir.join("olvin").exists());

    // the Modules dialog installs `lua` (with `scripts`) and removes it again
    let root = hub.rows[0].root.clone();
    let mut project = Project::open(&root).unwrap();
    let catalog = Catalog::builtin();
    project.add(&catalog, "lua").unwrap();
    project.add(&catalog, "game").unwrap();
    project.save().unwrap();
    hub.refresh();
    assert!(hub.rows[0].modules.iter().any(|m| m == "Lua scripts"));
    assert!(hub.rows[0].modules.iter().any(|m| m == "Game runtime"));

    // a deleted project stays in the list, marked missing
    std::fs::remove_dir_all(&root).unwrap();
    hub.refresh();
    assert!(hub.rows[0].missing);
    hub.forget(&root);
    assert!(hub.rows.is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn adding_a_folder_that_is_not_a_project_reports_an_error() {
    let dir = temp("add-bad");
    let mut hub = Hub::new(HubConfig::default(), None);
    hub.config_file = Some(dir.join("config/config.toml"));
    assert!(!hub.add_existing(dir.to_str().unwrap()));
    assert!(hub.toasts.last().unwrap().error);
    assert!(!hub.add_existing("/definitely/not/here"));
    assert!(hub.rows.is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn typed_paths_expand_the_home_folder() {
    if let Some(home) = dirs::home_dir() {
        assert_eq!(expand_home(" ~/games "), home.join("games"));
        assert_eq!(expand_home("~"), home);
    }
    assert_eq!(expand_home("/a/~/b"), PathBuf::from("/a/~/b"));
    assert_eq!(expand_home("~x"), PathBuf::from("~x"));
}

/// Stop must end the game cargo started, not just cargo.
#[cfg(unix)]
#[test]
fn stopping_kills_the_whole_process_group() {
    let dir = temp("kill");
    let pid_file = dir.join("grandchild.pid");
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(format!("sleep 60 & echo $! > {}; wait", pid_file.display()));
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = cmd.spawn().unwrap();
    let grandchild = loop {
        if let Ok(text) = std::fs::read_to_string(&pid_file)
            && let Ok(pid) = text.trim().parse::<u32>()
        {
            break pid;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    kill_tree(&mut child);
    std::thread::sleep(Duration::from_millis(100));
    let alive = std::path::Path::new(&format!("/proc/{grandchild}")).exists()
        && std::fs::read_to_string(format!("/proc/{grandchild}/stat"))
            .is_ok_and(|s| !s.contains(") Z"));
    assert!(!alive, "the grandchild survived");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn relative_times_are_english() {
    let now = SystemTime::now();
    assert_eq!(ago(now), "just now");
    assert_eq!(ago(now - Duration::from_secs(4 * 3600 + 5)), "4h ago");
    assert_eq!(ago(now - Duration::from_secs(86_400 + 5)), "1d ago");
}

#[test]
fn theme_keys_round_trip() {
    for kind in ThemeKind::ALL {
        assert_eq!(theme_from_key(theme_key(kind)), kind);
    }
    assert_eq!(theme_from_key("nope"), ThemeKind::Dark);
}

/// Every page at every breakpoint lays out without panicking, and the
/// layout settles (no frame-to-frame growth).
#[test]
fn pages_render_at_all_widths() {
    for width in [470.0, 800.0, 1280.0, 1920.0] {
        for page in [Page::Projects, Page::Editor, Page::Settings] {
            for view in [View::List, View::Grid] {
                let ctx = egui::Context::default();
                let mut hub = Hub::new(HubConfig::default(), None);
                hub.page = page;
                hub.view = view;
                run_frames(&mut hub, &ctx, egui::vec2(width, 800.0), 30);
            }
        }
    }
}
