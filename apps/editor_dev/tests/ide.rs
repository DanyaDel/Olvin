//! Drives the IDE element inside the full editor (`olvin_editor::build_app`,
//! the "Code" workspace) with real pointer and key events, the way a user
//! would: open files from the explorer, type, save, run cargo, use dialogs.
//! Ignored by default (needs a GPU or llvmpipe), like screenshots.rs:
//!
//! ```sh
//! __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json WGPU_BACKEND=gl \
//! cargo test -p editor_dev --test ide -- --ignored
//! ```
//!
//! Screenshots go to `$OLVIN_IDE_SHOTS` (default: a folder in the temp dir).
//! The temp projects live in `<target>/ide-tests`, not in /tmp.

#![allow(dead_code)]

#[path = "screenshots.rs"]
mod shots;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use olvin_editor::EditorApp;
use olvin_editor::egui::{self, Key, Modifiers as M};
use olvin_editor_ide::ide;

type H = Harness<'static, EditorApp>;

// Screen positions in the 1400x860 "Code" workspace: explorer rows, the
// toolbar buttons and the Problems / Output tabs.
const MAIN_RS: (f32, f32) = (80.0, 173.0);
const NEW_FILE_BTN: (f32, f32) = (176.0, 92.0);
const CHECK_BTN: (f32, f32) = (1354.0, 91.0);
const RUN_BTN: (f32, f32) = (1221.0, 91.0);
const STOP_BTN: (f32, f32) = (1160.0, 91.0);
const OUTPUT_TAB: (f32, f32) = (446.0, 623.0);
const PROBLEM_ROW: (f32, f32) = (500.0, 691.0);
const EDITOR_BLANK: (f32, f32) = (700.0, 300.0);

fn shots_dir() -> PathBuf {
    let d = std::env::var_os("OLVIN_IDE_SHOTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("olvin-ide-shots"));
    fs::create_dir_all(&d).unwrap();
    d
}

/// A tiny cargo project (its own `[workspace]`, so it never joins ours).
fn project(name: &str) -> PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"));
    let root = target.join("ide-tests").join(name);
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("assets/shaders")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[workspace]\n",
    )
    .unwrap();
    fs::write(
        root.join("src/main.rs"),
        "fn main() {\n    let x: i32 = \"no\";\n    println!(\"hi {x}\");\n}\n",
    )
    .unwrap();
    fs::write(
        root.join("assets/shaders/s.wgsl"),
        olvin_editor_ide::shader::template(),
    )
    .unwrap();
    root.canonicalize().unwrap()
}

fn harness(project: &Path) -> H {
    shots::isolate_config();
    let project = project.to_path_buf();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 860.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .renderer(shots::EditorRenderer::new())
        .build_eframe(move |cc| -> EditorApp {
            let plugins: Vec<Box<dyn olvin_editor::EditorPlugin>> =
                vec![Box::new(olvin_editor_ide::IdePlugin)];
            let registry = olvin_editor::registry(&plugins);
            let mut app = olvin_editor::build_app(cc, registry, Some(project)).expect("app");
            app.reset_layout();
            assert!(app.show_workspace("Code"));
            app
        });
    h.run_steps(20);
    h
}

fn shot(h: &mut H, name: &str) {
    h.remove_cursor();
    let img = h.render().expect("render");
    image::DynamicImage::ImageRgba8(img)
        .to_rgb8()
        .save(shots_dir().join(name))
        .unwrap();
}

fn click_with(h: &mut H, (x, y): (f32, f32), button: egui::PointerButton) {
    let pos = egui::pos2(x, y);
    h.event(egui::Event::PointerMoved(pos));
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: M::NONE,
        });
        h.run_steps(1);
    }
    h.run_steps(3);
}

fn click(h: &mut H, at: (f32, f32)) {
    click_with(h, at, egui::PointerButton::Primary);
}

fn right_click(h: &mut H, at: (f32, f32)) {
    click_with(h, at, egui::PointerButton::Secondary);
}

fn key(h: &mut H, k: Key, m: M) {
    h.key_press_modifiers(m, k);
    h.run_steps(3);
}

fn type_text(h: &mut H, text: &str) {
    h.event(egui::Event::Text(text.into()));
    h.run_steps(3);
}

fn doc_text() -> String {
    ide::with(|i| i.active_doc().map(|d| d.text.clone()).unwrap_or_default())
}

/// Steps the harness until `done`, sleeping between steps (cargo and the
/// disk poll run on real time).
fn wait(h: &mut H, secs: u64, mut done: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while !done() {
        if start.elapsed() > Duration::from_secs(secs) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
        h.run_steps(2);
    }
    true
}

fn open_main(h: &mut H) {
    click(h, MAIN_RS);
    assert_eq!(ide::with(|i| i.docs.len()), 1, "clicking main.rs opens it");
    click(h, EDITOR_BLANK);
}

#[test]
#[ignore = "renders with wgpu; run with --ignored"]
fn explorer_click_opens_types_and_saves() {
    let p = project("open");
    let mut h = harness(&p);
    shot(&mut h, "01-code-workspace.png");
    // Regression: a click-sensing "empty space" widget registered after the
    // rows used to swallow every click on them.
    click(&mut h, MAIN_RS);
    ide::with(|i| {
        assert_eq!(i.docs.len(), 1);
        assert!(
            i.selected
                .as_deref()
                .is_some_and(|s| s.ends_with("main.rs"))
        );
    });
    click(&mut h, EDITOR_BLANK);
    type_text(&mut h, "abc");
    assert!(doc_text().ends_with("}\nabc"), "{:?}", doc_text());
    assert!(ide::with(|i| i.active_doc().unwrap().dirty));
    key(&mut h, Key::S, M::COMMAND);
    assert!(!ide::with(|i| i.active_doc().unwrap().dirty));
    assert!(
        fs::read_to_string(p.join("src/main.rs"))
            .unwrap()
            .ends_with("}\nabc")
    );
    shot(&mut h, "02-typed-and-saved.png");
}

#[test]
#[ignore = "renders with wgpu; run with --ignored"]
fn find_bar_and_go_to_line_hand_focus_back() {
    let p = project("find");
    let mut h = harness(&p);
    open_main(&mut h);
    key(&mut h, Key::F, M::COMMAND);
    type_text(&mut h, "println");
    key(&mut h, Key::Enter, M::NONE);
    shot(&mut h, "03-find.png");
    // Regression: Esc closed nothing and left no focus, so Ctrl+G and
    // Ctrl+S (which the editor only sees while focused) went nowhere.
    key(&mut h, Key::Escape, M::NONE);
    type_text(&mut h, "Q");
    assert!(doc_text().contains("printlnQ"), "{:?}", doc_text());
    key(&mut h, Key::G, M::COMMAND);
    assert!(
        ide::with(|i| i.palette.is_some()),
        "Ctrl+G opens the palette"
    );
    key(&mut h, Key::Escape, M::NONE);
    assert!(ide::with(|i| i.palette.is_none()));
    // Esc in the palette gives the keyboard back, too.
    key(&mut h, Key::S, M::COMMAND);
    assert!(
        fs::read_to_string(p.join("src/main.rs"))
            .unwrap()
            .contains("printlnQ")
    );
}

#[test]
#[ignore = "renders with wgpu; run with --ignored"]
fn editing_helpers() {
    let p = project("edit");
    let mut h = harness(&p);
    open_main(&mut h);
    type_text(&mut h, "fn f(");
    key(&mut h, Key::Enter, M::NONE);
    key(&mut h, Key::Tab, M::NONE);
    // Enter after an open bracket indents; Tab adds one more level.
    assert!(doc_text().ends_with("fn f(\n        "), "{:?}", doc_text());
    key(&mut h, Key::Z, M::COMMAND);
    assert!(!doc_text().contains("fn f("), "undo");
    key(&mut h, Key::Z, M::COMMAND | M::SHIFT);
    assert!(doc_text().contains("fn f("), "redo");
    key(&mut h, Key::A, M::COMMAND);
    key(&mut h, Key::Slash, M::COMMAND);
    assert!(doc_text().starts_with("// fn main() {\n//     let x"));
    key(&mut h, Key::Slash, M::COMMAND);
    assert!(doc_text().starts_with("fn main() {\n    let x"));
    shot(&mut h, "04-edited.png");
}

#[test]
#[ignore = "runs cargo; renders with wgpu; run with --ignored"]
fn cargo_check_shows_clickable_problem() {
    let p = project("check");
    let mut h = harness(&p);
    click(&mut h, CHECK_BTN);
    assert!(wait(&mut h, 120, || ide::with(|i| i.last_job.is_some())));
    h.run_steps(5);
    ide::with(|i| {
        assert_eq!(
            i.last_job,
            Some((olvin_editor_ide::job::JobKind::Check, false))
        );
        assert_eq!(i.diagnostics.len(), 1);
        let d = &i.diagnostics[0];
        assert!(d.path.ends_with("src/main.rs") && d.line == 2 && d.column == 18);
    });
    shot(&mut h, "05-problems.png");
    click(&mut h, PROBLEM_ROW);
    ide::with(|i| {
        assert!(i.active_path().is_some_and(|p| p.ends_with("src/main.rs")));
        let doc = i.active_doc().unwrap();
        assert_eq!(doc.goto, None, "the jump was applied");
    });
    shot(&mut h, "06-problem-jump.png");
}

#[test]
#[ignore = "runs cargo; renders with wgpu; run with --ignored"]
fn run_streams_output_and_stop_kills() {
    let p = project("run");
    fs::write(
        p.join("src/main.rs"),
        "fn main() {\n    for i in 0.. {\n        println!(\"tick {i}\");\n        std::thread::sleep(std::time::Duration::from_millis(50));\n    }\n}\n",
    )
    .unwrap();
    let mut h = harness(&p);
    click(&mut h, RUN_BTN);
    assert!(wait(&mut h, 120, || ide::with(|i| i
        .output
        .iter()
        .any(|l| l.text.starts_with("tick 3")))));
    assert!(ide::with(|i| i.job_running()));
    click(&mut h, OUTPUT_TAB);
    shot(&mut h, "07-output-running.png");
    click(&mut h, STOP_BTN);
    assert!(wait(&mut h, 10, || !ide::with(|i| i.job_running())));
    assert!(ide::with(|i| i
        .output
        .last()
        .is_some_and(|l| l.text.starts_with("Run stopped"))));
    shot(&mut h, "08-output-stopped.png");
}

#[test]
#[ignore = "renders with wgpu; run with --ignored"]
fn new_file_shader_errors_and_external_reload() {
    let p = project("files");
    let mut h = harness(&p);
    click(&mut h, NEW_FILE_BTN);
    shot(&mut h, "09-new-file-dialog.png");
    type_text(&mut h, "util.rs");
    key(&mut h, Key::Enter, M::NONE);
    assert!(p.join("util.rs").exists());
    // Regression: the Enter that confirmed the dialog also reached the
    // freshly focused editor and typed a newline into the empty file.
    ide::with(|i| {
        let d = i.active_doc().unwrap();
        assert_eq!(d.text, "");
        assert!(!d.dirty);
    });
    type_text(&mut h, "x");
    assert_eq!(doc_text(), "x", "the new file has the keyboard");

    // Quick open the surface shader and break it.
    key(&mut h, Key::E, M::COMMAND);
    type_text(&mut h, "s.wgsl");
    key(&mut h, Key::Enter, M::NONE);
    ide::with(|i| assert_eq!(i.active_doc().unwrap().shader, Some(vec![])));
    click(&mut h, (800.0, 494.0));
    type_text(&mut h, "x");
    assert!(wait(&mut h, 5, || ide::with(|i| i
        .active_doc()
        .unwrap()
        .shader
        .as_ref()
        .is_some_and(|e| !e.is_empty()))));
    shot(&mut h, "10-shader-error.png");

    // A change on disk reloads a clean tab.
    ide::with(|i| i.docs[0].save().unwrap());
    fs::write(p.join("util.rs"), "// changed outside\n").unwrap();
    assert!(wait(&mut h, 5, || ide::with(
        |i| i.docs[0].text == "// changed outside\n"
    )));
}

#[test]
#[ignore = "renders with wgpu; run with --ignored"]
fn rename_close_dirty_and_delete() {
    let p = project("rename");
    let mut h = harness(&p);
    open_main(&mut h);
    type_text(&mut h, "z");
    right_click(&mut h, MAIN_RS);
    shot(&mut h, "11-context-menu.png");
    h.get_by_label("Rename…").click();
    h.run_steps(3);
    key(&mut h, Key::A, M::COMMAND);
    type_text(&mut h, "game.rs");
    key(&mut h, Key::Enter, M::NONE);
    assert!(p.join("src/game.rs").exists() && !p.join("src/main.rs").exists());
    assert!(ide::with(|i| i
        .active_path()
        .is_some_and(|p| p.ends_with("game.rs"))));
    // The dialog gave the keyboard back to the code.
    type_text(&mut h, "q");
    assert!(doc_text().contains('q'));
    key(&mut h, Key::W, M::COMMAND);
    assert!(
        ide::with(|i| i.dialog.is_some()),
        "closing a dirty tab asks first"
    );
    shot(&mut h, "12-close-dirty.png");
    h.get_by_label("Don't Save").click();
    h.run_steps(3);
    assert!(ide::with(|i| i.docs.is_empty()));
    right_click(&mut h, MAIN_RS);
    h.get_by_label("Delete…").click();
    h.run_steps(3);
    shot(&mut h, "13-delete-dialog.png");
    key(&mut h, Key::Enter, M::NONE);
    assert!(!p.join("src/game.rs").exists());
}
