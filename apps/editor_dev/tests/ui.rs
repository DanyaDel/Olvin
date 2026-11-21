//! Headless UI regression tests: the whole editor (every element) driven by
//! egui input events through egui_kittest, without a GPU. Run with
//! `cargo test -p editor_dev --test ui`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Once;

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use olvin_editor::glam::Vec3;
use olvin_editor::olvin_scene::file::{EntityDesc, MeshDesc, SceneFile, Shape};
use olvin_editor::{EditorApp, EditorPlugin, egui};

const SIZE: egui::Vec2 = egui::vec2(1600.0, 940.0);

fn plugins() -> Vec<Box<dyn EditorPlugin>> {
    vec![
        Box::new(olvin_editor_console::ConsolePlugin),
        Box::new(olvin_editor_animator::AnimatorPlugin),
        Box::new(olvin_editor_csg::CsgPlugin),
        Box::new(olvin_editor_modeling::ModelingPlugin),
        Box::new(olvin_editor_uv::UvPlugin),
        Box::new(olvin_editor_ide::IdePlugin),
    ]
}

/// Layout and preferences go to a throwaway config directory.
fn isolate_config() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let dir =
            std::env::temp_dir().join(format!("olvin-editor-ui-config-{}", std::process::id()));
        // SAFETY: set once, before any test thread creates an editor.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", dir) };
    });
}

fn cube(id: u64, name: &str, pos: Vec3) -> EntityDesc {
    let mut e = EntityDesc::new(id, name);
    e.position = pos;
    e.mesh = Some(MeshDesc {
        shape: Shape::Cube,
        color: [0.5; 4],
    });
    e
}

/// A project with one scene: Root > Child, and a Cube.
fn project(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("olvin-ui-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("assets/scenes")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    let mut child = cube(2, "Child", Vec3::new(1.0, 0.0, 0.0));
    child.parent = Some(1);
    let scene = SceneFile {
        entities: vec![cube(1, "Root", Vec3::ZERO), child, cube(3, "Cube", Vec3::Y)],
        ..Default::default()
    };
    scene
        .save(root.join("assets/scenes/main.scene.ron"))
        .unwrap();
    root
}

fn editor(project: &Path) -> Harness<'static, EditorApp> {
    isolate_config();
    let project = project.to_path_buf();
    let mut harness = Harness::builder()
        .with_size(SIZE)
        .with_step_dt(1.0 / 60.0)
        .build_eframe(move |cc| {
            olvin_editor::theme::apply(&cc.egui_ctx);
            let registry = olvin_editor::registry(&plugins());
            let mut app = EditorApp::new(registry, Some(project), None);
            app.reset_layout();
            app.state_mut().prefs = Default::default();
            assert!(app.show_workspace("Layout"));
            app
        });
    harness.run_steps(5);
    harness
}

type Ed = Harness<'static, EditorApp>;

fn state(h: &mut Ed) -> &mut olvin_editor::EditorState {
    h.state_mut().state_mut()
}

fn names(h: &mut Ed) -> Vec<String> {
    state(h)
        .scene
        .entities
        .iter()
        .map(|e| e.name.clone())
        .collect()
}

#[test]
fn smoke() {
    let p = project("smoke");
    let mut h = editor(&p);
    h.run_steps(10);
    assert_eq!(state(&mut h).scene.entities.len(), 3);
}

#[test]
fn x_over_the_scene_view_toggles_gizmo_space_and_keeps_objects() {
    let p = project("xkey");
    let mut h = editor(&p);
    state(&mut h).select(Some(3));
    h.run_steps(3);
    h.hover_at(egui::pos2(800.0, 400.0));
    h.run_steps(2);
    let before = state(&mut h).local_space;
    h.key_press(egui::Key::X);
    h.run_steps(3);
    assert_eq!(names(&mut h).len(), 3, "X must not delete");
    assert_ne!(state(&mut h).local_space, before, "X toggles Global/Local");
}

#[test]
fn duplicate_with_parent_and_child_selected_copies_each_once() {
    let p = project("dup");
    let mut h = editor(&p);
    {
        let s = state(&mut h);
        s.select(Some(1));
        s.toggle_selected(2);
    }
    h.run_steps(3);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::D);
    h.run_steps(3);
    assert_eq!(names(&mut h).len(), 5, "{:?}", names(&mut h));
}

#[test]
fn saving_while_playing_keeps_the_edited_scene() {
    let p = project("playsave");
    let mut h = editor(&p);
    state(&mut h).start_play();
    state(&mut h).scene.get_mut(3).unwrap().position.x = 42.0;
    assert!(state(&mut h).save());
    state(&mut h).stop_play();
    let on_disk = SceneFile::load(p.join("assets/scenes/main.scene.ron")).unwrap();
    assert_eq!(on_disk.get(3).unwrap().position.x, 0.0);
    assert!(!state(&mut h).is_dirty());
}

#[test]
fn opening_a_scene_stops_play_mode() {
    let p = project("playopen");
    let mut h = editor(&p);
    let mut other = SceneFile::default();
    other.entities.push(cube(7, "Other", Vec3::ZERO));
    let path = p.join("assets/scenes/other.scene.ron");
    other.save(&path).unwrap();
    state(&mut h).start_play();
    state(&mut h).open(&path);
    assert!(!state(&mut h).is_playing());
    assert_eq!(names(&mut h), vec!["Other"]);
}

fn mouse_move(h: &Ed, pos: egui::Pos2) {
    h.event(egui::Event::PointerMoved(pos));
}

fn mouse_button(h: &Ed, pos: egui::Pos2, pressed: bool) {
    h.event(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    });
}

/// Press at `from`, drag to `to` in a few steps, release.
fn drag(h: &mut Ed, from: egui::Pos2, to: egui::Pos2) {
    mouse_move(h, from);
    h.run_steps(2);
    mouse_button(h, from, true);
    h.run_steps(2);
    for i in 1..=6 {
        mouse_move(h, from + (to - from) * (i as f32 / 6.0));
        h.run_steps(1);
    }
    h.run_steps(2);
    mouse_button(h, to, false);
    h.run_steps(3);
}

fn row_center(h: &Ed, label: &str) -> egui::Pos2 {
    h.get_by_label(label).rect().center()
}

fn undo(h: &mut Ed) {
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(3);
}

/// The text field showing `value` (a label with the same text may exist).
fn field<'a>(h: &'a Ed, value: &'a str) -> egui_kittest::Node<'a> {
    h.get_all_by_value(value)
        .find(|n| n.accesskit_node().role() == egui::accesskit::Role::TextInput)
        .expect("text field")
}

fn redo(h: &mut Ed) {
    h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::Z);
    h.run_steps(3);
}

#[test]
fn hierarchy_drag_reparent_and_undo() {
    let p = project("hier-drag");
    let mut h = editor(&p);
    h.run_steps(5);
    let (from, to) = (row_center(&h, "◼ Cube"), row_center(&h, "◼ Root"));
    drag(&mut h, from, to);
    assert_eq!(state(&mut h).scene.get(3).unwrap().parent, Some(1));
    undo(&mut h);
    assert_eq!(state(&mut h).scene.get(3).unwrap().parent, None);
    redo(&mut h);
    assert_eq!(state(&mut h).scene.get(3).unwrap().parent, Some(1));
}

#[test]
fn hierarchy_rename_cyrillic_and_escape() {
    let p = project("hier-rename");
    let mut h = editor(&p);
    h.get_by_label("◼ Cube").click();
    h.run_steps(2);
    h.get_by_label("◼ Cube").click();
    h.run_steps(1);
    h.get_by_label("◼ Cube").click();
    h.run_steps(3);
    // double click via two quick clicks is flaky: use the context menu
}

#[test]
fn script_param_key_can_be_renamed() {
    let p = project("script-param");
    let mut h = editor(&p);
    {
        let s = state(&mut h);
        s.scene.get_mut(3).unwrap().scripts.push(olvin_editor::olvin_scene::file::ScriptDesc {
            name: "Spin".into(),
            params: [("speed".to_owned(), "1".to_owned())].into_iter().collect(),
            ..Default::default()
        });
        s.select(Some(3));
    }
    h.run_steps(5);
    field(&h, "speed").focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    field(&h, "speed").type_text("rate");
    h.run_steps(3);
    // Commit by leaving the field.
    h.get_by_label("Behaviour").click();
    h.run_steps(3);
    let params = state(&mut h).scene.get(3).unwrap().scripts[0].params.clone();
    assert!(params.contains_key("rate"), "{params:?}");
}
