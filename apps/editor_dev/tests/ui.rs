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

const SIZE: egui::Vec2 = egui::vec2(1600.0, 1700.0);

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
    editor_with(project, true)
}

/// `fresh`: default workspaces and preferences instead of the saved ones.
fn editor_with(project: &Path, fresh: bool) -> Harness<'static, EditorApp> {
    isolate_config();
    let project = project.to_path_buf();
    let mut harness = Harness::builder()
        .with_size(SIZE)
        .with_step_dt(1.0 / 60.0)
        .build_eframe(move |cc| {
            olvin_editor::theme::apply(&cc.egui_ctx);
            let registry = olvin_editor::registry(&plugins());
            let mut app = EditorApp::new(registry, Some(project), None);
            if fresh {
                app.reset_layout();
                app.state_mut().prefs = Default::default();
                assert!(app.show_workspace("Layout"));
            }
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
    h.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::Z,
    );
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
    h.get_by_label("◼ Cube").click_secondary();
    h.run_steps(3);
    h.get_by_label("Rename").click();
    h.run_steps(3);
    field(&h, "Cube").type_text("Ящик №1");
    h.run_steps(2);
    for n in h.get_all_by_role(egui::accesskit::Role::TextInput) {
        eprintln!("field {:?}", n.accesskit_node().value());
    }
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    for n in h.get_all_by_role(egui::accesskit::Role::TextInput) {
        eprintln!("after enter field {:?}", n.accesskit_node().value());
    }
    eprintln!("name {:?}", state(&mut h).scene.get(3).unwrap().name);
    assert_eq!(state(&mut h).scene.get(3).unwrap().name, "CubeЯщик №1");
    // Escape cancels a rename.
    h.get_by_label("◼ CubeЯщик №1").click_secondary();
    h.run_steps(3);
    h.get_by_label("Rename").click();
    h.run_steps(3);
    field(&h, "CubeЯщик №1").type_text("zzz");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert_eq!(state(&mut h).scene.get(3).unwrap().name, "CubeЯщик №1");
}

#[test]
fn hotkeys_do_not_fire_while_typing() {
    let p = project("typing");
    let mut h = editor(&p);
    state(&mut h).select(Some(3));
    h.run_steps(3);
    field(&h, "Cube").focus();
    h.run_steps(2);
    h.key_press(egui::Key::Delete);
    h.key_press(egui::Key::X);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::D);
    h.run_steps(3);
    assert_eq!(names(&mut h).len(), 3, "{:?}", names(&mut h));
}

#[test]
fn game_object_menu_creates_and_undo_removes() {
    let p = project("menu-create");
    let mut h = editor(&p);
    h.get_by_label("GAMEOBJECT").click();
    h.run_steps(3);
    h.get_by_label("Create Empty").click();
    h.run_steps(3);
    assert_eq!(names(&mut h).len(), 4);
    assert_eq!(state(&mut h).selection, Some(4));
    undo(&mut h);
    assert_eq!(names(&mut h).len(), 3);
    redo(&mut h);
    assert_eq!(names(&mut h).len(), 4);
}

#[test]
fn new_scene_asks_before_discarding_changes() {
    let p = project("new-scene");
    let mut h = editor(&p);
    state(&mut h).scene.get_mut(3).unwrap().position.x = 5.0;
    h.run_steps(3);
    h.get_by_label("FILE").click();
    h.run_steps(3);
    h.get_by_label_contains("New Scene").click();
    h.run_steps(3);
    assert!(h.query_by_label("Unsaved Changes").is_some());
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert_eq!(names(&mut h).len(), 3);
    h.get_by_label("FILE").click();
    h.run_steps(3);
    h.get_by_label_contains("New Scene").click();
    h.run_steps(3);
    h.get_by_label("Don't Save").click();
    h.run_steps(3);
    assert_eq!(names(&mut h).len(), 4, "the starter scene");
}

/// The "+" of the top bar's workspace tabs (docks have their own).
fn workspace_plus(h: &Ed) -> egui_kittest::Node<'_> {
    h.get_all_by_label("+")
        .min_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
        .expect("workspace +")
}

#[test]
fn workspace_tabs_add_rename_close_and_persist() {
    let p = project("tabs");
    let mut h = editor(&p);
    workspace_plus(&h).click();
    h.run_steps(3);
    h.get_by_label("Empty").click();
    h.run_steps(3);
    // The new workspace is in rename mode.
    field(&h, "New").type_text("Мой воркспейс");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.query_by_label("NewМой воркспейс").is_some());
    // Layout survives a restart.
    eframe::App::on_exit(h.state_mut());
    let mut h = editor_with(&p, false);
    assert!(h.query_by_label("NewМой воркспейс").is_some());
    // Close it from the context menu.
    h.get_by_label("NewМой воркспейс").click_secondary();
    h.run_steps(3);
    h.get_by_label("Close").click();
    h.run_steps(3);
    assert!(h.query_by_label("NewМой воркспейс").is_none());
    assert!(h.query_by_label("Layout").is_some());
}

#[test]
fn script_param_key_can_be_renamed() {
    let p = project("script-param");
    let mut h = editor(&p);
    {
        let s = state(&mut h);
        s.scene
            .get_mut(3)
            .unwrap()
            .scripts
            .push(olvin_editor::olvin_scene::file::ScriptDesc {
                name: "Spin".into(),
                params: [("speed".to_owned(), "1".to_owned())].into_iter().collect(),
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
    let params = state(&mut h).scene.get(3).unwrap().scripts[0]
        .params
        .clone();
    assert!(params.contains_key("rate"), "{params:?}");
}

fn faces(h: &mut Ed, id: u64) -> usize {
    state(h)
        .scene
        .get(id)
        .and_then(|e| e.poly.as_ref())
        .map_or(0, |p| p.faces.len())
}

#[test]
fn edit_mode_operations_undo_and_redo() {
    let p = project("edit-undo");
    let mut h = editor(&p);
    assert!(h.state_mut().show_workspace("Modeling"));
    state(&mut h).select(Some(3));
    h.run_steps(3);
    h.get_by_label("◆ Edit Mode").click();
    h.run_steps(3);
    assert_eq!(faces(&mut h, 3), 6, "converted to a six-face poly");
    h.get_by_label("All  A").click();
    h.run_steps(3);
    h.get_by_label("Subdivide").click();
    h.run_steps(3);
    assert_eq!(faces(&mut h, 3), 24);
    h.get_by_label("Flip Normals").click();
    h.run_steps(3);
    undo(&mut h);
    assert_eq!(faces(&mut h, 3), 24, "undo of Flip keeps the subdivision");
    undo(&mut h);
    assert_eq!(faces(&mut h, 3), 6, "undo of Subdivide");
    // The vertex selection outlived the mesh it indexed; frames still run.
    h.run_steps(5);
    h.run_steps(3);
    let node = h.get_by_label_contains("vertices selected");
    let selected = node.accesskit_node().value().expect("label text");
    let count: usize = selected.split(' ').next().unwrap().parse().unwrap();
    assert!(
        count <= 8,
        "selection pruned to the cube's 8 vertices: {selected}"
    );
    h.get_by_label("None  Alt+A").click();
    h.run_steps(3);
    h.get_by_label("All  A").click();
    h.run_steps(3);
    h.get_by_label("Delete  X").click();
    h.run_steps(3);
    assert_eq!(faces(&mut h, 3), 0, "everything deleted");
    undo(&mut h);
    assert_eq!(faces(&mut h, 3), 6);
    redo(&mut h);
    redo(&mut h);
    h.run_steps(5);
    undo(&mut h);
    undo(&mut h);
    undo(&mut h);
    assert!(
        state(&mut h).scene.get(3).unwrap().poly.is_none(),
        "back to the plain cube"
    );
    h.run_steps(5);
}
