//! Headless UI regression tests: the whole editor (every element) driven by
//! egui input events through egui_kittest, without a GPU. Run with
//! `cargo test -p editor_dev --test ui`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Once;

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use olvin_editor::glam::Vec3;
use olvin_editor::olvin_scene::file::{BrushDesc, CsgOp, EntityDesc, MeshDesc, SceneFile, Shape};
use olvin_editor::{EditorApp, EditorPlugin, egui};

const SIZE: egui::Vec2 = egui::vec2(1600.0, 1700.0);

fn plugins() -> Vec<Box<dyn EditorPlugin>> {
    vec![
        Box::new(olvin_editor_console::ConsolePlugin),
        Box::new(olvin_editor_profiler::ProfilerPlugin),
        Box::new(olvin_editor_animator::AnimatorPlugin),
        Box::new(olvin_editor_csg::CsgPlugin),
        Box::new(olvin_editor_modeling::ModelingPlugin),
        Box::new(olvin_editor_uv::UvPlugin),
        Box::new(olvin_editor_ide::IdePlugin),
        Box::new(olvin_editor_library::LibraryPlugin),
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
        // Test projects live in temp folders nobody trusted: `OLVIN_TRUST=1`
        // keeps the "Trust this project?" modal from covering the UI.
        unsafe { std::env::set_var("OLVIN_TRUST", "1") };
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

/// A drag in 1 px steps: the drag starts right at `from`, over the handle.
fn drag_slow(h: &mut Ed, from: egui::Pos2, dx: f32) {
    mouse_move(h, from);
    h.run_steps(2);
    mouse_button(h, from, true);
    h.run_steps(2);
    for i in 1..=dx as i32 {
        mouse_move(h, from + egui::vec2(i as f32, 0.0));
        h.run_steps(1);
    }
    mouse_button(h, from + egui::vec2(dx, 0.0), false);
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

// Fixed geometry of the default Layout workspace at `SIZE` (dock tabs are
// painted, not exposed to accessibility).
/// Middle of the scene view's area.
const SCENE_VIEW: egui::Pos2 = egui::pos2(761.0, 640.0);
/// The right end of the Scene/Game tab bar: it used to hold a "close all" ×
/// (now the "+" is there).
const GROUP_CLOSE: egui::Pos2 = egui::pos2(1232.0, 59.0);
/// The × inside the Game tab.
const GAME_TAB_CLOSE: egui::Pos2 = egui::pos2(457.6, 60.0);
const GAME_TAB: egui::Pos2 = egui::pos2(410.0, 60.0);
/// The first tile of the Assets panel.
const FIRST_TILE: egui::Pos2 = egui::pos2(327.0, 1293.0);

fn click_at(h: &mut Ed, at: egui::Pos2) {
    mouse_move(h, at);
    h.run_steps(2);
    mouse_button(h, at, true);
    h.run_steps(2);
    mouse_button(h, at, false);
    h.run_steps(3);
}

fn set_modifiers(h: &Ed, modifiers: egui::Modifiers) {
    h.event(egui::Event::ModifiersChanged(modifiers));
}

#[test]
fn alt_drag_orbits_even_over_a_gizmo_handle() {
    let p = project("alt-orbit");
    let mut h = editor(&p);
    state(&mut h).select(Some(3));
    h.run_steps(5);
    // Find a spot on the Move gizmo: a plain drag there moves the object.
    let mut spot = None;
    'scan: for dy in (-150..=150).step_by(10) {
        for dx in (-150..=150).step_by(10) {
            let at = SCENE_VIEW + egui::vec2(dx as f32, dy as f32);
            drag_slow(&mut h, at, 40.0);
            let moved = state(&mut h).scene.get(3).unwrap().position != Vec3::Y;
            state(&mut h).scene.get_mut(3).unwrap().position = Vec3::Y;
            h.run_steps(3);
            if moved {
                spot = Some(at);
                break 'scan;
            }
        }
    }
    let at = spot.expect("a gizmo handle near the middle of the scene view");
    set_modifiers(&h, egui::Modifiers::ALT);
    h.run_steps(2);
    drag_slow(&mut h, at, 40.0);
    set_modifiers(&h, egui::Modifiers::NONE);
    h.run_steps(3);
    assert_eq!(
        state(&mut h).scene.get(3).unwrap().position,
        Vec3::Y,
        "Alt+LMB orbits, it must not drag the handle"
    );
}

#[test]
fn frame_with_nothing_selected_does_not_jump_on_the_next_selection() {
    let p = project("frame-none");
    let mut h = editor(&p);
    state(&mut h).select(None);
    h.run_steps(3);
    h.hover_at(SCENE_VIEW);
    h.run_steps(2);
    h.key_press(egui::Key::F);
    h.run_steps(3);
    assert!(
        !state(&mut h).frame_selection,
        "F with no selection is dropped, not remembered"
    );
}

/// Opens File > Save As….
fn open_save_as(h: &mut Ed) {
    h.get_by_label("FILE").click();
    h.run_steps(3);
    h.get_by_label_contains("Save As").click();
    h.run_steps(3);
}

/// Types `path` into the open Save As dialog and presses Save.
fn save_as_dialog(h: &mut Ed, path: &str) {
    let current = h
        .get_all_by_role(egui::accesskit::Role::TextInput)
        .filter_map(|n| n.accesskit_node().value())
        .find(|v| v.ends_with(".scene.ron"))
        .expect("the path field");
    field(h, &current).focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    field(h, &current).type_text(path);
    h.run_steps(2);
    h.get_by_label("Save").click();
    h.run_steps(3);
}

#[test]
fn save_as_asks_before_replacing_a_scene() {
    let p = project("save-as");
    let mut h = editor(&p);
    let other = p.join("assets/scenes/other.scene.ron");
    let mut scene = SceneFile::default();
    scene.entities.push(cube(7, "Other", Vec3::ZERO));
    scene.save(&other).unwrap();
    let before = fs::read_to_string(&other).unwrap();

    open_save_as(&mut h);
    save_as_dialog(&mut h, "scenes/other.scene.ron");
    assert!(h.query_by_label("Replace Scene?").is_some());
    assert_eq!(fs::read_to_string(&other).unwrap(), before, "not yet");
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert_eq!(fs::read_to_string(&other).unwrap(), before);
    assert!(h.query_by_label("Replace Scene?").is_none());
    assert!(
        h.query_by_label("Save Scene As").is_some(),
        "back to the path"
    );

    // A new name needs no question.
    save_as_dialog(&mut h, "scenes/fresh.scene.ron");
    assert!(p.join("assets/scenes/fresh.scene.ron").exists());
    assert!(h.query_by_label("Replace Scene?").is_none());

    // Confirmed: replaced.
    open_save_as(&mut h);
    save_as_dialog(&mut h, "scenes/other.scene.ron");
    h.get_by_label("Replace").click();
    h.run_steps(3);
    assert_ne!(fs::read_to_string(&other).unwrap(), before);
    assert!(h.query_by_label("Replace Scene?").is_none());
}

#[test]
fn delete_and_duplicate_belong_to_the_game_view_while_playing() {
    let p = project("play-keys");
    let mut h = editor(&p);
    click_at(&mut h, GAME_TAB);
    state(&mut h).select(Some(3));
    state(&mut h).start_play();
    h.run_steps(5);
    h.hover_at(SCENE_VIEW);
    h.run_steps(3);
    h.key_press(egui::Key::Delete);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::D);
    h.run_steps(3);
    assert_eq!(names(&mut h).len(), 3, "{:?}", names(&mut h));
    // Back in the editor the same keys work again.
    state(&mut h).stop_play();
    state(&mut h).select(Some(3));
    h.run_steps(3);
    h.hover_at(SCENE_VIEW);
    h.run_steps(3);
    h.key_press(egui::Key::Delete);
    h.run_steps(3);
    assert_eq!(names(&mut h).len(), 2);
}

/// Is a widget's top-left corner at `at` (a dock tab's, say)?
fn node_at(h: &Ed, at: egui::Pos2) -> bool {
    h.root()
        .children_recursive()
        .any(|n| n.accesskit_node().bounding_box().is_some() && (n.rect().min - at).length() < 2.0)
}

#[test]
fn closing_a_tab_does_not_take_its_group_along() {
    let p = project("tab-close");
    let mut h = editor(&p);
    let (scene_tab, game_tab) = (egui::pos2(279.0, 45.0), egui::pos2(376.6, 45.0));
    assert!(node_at(&h, scene_tab) && node_at(&h, game_tab));
    click_at(&mut h, GROUP_CLOSE);
    assert!(
        node_at(&h, scene_tab) && node_at(&h, game_tab),
        "no group ×"
    );
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    // Each tab closes through its own × (shown on the front tab).
    click_at(&mut h, GAME_TAB);
    click_at(&mut h, GAME_TAB_CLOSE);
    assert!(!node_at(&h, game_tab), "its own × closes it");
    assert!(node_at(&h, scene_tab), "the neighbour stays");
}

const ROBOT: &str = include_str!("../../vertical_slice/assets/models/robot.gltf");

/// The tile `n` of the Assets panel's first row.
fn tile_at(n: usize) -> egui::Pos2 {
    FIRST_TILE + egui::vec2(92.0 * n as f32, 0.0)
}

/// `project` plus assets: folder `a_dest` (tile 0), `scenes` (1), then the
/// files `b.mat.ron` (2), `robot.gltf` (3) and `z.scene.ron` (4).
fn assets_project(name: &str) -> PathBuf {
    let p = project(name);
    let assets = p.join("assets");
    fs::create_dir_all(assets.join("a_dest")).unwrap();
    olvin_editor::olvin_scene::material::MaterialDesc::default()
        .save(assets.join("b.mat.ron"))
        .unwrap();
    fs::write(assets.join("robot.gltf"), ROBOT).unwrap();
    let mut other = SceneFile::default();
    other.entities.push(cube(7, "Other", Vec3::ZERO));
    other.save(assets.join("z.scene.ron")).unwrap();
    p
}

#[test]
fn assets_drag_into_a_folder_rename_and_delete() {
    let p = assets_project("assets-files");
    let assets = p.join("assets");
    let mut h = editor(&p);
    // Drag b.mat.ron onto the folder: moved with its .meta.
    drag(&mut h, tile_at(2), tile_at(0));
    assert!(assets.join("a_dest/b.mat.ron").exists());
    assert!(assets.join("a_dest/b.mat.ron.meta").exists());
    assert!(!assets.join("b.mat.ron").exists());
    // The rest shifts left: robot.gltf is tile 2 now. F2 renames it.
    click_at(&mut h, tile_at(2));
    h.key_press(egui::Key::F2);
    h.run_steps(3);
    field(&h, "robot.gltf").focus();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    field(&h, "robot.gltf").type_text("bot.gltf");
    h.key_press(egui::Key::Enter);
    h.run_steps(5);
    assert!(assets.join("bot.gltf").exists());
    assert!(!assets.join("robot.gltf").exists());
    // Delete asks first and leaves scene objects alone.
    state(&mut h).select(Some(3));
    click_at(&mut h, tile_at(2));
    h.key_press(egui::Key::Delete);
    h.run_steps(3);
    assert!(h.query_by_label("Delete Asset?").is_some());
    assert!(assets.join("bot.gltf").exists(), "not before the answer");
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(assets.join("bot.gltf").exists());
    h.hover_at(tile_at(2));
    h.run_steps(2);
    h.key_press(egui::Key::Delete);
    h.run_steps(3);
    h.get_by_label("Delete").click();
    h.run_steps(3);
    assert!(!assets.join("bot.gltf").exists());
    assert!(!assets.join("bot.gltf.meta").exists(), "the .meta goes too");
    assert_eq!(
        names(&mut h).len(),
        3,
        "Del over Assets is not Delete Object"
    );
}

#[test]
fn assets_drop_on_scene_view_and_hierarchy_and_open_by_double_click() {
    let p = assets_project("assets-drop");
    let mut h = editor(&p);
    // A material onto an object in the scene view.
    drag(&mut h, tile_at(2), SCENE_VIEW);
    let assigned = |h: &mut Ed| {
        state(h)
            .scene
            .entities
            .iter()
            .any(|e| e.material.as_deref() == Some("b.mat.ron"))
    };
    assert!(assigned(&mut h), "dropped on the middle of the scene");
    // A model onto the scene view adds objects.
    let before = names(&mut h).len();
    drag(&mut h, tile_at(3), SCENE_VIEW + egui::vec2(300.0, 200.0));
    assert!(names(&mut h).len() > before, "model spawned");
    // A model onto a hierarchy row becomes its child.
    let before = names(&mut h).len();
    let cube_row = row_center(&h, "◼ Cube");
    drag(&mut h, tile_at(3), cube_row);
    assert!(names(&mut h).len() > before);
    assert!(
        state(&mut h)
            .scene
            .entities
            .iter()
            .any(|e| e.parent == Some(3)),
        "child of Cube"
    );
    // Double-click opens a scene.
    click_at(&mut h, tile_at(4));
    mouse_button(&h, tile_at(4), true);
    h.run_steps(1);
    mouse_button(&h, tile_at(4), false);
    h.run_steps(3);
    if h.query_by_label("Unsaved Changes").is_some() {
        h.get_by_label("Don't Save").click();
        h.run_steps(3);
    }
    assert_eq!(names(&mut h), vec!["Other"]);
}

#[test]
fn material_edits_from_the_inspector_are_undoable() {
    use olvin_editor::olvin_scene::material::MaterialDesc;
    let p = project("mat-undo");
    let mut h = editor(&p);
    state(&mut h).select(Some(3));
    h.run_steps(5);
    h.get_by_label("New").click();
    h.run_steps(3);
    let rel = state(&mut h)
        .scene
        .get(3)
        .unwrap()
        .material
        .clone()
        .expect("a material was created and assigned");
    let metallic = |p: &Path| {
        MaterialDesc::load(p.join("assets").join(&rel))
            .unwrap()
            .metallic
    };
    assert_eq!(metallic(&p), 0.0);
    h.get_by_label("Metal").click();
    h.run_steps(3);
    assert_eq!(metallic(&p), 1.0, "edits are written as they are made");
    undo(&mut h);
    assert_eq!(metallic(&p), 0.0, "undo rewrites the material file");
    redo(&mut h);
    assert_eq!(metallic(&p), 1.0);
}

fn brush(id: u64, name: &str, pos: Vec3, scale: Vec3, op: CsgOp) -> EntityDesc {
    let mut e = EntityDesc::new(id, name);
    e.position = pos;
    e.scale = scale;
    e.brush = Some(BrushDesc {
        op,
        ..Default::default()
    });
    e
}

#[test]
fn csg_odd_and_many_brushes_do_not_break_the_editor() {
    let p = project("csg-odd");
    let mut h = editor(&p);
    assert!(h.state_mut().show_workspace("Mapping"));
    {
        let s = state(&mut h);
        s.scene.entities.clear();
        let odd = [
            Vec3::ZERO,
            Vec3::new(-2.0, 2.0, 2.0),
            Vec3::new(1e-9, 1.0, 1.0),
            Vec3::new(f32::NAN, 1.0, 1.0),
            Vec3::splat(1e9),
            Vec3::new(-1.0, -1.0, -1.0),
        ];
        let mut id = 1;
        for (i, scale) in odd.into_iter().enumerate() {
            for op in [CsgOp::Add, CsgOp::Subtract, CsgOp::Intersect] {
                s.scene
                    .entities
                    .push(brush(id, "odd", Vec3::new(i as f32, 0.5, 0.0), scale, op));
                id += 1;
            }
        }
        // And a pile of ordinary ones.
        for i in 0..40 {
            let at = Vec3::new((i % 15) as f32 * 1.5, 0.5, (i / 15) as f32 * 1.5);
            s.scene
                .entities
                .push(brush(id, "many", at, Vec3::splat(2.0), CsgOp::Add));
            id += 1;
        }
        s.select(Some(1));
    }
    h.run_steps(10);
    for label in [
        "Hollow",
        "Rotate 90°",
        "Flip X",
        "Flip Z",
        "▲ Earlier",
        "▼ Later",
    ] {
        h.get_by_label(label).click();
        h.run_steps(3);
    }
    h.get_by_label("Duplicate as Carve").click();
    h.run_steps(3);
    // Zero-scale brush: Hollow must not produce NaN/infinite scales.
    state(&mut h).select(Some(1));
    h.run_steps(3);
    h.get_by_label("Hollow").click();
    h.run_steps(3);
    for e in &state(&mut h).scene.entities {
        assert!(
            e.name == "odd" || e.scale.is_finite(),
            "{} has scale {:?}",
            e.name,
            e.scale
        );
    }
    for _ in 0..4 {
        undo(&mut h);
    }
    h.run_steps(5);
}

/// Draws a brush with the Draw Brush tool: drag a base, then click for height.
fn draw_brush(h: &mut Ed) {
    drag(h, egui::pos2(700.0, 600.0), egui::pos2(860.0, 700.0));
    for i in 1..=5 {
        mouse_move(h, egui::pos2(860.0, 700.0 - 20.0 * i as f32));
        h.run_steps(1);
    }
    click_at(h, egui::pos2(860.0, 600.0));
}

#[test]
fn csg_draw_tool_builds_a_brush_on_the_ground_and_alt_orbits_instead() {
    let p = project("csg-draw");
    let mut h = editor(&p);
    assert!(h.state_mut().show_workspace("Mapping"));
    state(&mut h).scene.entities.clear();
    state(&mut h).tool = "csg.draw".into();
    h.run_steps(5);
    // Alt+LMB is the camera: no brush appears.
    set_modifiers(&h, egui::Modifiers::ALT);
    h.run_steps(2);
    draw_brush(&mut h);
    set_modifiers(&h, egui::Modifiers::NONE);
    h.run_steps(3);
    assert!(
        state(&mut h).scene.entities.is_empty(),
        "Alt must not draw: {:?}",
        names(&mut h)
    );
    draw_brush(&mut h);
    let brushes: Vec<_> = state(&mut h)
        .scene
        .entities
        .iter()
        .filter(|e| e.brush.is_some())
        .map(|e| e.scale)
        .collect();
    assert_eq!(brushes.len(), 1, "one brush drawn on the empty ground");
    assert!(brushes[0].is_finite() && brushes[0].min_element() > 0.0);
    undo(&mut h);
    assert!(state(&mut h).scene.entities.is_empty(), "undo removes it");
    redo(&mut h);
    assert_eq!(names(&mut h).len(), 1);
}

/// The scene's prefab node `local` of the instance at `root`.
fn node(root: u64, local: u64) -> u64 {
    olvin_editor::olvin_scene::prefab::member_id(root, local)
}

fn asset_text(p: &Path, rel: &str) -> String {
    fs::read_to_string(p.join("assets").join(rel)).unwrap()
}

#[test]
fn create_prefab_from_the_selection_shows_it_in_hierarchy_and_inspector() {
    let p = project("prefab-create");
    let mut h = editor(&p);
    state(&mut h).select(Some(1));
    h.run_steps(3);
    click_menu(&mut h, "GAMEOBJECT", "Create Prefab");
    let asset = p.join("assets/prefabs/Root.prefab.ron");
    assert!(asset.exists(), "the subtree was saved as an asset");
    let scene = SceneFile::load(&asset).unwrap();
    assert_eq!(scene.entities.len(), 2, "Root and its Child");
    // The object is an instance now: its child is a node of it, the blue ◈ row shows it.
    let child = node(1, 2);
    assert!(state(&mut h).scene.get(child).is_some());
    assert!(state(&mut h).scene.get(1).unwrap().prefab.is_some());
    assert!(h.query_by_label("◆ Root").is_some(), "instance row");
    // The inspector has the Prefab block with its buttons.
    assert!(h.query_by_label("PREFAB").is_some());
    for button in ["Open", "Select", "Revert All", "Apply All", "Unpack"] {
        assert!(h.query_by_label(button).is_some(), "{button}");
    }
    // Undo turns it back into plain objects (the asset stays).
    undo(&mut h);
    assert!(state(&mut h).scene.get(1).unwrap().prefab.is_none());
    assert!(h.query_by_label("◼ Root").is_some());
    redo(&mut h);
    assert!(state(&mut h).scene.get(1).unwrap().prefab.is_some());
    // The right-click menu of a plain object has Create Prefab too.
    h.get_by_label("◼ Cube").click_secondary();
    h.run_steps(3);
    h.get_by_label("Create Prefab").click();
    h.run_steps(3);
    assert!(p.join("assets/prefabs/Cube.prefab.ron").exists());
}

#[test]
fn overridden_fields_are_marked_and_revert_one_by_one_or_all() {
    let p = project("prefab-override");
    let mut h = editor(&p);
    state(&mut h).select(Some(1));
    h.run_steps(3);
    click_menu(&mut h, "GAMEOBJECT", "Create Prefab");
    let child = node(1, 2);
    state(&mut h).select(Some(child));
    h.run_steps(5);
    assert!(h.query_by_label("↺").is_none(), "nothing overridden yet");
    assert!(
        h.query_by_label("Part of a prefab instance; edits here are overrides.")
            .is_some()
    );
    // An edit made like any other (a gizmo drag or the inspector would do the same).
    state(&mut h).scene.get_mut(child).unwrap().position.x = 7.0;
    h.run_steps(5);
    let marked = h.get_all_by_label("↺").count();
    assert!(
        marked >= 1,
        "the Position row and the list show a revert button"
    );
    assert!(h.query_by_label("position").is_some(), "listed by field");
    assert_eq!(
        state(&mut h).prefab_report(child).unwrap().overridden,
        ["position"]
    );
    // The instance root shows the total.
    state(&mut h).select(Some(1));
    h.run_steps(5);
    assert!(h.query_by_label("1 override").is_some());
    // Revert one field from the child.
    state(&mut h).select(Some(child));
    h.run_steps(5);
    h.get_all_by_label("↺").next().unwrap().click();
    h.run_steps(5);
    assert_eq!(state(&mut h).scene.get(child).unwrap().position.x, 1.0);
    assert!(h.query_by_label("↺").is_none());
    // Revert All on the root.
    state(&mut h).scene.get_mut(child).unwrap().position.x = 7.0;
    state(&mut h).scene.get_mut(child).unwrap().scale.y = 3.0;
    state(&mut h).select(Some(1));
    h.run_steps(5);
    assert!(h.query_by_label("2 overrides").is_some());
    h.get_by_label("Revert All").click();
    h.run_steps(5);
    assert_eq!(state(&mut h).scene.get(child).unwrap().position.x, 1.0);
    assert_eq!(state(&mut h).scene.get(child).unwrap().scale.y, 1.0);
    assert!(h.query_by_label("No overrides").is_some());
    // Saving writes the overrides, not the nodes.
    state(&mut h).scene.get_mut(child).unwrap().position.x = 2.5;
    assert!(state(&mut h).save());
    let saved = SceneFile::load(p.join("assets/scenes/main.scene.ron")).unwrap();
    assert_eq!(saved.entities.len(), 2, "Root (the instance) and Cube");
    let inst = saved.get(1).unwrap().prefab.as_ref().unwrap();
    assert!(inst.overrides["2"]["position"].contains("2.5"), "{inst:?}");
}

#[test]
fn apply_all_updates_the_prefab_and_every_instance_and_undo_takes_it_back() {
    let p = project("prefab-apply");
    let mut h = editor(&p);
    state(&mut h).select(Some(1));
    h.run_steps(3);
    click_menu(&mut h, "GAMEOBJECT", "Create Prefab");
    let rel = "prefabs/Root.prefab.ron";
    let before = asset_text(&p, rel);
    // A second instance, from the asset.
    let second = state(&mut h)
        .instantiate_prefab(rel, None, Vec3::new(0.0, 0.0, 9.0))
        .unwrap();
    h.run_steps(3);
    let (child, child2) = (node(1, 2), node(second, 2));
    state(&mut h).scene.get_mut(child).unwrap().scale = Vec3::new(2.0, 2.0, 2.0);
    h.run_steps(3);
    state(&mut h).select(Some(1));
    h.run_steps(5);
    h.get_by_label("Apply All").click();
    h.run_steps(5);
    assert_ne!(asset_text(&p, rel), before, "written into the asset");
    assert_eq!(
        state(&mut h).scene.get(child2).unwrap().scale,
        Vec3::splat(2.0),
        "the other instance follows"
    );
    assert!(h.query_by_label("No overrides").is_some());
    undo(&mut h);
    assert_eq!(asset_text(&p, rel), before, "undo rewrites the prefab file");
    assert_eq!(state(&mut h).scene.get(child2).unwrap().scale, Vec3::ONE);
    assert_eq!(
        state(&mut h).scene.get(child).unwrap().scale,
        Vec3::splat(2.0)
    );
    redo(&mut h);
    assert_ne!(asset_text(&p, rel), before);
    assert_eq!(
        state(&mut h).scene.get(child2).unwrap().scale,
        Vec3::splat(2.0)
    );
}

#[test]
fn dragging_a_prefab_from_assets_instantiates_it() {
    let p = project("prefab-drag");
    // A prefab file in the Assets root: tile 0 is the scenes folder, tile 1 the prefab.
    let mut crate_ = cube(1, "Crate", Vec3::ZERO);
    crate_.scale = Vec3::splat(1.0);
    let mut lid = cube(2, "Lid", Vec3::Y);
    lid.parent = Some(1);
    SceneFile {
        entities: vec![crate_, lid],
        ..Default::default()
    }
    .save(p.join("assets/crate.prefab.ron"))
    .unwrap();
    let mut h = editor(&p);
    let before = names(&mut h).len();
    drag(&mut h, tile_at(1), SCENE_VIEW + egui::vec2(200.0, 150.0));
    assert_eq!(names(&mut h).len(), before + 2, "root and lid");
    let placed = state(&mut h).selection.unwrap();
    let e = state(&mut h).scene.get(placed).unwrap().clone();
    assert_eq!(e.name, "crate");
    assert!(
        e.prefab
            .as_ref()
            .is_some_and(|p| p.asset == "crate.prefab.ron")
    );
    assert!(
        e.position.x != 0.0 || e.position.z != 0.0,
        "dropped where the pointer was"
    );
    // On a hierarchy row it becomes a child of that object.
    let before = names(&mut h).len();
    let cube_row = row_center(&h, "◼ Cube");
    drag(&mut h, tile_at(1), cube_row);
    assert_eq!(names(&mut h).len(), before + 2);
    let dropped = state(&mut h).selection.unwrap();
    assert_eq!(state(&mut h).scene.get(dropped).unwrap().parent, Some(3));
    // Undo takes the last one away again.
    undo(&mut h);
    assert_eq!(names(&mut h).len(), before);
    // Double-clicking the prefab opens it as a scene.
    click_at(&mut h, tile_at(1));
    mouse_button(&h, tile_at(1), true);
    h.run_steps(1);
    mouse_button(&h, tile_at(1), false);
    h.run_steps(3);
    if h.query_by_label("Unsaved Changes").is_some() {
        h.get_by_label("Don't Save").click();
        h.run_steps(3);
    }
    assert_eq!(names(&mut h), vec!["Crate", "Lid"]);
}

/// Clicks a toolbar button, or a menu title and then its item.
fn click_menu(h: &mut Ed, title: &str, item: &str) {
    h.get_by_label(title).click();
    h.run_steps(3);
    h.get_by_label(item).click();
    h.run_steps(3);
}

#[test]
fn uv_editor_operations_selection_and_modal_transforms() {
    let p = project("uv");
    let mut h = editor(&p);
    // No mesh yet: the editor opens and every operator is a no-op.
    assert!(h.state_mut().show_workspace("UV Editing"));
    state(&mut h).select(None);
    h.run_steps(5);
    h.get_by_label("Unwrap  U").click();
    h.run_steps(3);
    // A cube made editable in the Modeling workspace.
    assert!(h.state_mut().show_workspace("Modeling"));
    state(&mut h).select(Some(3));
    h.run_steps(3);
    h.get_by_label("◆ Edit Mode").click();
    h.run_steps(3);
    assert!(h.state_mut().show_workspace("UV Editing"));
    h.run_steps(5);
    // Empty selection: operators map all faces; selection ops work on nothing.
    for label in ["Face 3", "Island 4", "Edge 2", "Vertex 1"] {
        h.get_by_label_contains(label).click();
        h.run_steps(2);
    }
    click_menu(&mut h, "Select", "None  Alt+A");
    click_menu(&mut h, "Select", "Linked  Ctrl+L");
    click_menu(&mut h, "Select", "Invert  Ctrl+I");
    h.get_by_label("Unwrap  U").click();
    h.run_steps(3);
    for item in [
        "Box Projection",
        "Planar Projection",
        "Cylinder Projection",
        "Sphere Projection",
        "Follow Active Quads",
        "Reset",
    ] {
        click_menu(&mut h, "Project", item);
    }
    for item in ["Mark Seam", "Clear Seam", "Seams from Islands"] {
        click_menu(&mut h, "Seams", item);
    }
    h.get_by_label_contains("Pack").click();
    h.run_steps(3);
    for item in [
        "Flip U",
        "Flip V",
        "Rotate 90° ⟲",
        "Rotate 90° ⟳",
        "Align U",
        "Align V",
    ] {
        click_menu(&mut h, "Arrange", item);
    }
    click_menu(&mut h, "Select", "All  A");
    // Modal G / R / S over the canvas, with a numeric value, cancel and confirm.
    let canvas = egui::pos2(330.0, 700.0);
    for key in [egui::Key::G, egui::Key::R, egui::Key::S] {
        h.hover_at(canvas);
        h.run_steps(2);
        h.key_press(key);
        h.run_steps(2);
        mouse_move(&h, canvas + egui::vec2(40.0, 10.0));
        h.run_steps(2);
        h.key_press(egui::Key::Escape);
        h.run_steps(3);
        h.key_press(key);
        h.run_steps(2);
        mouse_move(&h, canvas + egui::vec2(30.0, 30.0));
        h.run_steps(2);
        h.event(egui::Event::Text("0".into()));
        h.event(egui::Event::Text("0".into()));
        h.run_steps(2);
        click_at(&mut h, canvas + egui::vec2(30.0, 30.0));
    }
    // Everything above must have left a consistent mesh that survives undo.
    for _ in 0..40 {
        undo(&mut h);
    }
    h.run_steps(5);
    let mesh = state(&mut h).scene.get(3).unwrap().poly.clone();
    if let Some(m) = mesh {
        assert!(m.positions.iter().all(|p| p.is_finite()));
        assert!(m.uvs.iter().flatten().all(|u| u.is_finite()), "finite UVs");
    }
}

#[test]
fn uv_editor_copes_with_an_empty_mesh() {
    let p = project("uv-empty");
    let mut h = editor(&p);
    state(&mut h).scene.get_mut(3).unwrap().poly = Some(Default::default());
    state(&mut h).select(Some(3));
    assert!(h.state_mut().show_workspace("UV Editing"));
    h.run_steps(5);
    h.get_by_label("Unwrap  U").click();
    h.run_steps(3);
    click_menu(&mut h, "Select", "All  A");
    h.hover_at(egui::pos2(330.0, 700.0));
    h.run_steps(2);
    for key in [egui::Key::G, egui::Key::A, egui::Key::U] {
        h.key_press(key);
        h.run_steps(2);
    }
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
}

#[test]
fn console_filters_clears_copies_and_keeps_rows_single_line() {
    let p = project("console");
    let mut h = editor(&p);
    h.get_by_label("WINDOW").click();
    h.run_steps(3);
    h.get_by_label_contains("Add Panel").click();
    h.run_steps(3);
    h.get_by_label("🖵 Console").click();
    h.run_steps(5);
    state(&mut h).log.clear();
    {
        let s = state(&mut h);
        s.info("plain info");
        s.warn("a warning");
        s.error("Ошибка: файл не найден 🦀");
        s.info("long ".repeat(400));
        s.info("two\nlines\nhere");
        for i in 0..3000 {
            s.info(format!("spam {i}"));
        }
    }
    h.run_steps(5);
    // Level toggles and the text filter.
    h.get_by_label_contains("⚠").click();
    h.run_steps(3);
    h.get_by_label_contains("⛔").click();
    h.run_steps(3);
    h.get_by_label_contains("⚠").click();
    h.get_by_label_contains("⛔").click();
    h.run_steps(3);
    // The console's filter: the wide field in the bottom leaf (the tab opens
    // there, where the Assets panel is).
    h.get_all_by_role(egui::accesskit::Role::TextInput)
        .find(|n| n.rect().width() > 400.0)
        .expect("console filter")
        .focus();
    h.run_steps(2);
    h.get_all_by_role(egui::accesskit::Role::TextInput)
        .find(|n| n.rect().width() > 400.0)
        .expect("console filter")
        .type_text("ошибка");
    h.run_steps(3);
    // Copy puts the shown lines on the clipboard.
    h.get_by_label("Copy").click();
    let mut copied = None;
    for _ in 0..4 {
        h.step();
        copied = copied.or_else(|| {
            h.output()
                .platform_output
                .commands
                .iter()
                .find_map(|c| match c {
                    egui::OutputCommand::CopyText(t) => Some(t.clone()),
                    _ => None,
                })
        });
    }
    let text = copied.expect("Copy sends text");
    assert!(text.contains("Ошибка: файл не найден 🦀"), "{text}");
    assert!(
        !text.contains("plain info"),
        "filtered out: {}",
        text.chars().take(200).collect::<String>()
    );
    h.get_by_label("Clear").click();
    h.run_steps(3);
    assert!(state(&mut h).log.is_empty() || state(&mut h).log.len() < 5);
    // The log is bounded.
    for i in 0..6000 {
        state(&mut h).info(format!("line {i}"));
    }
    assert!(state(&mut h).log.len() <= 5000);
    h.run_steps(3);
}

/// A spot on the selected object's gizmo for the active tool: a slow drag of
/// 40 px there changes `probe`. Restores the object afterwards.
fn find_handle(h: &mut Ed, probe: impl Fn(&mut Ed) -> Vec3, reset: impl Fn(&mut Ed)) -> egui::Pos2 {
    let start = probe(h);
    for dy in (-120..=120).step_by(12) {
        for dx in (-120..=120).step_by(12) {
            let at = SCENE_VIEW + egui::vec2(dx as f32, dy as f32);
            drag_slow(h, at, 40.0);
            let moved = probe(h) != start;
            reset(h);
            h.run_steps(3);
            if moved {
                return at;
            }
        }
    }
    panic!("no gizmo handle found");
}

#[test]
fn gizmo_tools_move_rotate_scale_in_both_spaces_and_undo() {
    let p = project("gizmo");
    let mut h = editor(&p);
    let reset = |h: &mut Ed| {
        let e = state(h).scene.get_mut(3).unwrap();
        e.position = Vec3::Y;
        e.rotation = Vec3::ZERO;
        e.scale = Vec3::ONE;
    };
    for (key, local) in [
        (egui::Key::W, false),
        (egui::Key::W, true),
        (egui::Key::E, false),
        (egui::Key::E, true),
        (egui::Key::R, false),
    ] {
        reset(&mut h);
        state(&mut h).select(Some(3));
        state(&mut h).local_space = local;
        h.run_steps(3);
        h.hover_at(SCENE_VIEW);
        h.run_steps(2);
        h.key_press(key);
        h.run_steps(3);
        let probe = |h: &mut Ed| {
            let e = state(h).scene.get(3).unwrap();
            e.position + e.rotation * 1000.0 + e.scale * 1e6
        };
        let at = find_handle(&mut h, probe, reset);
        drag_slow(&mut h, at, 60.0);
        let e = state(&mut h).scene.get(3).unwrap().clone();
        assert!(
            e.position.is_finite() && e.rotation.is_finite() && e.scale.is_finite(),
            "{key:?} local={local}: {e:?}"
        );
        let changed = e.position != Vec3::Y || e.rotation != Vec3::ZERO || e.scale != Vec3::ONE;
        assert!(changed, "{key:?} local={local} did nothing");
        // One drag, one undo step.
        undo(&mut h);
        let e = state(&mut h).scene.get(3).unwrap().clone();
        assert!(
            e.position == Vec3::Y && e.rotation == Vec3::ZERO && e.scale == Vec3::ONE,
            "{key:?} local={local}: undo left {e:?}"
        );
    }
}

#[test]
fn clicking_picks_objects_and_empty_space_clears() {
    let p = project("pick");
    let mut h = editor(&p);
    state(&mut h).select(None);
    h.run_steps(5);
    // Somewhere on the cubes near the middle of the view.
    let mut hit = None;
    'scan: for dy in (-200..=200).step_by(8) {
        for dx in (-200..=200).step_by(8) {
            let at = SCENE_VIEW + egui::vec2(dx as f32, dy as f32);
            click_at(&mut h, at);
            if state(&mut h).selection.is_some() {
                hit = Some(at);
                break 'scan;
            }
        }
    }
    let at = hit.expect("a click selects one of the cubes");
    // Ctrl+click toggles it off the multi-selection, plain click on the sky clears.
    click_at(&mut h, SCENE_VIEW + egui::vec2(-400.0, -450.0));
    assert!(state(&mut h).selection.is_none(), "sky click deselects");
    // Alt+click is the camera, never a selection.
    set_modifiers(&h, egui::Modifiers::ALT);
    h.run_steps(2);
    click_at(&mut h, at);
    set_modifiers(&h, egui::Modifiers::NONE);
    h.run_steps(2);
    assert!(state(&mut h).selection.is_none(), "Alt+click must not pick");
    // Picking still works after the view moved (wheel zoom, middle pan).
    h.hover_at(SCENE_VIEW);
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, 120.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(5);
    click_at(&mut h, SCENE_VIEW);
    h.run_steps(2);
}

#[test]
fn camera_navigation_keys_and_view_gizmo_are_stable() {
    let p = project("camera");
    let mut h = editor(&p);
    h.hover_at(SCENE_VIEW);
    h.run_steps(3);
    // Orbit (Alt+LMB), zoom (Alt+RMB), fly (RMB + WASD), frame, shading toggle.
    set_modifiers(&h, egui::Modifiers::ALT);
    h.run_steps(2);
    drag(&mut h, SCENE_VIEW, SCENE_VIEW + egui::vec2(120.0, -80.0));
    set_modifiers(&h, egui::Modifiers::NONE);
    h.run_steps(2);
    h.event(egui::Event::PointerButton {
        pos: SCENE_VIEW,
        button: egui::PointerButton::Secondary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(2);
    for key in [
        egui::Key::W,
        egui::Key::A,
        egui::Key::S,
        egui::Key::D,
        egui::Key::Q,
        egui::Key::E,
    ] {
        h.key_down(key);
        h.run_steps(3);
        h.key_up(key);
    }
    h.event(egui::Event::PointerButton {
        pos: SCENE_VIEW,
        button: egui::PointerButton::Secondary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(3);
    state(&mut h).select(Some(3));
    h.hover_at(SCENE_VIEW);
    h.run_steps(2);
    h.key_press(egui::Key::F);
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Z);
    h.run_steps(3);
    // View gizmo: its centre toggles Persp/Iso, the axis tips look along an axis.
    let gizmo = egui::pos2(1185.0, 138.0);
    for dx in [0.0, 22.0, -22.0, 0.0] {
        click_at(&mut h, gizmo + egui::vec2(dx, 0.0));
    }
    click_at(&mut h, gizmo);
    click_at(&mut h, gizmo);
    h.run_steps(5);
    // Still alive: the object can be picked and moved.
    assert!(state(&mut h).scene.get(3).unwrap().position.is_finite());
}

#[test]
fn profiler_panel_opens_and_shows_frame_time_counters_and_scopes() {
    let p = project("profiler");
    let mut h = editor(&p);
    h.get_by_label("WINDOW").click();
    h.run_steps(3);
    h.get_by_label_contains("Add Panel").click();
    h.run_steps(3);
    h.get_by_label("⏱ Profiler").click();
    h.run_steps(5);
    {
        let stats = olvin_editor::olvin::render::RenderStats {
            draw_calls: 7,
            instances: 4242,
            culled: 31,
            triangles: 123_456,
        };
        state(&mut h).render_stats.insert(1, stats);
    }
    h.run_steps(10);
    // Frame time line and the four render counters.
    h.get_by_label_contains("fps");
    h.get_by_label("Draw calls");
    h.get_by_label("7");
    h.get_by_label("4 242");
    h.get_by_label("31");
    h.get_by_label("123 456");
    // The editor's own frame scope shows up in the scope table. Profiler
    // frames arrive on wall-clock time, so wait for it instead of counting
    // steps (a loaded machine made the fixed count flaky).
    for _ in 0..60 {
        if h.query_by_label("editor frame").is_some() {
            break;
        }
        h.run_steps(5);
    }
    h.get_by_label("editor frame");
    // Pausing keeps the numbers on screen.
    h.get_by_label("⏸ Pause").click();
    h.run_steps(3);
    h.get_by_label_contains("Resume");
}

/// The top bar (logo, menus, workspace tabs, play controls, gear) keeps its
/// parts apart at every window width, however many workspaces there are: the
/// tabs scroll in the room between the menus and the play controls.
#[test]
fn top_bar_parts_never_overlap_at_any_width() {
    // The "+" of the workspace tabs (docked panels have their own below it).
    fn top_plus<'a>(h: &'a Ed) -> egui_kittest::Node<'a> {
        h.get_all_by_label("+")
            .min_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
            .unwrap()
    }
    let p = project("topbar");
    let mut h = editor(&p);
    // Many tabs with long names: each copy appends " (copy)".
    for _ in 0..8 {
        top_plus(&h).click();
        h.run_steps(3);
        h.get_by_label("Duplicate Current").click();
        h.run_steps(3);
    }
    let last = format!("Layout{}", " (copy)".repeat(8));
    for width in [800.0_f32, 1024.0, 1280.0, 1600.0, 1920.0] {
        h.set_size(egui::vec2(width, 900.0));
        h.run_steps(12);
        let bar = h.state().top_bar_rects();
        let at = format!("at {width} px: {bar:?}");
        // Left to right, in order, all inside the window.
        assert!(bar.menus_end <= bar.tabs.left() + 0.5, "menus vs tabs {at}");
        assert!(bar.tabs.right() <= bar.play.left(), "tabs vs play {at}");
        assert!(bar.play.right() <= bar.gear.left(), "play vs gear {at}");
        assert!(bar.gear.right() <= width, "gear outside the window {at}");
        assert!(bar.tabs.width() >= 100.0, "tabs strip too narrow {at}");
        // The real widgets sit where the bar says they do.
        for label in ["▶", "⏸", "⏭"] {
            let r = h.get_by_label(label).rect();
            assert!(bar.play.contains_rect(r.shrink(0.5)), "{label} {r:?} {at}");
        }
        let gear = h.get_by_label("⚙").rect();
        assert!(
            bar.gear.contains_rect(gear.shrink(0.5)),
            "gear {gear:?} {at}"
        );
        let plus = top_plus(&h).rect();
        assert!(bar.tabs.contains_rect(plus.shrink(0.5)), "+ {plus:?} {at}");
        // The active tab was scrolled into view (these names are far wider
        // than a narrow strip, so only part of it has to show).
        let tab = h.get_by_label(&last).rect();
        assert!(
            tab.right() > bar.tabs.left() + 8.0 && tab.left() < plus.left() - 8.0,
            "active tab {tab:?} not visible {at}"
        );
    }
}

#[test]
fn picking_a_look_changes_the_environment_and_undo_restores_it() {
    use olvin_editor::olvin_scene::look::LookDesc;
    use olvin_editor::settings::Page;
    let p = project("look");
    LookDesc {
        name: "Mine".into(),
        exposure: 0.4,
        ..Default::default()
    }
    .save(p.join("assets/looks/mine.look.ron"))
    .unwrap();
    let mut h = editor(&p);
    let before = state(&mut h).scene.environment.clone();
    state(&mut h).open_settings(Page::Rendering);
    h.run_steps(10);

    // A built-in look.
    h.get_by_value("(pick a look)").click();
    h.run_steps(3);
    h.get_by_label("Cinematic (built-in)").click();
    h.run_steps(3);
    h.get_by_label("Apply").click();
    h.run_steps(3);
    let env = state(&mut h).scene.environment.clone();
    assert!(env.post.bloom.is_some() && env.post.contrast > 1.0);
    assert_eq!(env.sky, before.sky, "lighting stays");
    undo(&mut h);
    assert_eq!(state(&mut h).scene.environment, before, "undo restores it");
    redo(&mut h);
    assert!(state(&mut h).scene.environment.post.bloom.is_some());
    undo(&mut h);

    // A look asset of the project.
    h.get_by_value("Cinematic (built-in)").click();
    h.run_steps(3);
    h.get_by_label_contains("mine  (looks/mine.look.ron)")
        .click();
    h.run_steps(5);
    h.get_by_label("Apply").click();
    h.run_steps(5);
    assert_eq!(state(&mut h).scene.environment.exposure, 0.4);
    undo(&mut h);
    assert_eq!(state(&mut h).scene.environment.exposure, before.exposure);
}

#[test]
fn the_ps1_preset_switches_the_retro_style_on_and_undo_restores_it() {
    use olvin_editor::olvin_scene::file::RetroDesc;
    use olvin_editor::settings::Page;
    let p = project("retro");
    let mut h = editor(&p);
    assert_eq!(state(&mut h).scene.environment.retro, None);
    state(&mut h).open_settings(Page::Rendering);
    h.run_steps(10);
    // Below the fold of the settings page: scroll to it first.
    h.get_by_label("PS1 preset").scroll_to_me();
    h.run_steps(3);
    h.get_by_label("PS1 preset").click();
    h.run_steps(5);
    assert_eq!(
        state(&mut h).scene.environment.retro,
        Some(RetroDesc::ps1())
    );
    undo(&mut h);
    assert_eq!(
        state(&mut h).scene.environment.retro,
        None,
        "undo restores it"
    );
    redo(&mut h);
    h.run_steps(3);
    h.get_by_label("Off").click();
    h.run_steps(5);
    assert_eq!(state(&mut h).scene.environment.retro, None);
}

#[test]
fn particle_system_add_edit_undo_and_presets() {
    use olvin_editor::olvin_scene::file::ParticleDesc;
    let p = project("particles");
    let mut h = editor(&p);
    state(&mut h).select(Some(3));
    h.run_steps(3);
    assert!(state(&mut h).scene.get(3).unwrap().particles.is_none());

    // Add Component > Particle System.
    h.get_by_label("Add Component").click();
    h.run_steps(3);
    h.get_by_label("Particle System").click();
    h.run_steps(5);
    assert_eq!(
        state(&mut h).scene.get(3).unwrap().particles,
        Some(ParticleDesc::default())
    );

    // A parameter edit is a scene edit: it undoes.
    h.get_by_label("+ Add burst").click();
    h.run_steps(5);
    assert_eq!(
        state(&mut h)
            .scene
            .get(3)
            .unwrap()
            .particles
            .as_ref()
            .unwrap()
            .bursts
            .len(),
        1
    );
    undo(&mut h);
    assert!(
        state(&mut h)
            .scene
            .get(3)
            .unwrap()
            .particles
            .as_ref()
            .unwrap()
            .bursts
            .is_empty()
    );

    // The preset menu replaces the look.
    h.get_by_value("Apply…").click();
    h.run_steps(3);
    h.get_by_label("Fire").click();
    h.run_steps(5);
    let got = state(&mut h)
        .scene
        .get(3)
        .unwrap()
        .particles
        .clone()
        .unwrap();
    assert_eq!(got, ParticleDesc::fire());
    undo(&mut h);
    assert_eq!(
        state(&mut h).scene.get(3).unwrap().particles,
        Some(ParticleDesc::default())
    );
}

#[test]
fn particle_preview_simulates_while_selected_and_obeys_the_buttons() {
    use olvin_editor::olvin_scene::file::ParticleDesc;
    let p = project("particles-preview");
    let mut h = editor(&p);
    state(&mut h).scene.get_mut(3).unwrap().particles = Some(ParticleDesc {
        rate: 200.0,
        lifetime: [5.0, 5.0],
        ..Default::default()
    });
    state(&mut h).select(Some(3));
    h.run_steps(30);
    let n = state(&mut h).particle_preview_count(3).expect("previewed");
    assert!(n > 10, "{n}");
    // Pause freezes, Stop clears, Play resumes.
    h.get_by_label("⏸ Pause").click();
    h.run_steps(3);
    let frozen = state(&mut h).particle_preview_count(3).unwrap();
    h.run_steps(20);
    assert_eq!(state(&mut h).particle_preview_count(3), Some(frozen));
    h.get_by_label("⏹ Stop").click();
    h.run_steps(3);
    assert_eq!(state(&mut h).particle_preview_count(3), Some(0));
    h.get_by_label("▶ Play").click();
    h.run_steps(20);
    assert!(state(&mut h).particle_preview_count(3).unwrap() > 0);
    // Deselected: gone (no Simulate toggle).
    state(&mut h).select(Some(1));
    h.run_steps(3);
    assert_eq!(state(&mut h).particle_preview_count(3), None);
}

// --- Library ----------------------------------------------------------------

/// Window > Add Panel > Library.
fn open_library(h: &mut Ed) {
    h.get_by_label("WINDOW").click();
    h.run_steps(3);
    h.get_by_label_contains("Add Panel").click();
    h.run_steps(3);
    h.get_by_label("📦 Library").click();
    h.run_steps(6);
}

/// Types into the Library's search field (the panel is small: one card at a time).
fn library_search(h: &mut Ed, text: &str) {
    fn field(h: &Ed) -> egui_kittest::Node<'_> {
        h.get_all_by_role(egui::accesskit::Role::TextInput)
            .find(|n| n.rect().width() > 400.0)
            .expect("library search")
    }
    field(h).focus();
    h.run_steps(2);
    field(h).type_text(text);
    h.run_steps(4);
}

/// Clicks the `n`th "Import"/"Re-import" button (cards are sorted by
/// category and name: First Person, Third Person, Top Down).
fn click_import(h: &mut Ed, n: usize) {
    let mut buttons: Vec<_> = h
        .get_all_by_role(egui::accesskit::Role::Button)
        .filter(|b| {
            matches!(
                b.accesskit_node().label().as_deref(),
                Some("Import" | "Re-import")
            )
        })
        .collect();
    buttons.sort_by(|a, b| a.rect().min.y.total_cmp(&b.rect().min.y));
    buttons[n].click();
    h.run_steps(4);
}

#[test]
fn library_lists_packs_imports_and_marks_them_installed() {
    let p = project("library-import");
    let mut h = editor(&p);
    open_library(&mut h);
    assert!(h.query_by_label("First Person Controller").is_some());
    assert!(h.query_by_label("Third Person Controller").is_some());
    assert!(h.query_by_label("Top Down Controller").is_some());
    assert!(h.query_by_label_contains("Installed").is_none());

    library_search(&mut h, "first person");
    click_import(&mut h, 0);
    // The files and their GUIDs are in the project, the lock records the pack.
    let assets = p.join("assets");
    assert!(assets.join("scripts/fps-controller/player.lua").is_file());
    assert!(
        assets
            .join("scripts/fps-controller/player.lua.meta")
            .is_file()
    );
    assert!(
        assets
            .join("prefabs/fps-controller/Player.prefab.ron")
            .is_file()
    );
    assert!(p.join("library.lock.ron").is_file());
    // Badge, toast with the notes, console line, the Assets panel follows.
    assert!(h.query_by_label_contains("✔ Installed").is_some());
    assert!(
        h.query_all_by_label_contains("Imported fps-controller 1.0.0")
            .next()
            .is_some()
    );
    assert!(
        h.query_all_by_label_contains("press Play").next().is_some(),
        "the pack's notes"
    );
    assert!(
        state(&mut h)
            .log
            .iter()
            .any(|l| l.text.contains("Library: Imported fps-controller"))
    );
    // Only the imported pack is marked.
    assert_eq!(h.query_all_by_label_contains("✔ Installed").count(), 1);

    // Importing again changes nothing and keeps the GUID.
    let meta = fs::read_to_string(assets.join("scripts/fps-controller/player.lua.meta")).unwrap();
    click_import(&mut h, 0);
    assert_eq!(
        fs::read_to_string(assets.join("scripts/fps-controller/player.lua.meta")).unwrap(),
        meta
    );
}

#[test]
fn library_add_to_scene_instantiates_the_headline_prefab() {
    let p = project("library-add");
    let mut h = editor(&p);
    open_library(&mut h);
    // Not before the import.
    let before = state(&mut h).scene.entities.len();
    library_search(&mut h, "third person");
    click_import(&mut h, 0);
    let add = h
        .get_all_by_role(egui::accesskit::Role::Button)
        .filter(|b| b.accesskit_node().label().as_deref() == Some("Add to scene"))
        .filter(|b| !b.accesskit_node().is_disabled())
        .collect::<Vec<_>>();
    assert_eq!(add.len(), 1, "only the imported pack can be added");
    add[0].click();
    h.run_steps(4);
    let s = state(&mut h);
    assert!(s.scene.entities.len() > before, "the prefab was added");
    let root = s
        .scene
        .entities
        .iter()
        .find(|e| {
            e.prefab
                .as_ref()
                .is_some_and(|i| i.asset == "prefabs/third-person-controller/Character.prefab.ron")
        })
        .expect("an instance of the pack's prefab");
    assert!(
        root.position.y > 0.5,
        "a character stands on the ground, not in it"
    );
    assert!(s.selection.is_some());
}

#[test]
fn library_asks_before_overwriting_and_offers_keep_both() {
    let p = project("library-conflict");
    let mine = p.join("assets/scripts/top-down-controller/controller.lua");
    fs::create_dir_all(mine.parent().unwrap()).unwrap();
    fs::write(&mine, "-- mine").unwrap();
    let mut h = editor(&p);
    open_library(&mut h);

    library_search(&mut h, "top down");
    // Cancel: nothing happens.
    click_import(&mut h, 0);
    assert!(h.query_by_label("Files already exist").is_some());
    assert!(
        h.query_by_label_contains("scripts/top-down-controller/controller.lua")
            .is_some()
    );
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.query_by_label("Files already exist").is_none());
    assert_eq!(fs::read_to_string(&mine).unwrap(), "-- mine");
    assert!(!p.join("library.lock.ron").exists());

    // Keep both: the pack's file gets a suffix, the user's stays.
    click_import(&mut h, 0);
    h.get_by_label("Keep both").click();
    h.run_steps(4);
    assert_eq!(fs::read_to_string(&mine).unwrap(), "-- mine");
    assert!(
        p.join("assets/scripts/top-down-controller/controller-2.lua")
            .is_file()
    );
    assert!(
        h.query_all_by_label_contains("Imported top-down-controller")
            .next()
            .is_some()
    );

    // Replace overwrites on the next import.
    click_import(&mut h, 0);
    h.get_by_label("Replace").click();
    h.run_steps(4);
    assert_ne!(fs::read_to_string(&mine).unwrap(), "-- mine");
}

#[test]
fn library_filters_by_category_and_search() {
    let p = project("library-filter");
    let mut h = editor(&p);
    open_library(&mut h);
    assert!(h.query_by_label_contains("All ").is_some());
    h.get_all_by_label_contains("Characters")
        .next()
        .unwrap()
        .click();
    h.run_steps(3);
    assert!(h.query_by_label("First Person Controller").is_some());
    // The search field: the wide text input of the panel.
    h.get_all_by_role(egui::accesskit::Role::TextInput)
        .find(|n| n.rect().width() > 400.0)
        .expect("library search")
        .focus();
    h.run_steps(2);
    h.get_all_by_role(egui::accesskit::Role::TextInput)
        .find(|n| n.rect().width() > 400.0)
        .unwrap()
        .type_text("isometric");
    h.run_steps(4);
    assert!(h.query_by_label("Top Down Controller").is_some());
    assert!(h.query_by_label("First Person Controller").is_none());
    assert!(h.query_by_label("Third Person Controller").is_none());
}

/// `project` plus the vertical slice's skinned robot (clips Idle, Wave) as
/// object 4 with a model.
fn rig_project(name: &str) -> PathBuf {
    let root = project(name);
    fs::create_dir_all(root.join("assets/models")).unwrap();
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../vertical_slice/assets/models/robot.gltf"),
        root.join("assets/models/robot.gltf"),
    )
    .unwrap();
    let path = root.join("assets/scenes/main.scene.ron");
    let mut scene = SceneFile::load(&path).unwrap();
    let mut robot = EntityDesc::new(4, "Robot");
    robot.model = Some(olvin_editor::olvin_scene::file::ModelDesc::new(
        "models/robot.gltf",
    ));
    scene.entities.push(robot);
    scene.save(&path).unwrap();
    root
}

#[test]
fn model_inspector_picks_clips_and_previews_without_dirtying_the_scene() {
    use olvin_editor::olvin_scene::file::AnimationDesc;
    let p = rig_project("rigui");
    let mut h = editor(&p);
    state(&mut h).select(Some(4));
    h.run_steps(5);
    let saved = state(&mut h).scene.clone();

    // Play / stop the preview: state only, no undo step, scene untouched.
    h.get_by_label("▶ Play preview").click();
    h.run_steps(3);
    assert!(state(&mut h).model_previews[&4].playing);
    assert!(!state(&mut h).can_undo() && !state(&mut h).is_dirty());
    assert_eq!(state(&mut h).scene, saved);
    h.get_by_label("■ Stop preview").click();
    h.run_steps(3);
    assert!(!state(&mut h).model_previews[&4].playing);
    assert!(!state(&mut h).can_undo() && !state(&mut h).is_dirty());

    // The preview's clip, speed and loop are not scene data either.
    state(&mut h).model_previews.get_mut(&4).unwrap().clip = Some("Wave".into());
    h.run_steps(3);
    assert_eq!(state(&mut h).scene, saved);

    // The Animation dropdown lists the model's clips and sets the default
    // clip: a saved, undoable edit.
    h.get_all_by_role(egui::accesskit::Role::ComboBox)
        .find(|n| n.accesskit_node().value().as_deref() == Some("None"))
        .expect("the Animation dropdown")
        .click();
    h.run_steps(3);
    h.get_by_label("Wave").click();
    h.run_steps(5);
    assert_eq!(
        state(&mut h)
            .scene
            .get(4)
            .unwrap()
            .model
            .as_ref()
            .unwrap()
            .animation,
        Some(AnimationDesc::Clip("Wave".into()))
    );
    assert!(state(&mut h).can_undo());
    undo(&mut h);
    assert_eq!(
        state(&mut h)
            .scene
            .get(4)
            .unwrap()
            .model
            .as_ref()
            .unwrap()
            .animation,
        None
    );
}

/// A file dragged in from the OS file manager, as the window backend reports it.
#[derive(Debug)]
struct OsFile(PathBuf);

impl egui::DroppedFile for OsFile {
    fn path(&self) -> &std::path::Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        fs::read(&self.0).map_err(|e| e.to_string())
    }
}

#[test]
fn files_dropped_from_the_os_land_in_the_shown_folder() {
    let p = project("os-drop");
    let mut h = editor(&p);
    let src = std::env::temp_dir().join(format!("olvin-os-src-{}", std::process::id()));
    fs::create_dir_all(&src).unwrap();
    let file = src.join("rock.png");
    fs::write(&file, b"png bytes").unwrap();

    // Dropped on the window once, then again: the second copy must not overwrite.
    for _ in 0..2 {
        h.input_mut()
            .dropped_files
            .push(std::sync::Arc::new(OsFile(file.clone())));
        h.run_steps(5);
    }

    assert_eq!(fs::read(p.join("assets/rock.png")).unwrap(), b"png bytes");
    assert!(p.join("assets/rock-2.png").exists(), "second drop renamed");
    assert!(
        p.join("assets/rock.png.meta").exists(),
        "a GUID was created"
    );
    assert!(p.join("assets/rock-2.png.meta").exists());
}

#[test]
fn collider_layer_mask_menu_edits_the_scene() {
    use olvin_editor::olvin_scene::file::{ColliderDesc, ColliderShape};
    let p = project("collider-mask");
    let mut h = editor(&p);
    {
        let s = state(&mut h);
        s.scene.get_mut(3).unwrap().collider =
            Some(ColliderDesc::new(ColliderShape::Box { size: Vec3::ONE }));
        s.select(Some(3));
    }
    h.run_steps(5);
    h.get_by_label("All layers").click();
    h.run_steps(3);
    h.get_by_label("None").click();
    h.run_steps(3);
    let c = state(&mut h)
        .scene
        .get(3)
        .unwrap()
        .collider
        .clone()
        .unwrap();
    assert_eq!(c.mask, 0, "None cleared every layer");
    assert_eq!(c.layer, 0);
}

#[test]
fn joint_section_shows_the_kind_fields_and_connects_to_the_world() {
    use olvin_editor::olvin_scene::file::{JointDesc, JointKind};
    let p = project("joint-section");
    let mut h = editor(&p);
    {
        let s = state(&mut h);
        s.scene.get_mut(3).unwrap().joint = Some(JointDesc::new(JointKind::Hinge {
            axis: Vec3::Y,
            limits: None,
        }));
        s.select(Some(3));
    }
    h.run_steps(5);
    // The hinge's own fields and the connection list.
    h.get_by_label("Angle Limits");
    h.get_by_label("Connected To");
    h.get_by_label("Axis");
}

#[test]
fn a_rebound_tool_key_works_and_the_old_one_does_not() {
    use olvin_editor::keymap::{Chord, Command};
    let p = project("rebound-tool");
    let mut h = editor(&p);
    state(&mut h)
        .prefs
        .keymap
        .set(Command::ToolRotate, Chord::plain(egui::Key::M));
    state(&mut h).select(Some(3));
    h.run_steps(3);
    h.hover_at(egui::pos2(800.0, 400.0));
    h.run_steps(2);
    let before = state(&mut h).tool.clone();
    h.key_press(egui::Key::E);
    h.run_steps(3);
    assert_eq!(state(&mut h).tool, before, "E no longer rotates");
    h.key_press(egui::Key::M);
    h.run_steps(3);
    assert_eq!(state(&mut h).tool, olvin_editor::tools::ROTATE);
}

#[test]
fn settings_rebinds_a_command_by_pressing_the_new_key() {
    use olvin_editor::keymap::Command;
    let p = project("rebind-ui");
    let mut h = editor(&p);
    state(&mut h).settings_page = Some(olvin_editor::settings::Page::Keymap);
    h.run_steps(5);
    h.get_by_label("W").click();
    h.run_steps(3);
    assert!(state(&mut h).capturing_key, "waiting for a key");
    h.key_press(egui::Key::M);
    h.run_steps(3);
    assert!(!state(&mut h).capturing_key);
    let chords = state(&mut h).prefs.keymap.chords(Command::ToolMove);
    assert_eq!(chords.len(), 1);
    assert_eq!(chords[0].to_string(), "M");
    // Esc while waiting changes nothing.
    h.get_by_label("M").click();
    h.run_steps(3);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(!state(&mut h).capturing_key);
    assert_eq!(
        state(&mut h).prefs.keymap.chords(Command::ToolMove)[0].to_string(),
        "M"
    );
}

/// Replaces the Hierarchy with a panel that panics while `BROKEN` is set.
static BROKEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

struct Fragile;
impl olvin_editor::Panel for Fragile {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut olvin_editor::EditorState) {
        if BROKEN.load(std::sync::atomic::Ordering::Relaxed) {
            panic!("fragile panel broke on purpose");
        }
        ui.label("fragile panel is fine");
    }
}

struct FragilePlugin;
impl EditorPlugin for FragilePlugin {
    fn name(&self) -> &'static str {
        "fragile"
    }
    fn build(&self, registry: &mut olvin_editor::Registry) {
        registry.add_panel(olvin_editor::PanelKind {
            id: olvin_editor::panels::HIERARCHY,
            title: "Fragile",
            factory: || Box::new(Fragile),
        });
    }
}

#[test]
fn a_crashing_panel_shows_a_card_and_restarts_without_taking_the_editor_down() {
    isolate_config();
    let p = project("crash");
    let project = p.clone();
    let mut h = Harness::builder()
        .with_size(SIZE)
        .with_step_dt(1.0 / 60.0)
        .build_eframe(move |cc| {
            olvin_editor::theme::apply(&cc.egui_ctx);
            let mut all = plugins();
            all.push(Box::new(FragilePlugin));
            let registry = olvin_editor::registry(&all);
            let mut app = EditorApp::new(registry, Some(project), None);
            // CI runs with OLVIN_FAULTS=abort; this test is about isolation.
            app.set_fault_policy(olvin_editor::olvin::core::FaultPolicy::Isolate);
            app.reset_layout();
            assert!(app.show_workspace("Layout"));
            app
        });
    h.run_steps(5);
    // The editor is alive and still edits.
    assert_eq!(state(&mut h).scene.entities.len(), 3);
    h.get_by_label("⚠ Fragile crashed");
    assert!(
        state(&mut h)
            .log
            .iter()
            .any(|l| l.text.contains("fragile panel broke on purpose")),
        "the crash is in the console log"
    );
    BROKEN.store(false, std::sync::atomic::Ordering::Relaxed);
    h.get_by_label("⟲ Restart panel").click();
    h.run_steps(3);
    h.get_by_label("fragile panel is fine");
}

#[test]
fn a_crash_in_play_reaches_the_console_and_the_subsystems_panel_restarts_it() {
    use olvin_editor::olvin::core::{Resources, Stage};
    struct Healthy;
    let p = project("play-crash");
    let mut h = editor(&p);
    state(&mut h).start_play();
    let play = state(&mut h).play.as_mut().unwrap();
    play.app.add_system(Stage::Update, |res: &mut Resources| {
        if !res.contains::<Healthy>() {
            panic!("game system broke on purpose");
        }
    });
    h.run_steps(3);
    assert!(state(&mut h).play.is_some(), "the game still runs");
    assert!(
        state(&mut h)
            .log
            .iter()
            .any(|l| l.text.contains("game system broke on purpose")),
        "the crash is in the console"
    );

    h.get_by_label("WINDOW").click();
    h.run_steps(3);
    h.get_by_label_contains("Add Panel").click();
    h.run_steps(3);
    h.get_by_label("🛡 Subsystems").click();
    h.run_steps(4);
    h.get_by_label("● core");
    state(&mut h)
        .play
        .as_mut()
        .unwrap()
        .app
        .insert_resource(Healthy);
    h.get_by_label("⟲ Restart").click();
    h.run_steps(3);
    h.get_by_label("✔ All subsystems are running");
}
