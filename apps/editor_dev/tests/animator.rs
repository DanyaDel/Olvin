//! Drives the Animator through the real editor UI, headless: input events go
//! into the egui_kittest harness (like a user's keyboard and mouse), the
//! assertions read the editor state. The harness tests render with wgpu and
//! are ignored by default, like `screenshots`:
//!
//! ```sh
//! __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json WGPU_BACKEND=gl \
//! cargo test -p editor_dev --test animator -- --ignored
//! ```
//!
//! PNGs go to `$OLVIN_ANIM_SHOTS` when set.

#![allow(dead_code)]

#[path = "screenshots.rs"]
mod common;

use std::path::PathBuf;

use common::{EditorRenderer, SELECTED, SIZE, demo_project, plugins};
use egui_kittest::Harness;
use olvin_editor::egui::{self, Key, Modifiers, pos2};
use olvin_editor::olvin_scene::action::Prop;
use olvin_editor::theme::ThemeKind;
use olvin_editor::{EditorApp, EditorState};

/// The viewport of the Animation workspace.
const VIEWPORT: egui::Pos2 = pos2(800.0, 250.0);

fn project() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "olvin-anim-{}-{:?}/proj",
        std::process::id(),
        std::thread::current().id()
    ));
    demo_project(&dir);
    dir
}

fn editor(workspace: &str) -> Harness<'static, EditorApp> {
    let project = project();
    let workspace = workspace.to_owned();
    let mut harness = Harness::builder()
        .with_size(SIZE)
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .renderer(EditorRenderer::new())
        .build_eframe(move |cc| -> EditorApp {
            let registry = olvin_editor::registry(&plugins());
            let mut app = olvin_editor::build_app(cc, registry, Some(project)).expect("editor app");
            app.reset_layout();
            assert!(app.show_workspace(&workspace), "no workspace {workspace}");
            let state = app.state_mut();
            state.prefs = olvin_editor::prefs::Preferences {
                theme: ThemeKind::Dark,
                ..Default::default()
            };
            state.select(Some(SELECTED));
            app
        });
    harness.run_steps(20);
    harness
}

fn state<'a>(h: &'a mut Harness<'static, EditorApp>) -> &'a mut EditorState {
    h.state_mut().state_mut()
}

/// A key press with the pointer over `at`.
fn press(h: &mut Harness<'static, EditorApp>, at: egui::Pos2, key: Key) {
    h.hover_at(at);
    h.run_steps(2);
    h.key_press(key);
    h.run_steps(3);
}

fn press_mod(h: &mut Harness<'static, EditorApp>, at: egui::Pos2, mods: Modifiers, key: Key) {
    h.hover_at(at);
    h.run_steps(2);
    h.key_press_modifiers(mods, key);
    h.run_steps(3);
}

fn shots() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("OLVIN_ANIM_SHOTS")?);
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn shot(h: &mut Harness<'static, EditorApp>, name: &str) {
    if let Some(dir) = shots() {
        h.remove_cursor();
        h.run_steps(2);
        let image = h.render().expect("render");
        image::DynamicImage::ImageRgba8(image)
            .to_rgb8()
            .save(dir.join(name))
            .unwrap();
    }
}

fn x_keys(s: &EditorState, prop: Prop, index: u8) -> Vec<(f32, f32)> {
    s.scene
        .animation
        .curve(SELECTED, prop, index)
        .map(|c| c.keys.iter().map(|k| (k.frame, k.value)).collect())
        .unwrap_or_default()
}

#[test]
#[ignore = "renders with wgpu; run with --ignored"]
fn i_in_the_viewport_keys_location_rotation_scale_and_alt_i_removes_them() {
    let mut h = editor("Animation");
    assert_eq!(state(&mut h).anim_frame, 1);
    press(&mut h, VIEWPORT, Key::I);
    let s = state(&mut h);
    assert_eq!(
        s.scene.animation.curves.len(),
        9,
        "Blender 4.x default channels"
    );
    assert!(
        s.scene
            .animation
            .curves
            .iter()
            .all(|c| c.keys.len() == 1 && c.keys[0].frame == 1.0)
    );
    // Frame 10, move the crate, key again.
    for _ in 0..9 {
        press(&mut h, VIEWPORT, Key::ArrowRight);
    }
    assert_eq!(state(&mut h).anim_frame, 10);
    state(&mut h).scene.get_mut(SELECTED).unwrap().position.x = 8.0;
    press(&mut h, VIEWPORT, Key::I);
    assert_eq!(
        x_keys(state(&mut h), Prop::Location, 0),
        [(1.0, 0.0), (10.0, 8.0)]
    );
    // Back to frame 5: the animation is evaluated into the object.
    for _ in 0..5 {
        press(&mut h, VIEWPORT, Key::ArrowLeft);
    }
    let x = state(&mut h).scene.get(SELECTED).unwrap().position.x;
    let expected = state(&mut h)
        .scene
        .animation
        .eval(SELECTED, Prop::Location, 0, 5.0)
        .unwrap();
    assert!(
        (x - expected).abs() < 1e-4 && x > 0.5 && x < 7.5,
        "x at frame 5: {x}"
    );
    // Moving the playhead is not an edit: nothing new to save, no extra undo step.
    // Alt+I at a key frame deletes that frame's keys.
    for _ in 0..4 {
        press(&mut h, VIEWPORT, Key::ArrowRight);
    }
    assert_eq!(state(&mut h).anim_frame, 9);
    press(&mut h, VIEWPORT, Key::ArrowRight);
    press_mod(&mut h, VIEWPORT, Modifiers::ALT, Key::I);
    assert_eq!(x_keys(state(&mut h), Prop::Location, 0), [(1.0, 0.0)]);
    // Undo brings them back.
    state(&mut h).undo();
    h.run_steps(3);
    assert_eq!(x_keys(state(&mut h), Prop::Location, 0).len(), 2);
}

#[test]
#[ignore = "renders with wgpu; run with --ignored"]
fn frame_keys_follow_blenders_keymap() {
    let mut h = editor("Animation");
    press(&mut h, VIEWPORT, Key::I); // key at frame 1
    for _ in 0..19 {
        press(&mut h, VIEWPORT, Key::ArrowRight);
    }
    press(&mut h, VIEWPORT, Key::I); // key at frame 20
    press_mod(&mut h, VIEWPORT, Modifiers::SHIFT, Key::ArrowLeft);
    assert_eq!(state(&mut h).anim_frame, 1, "Shift+Left: start");
    press_mod(&mut h, VIEWPORT, Modifiers::SHIFT, Key::ArrowRight);
    assert_eq!(state(&mut h).anim_frame, 250, "Shift+Right: end");
    press(&mut h, VIEWPORT, Key::ArrowDown);
    assert_eq!(state(&mut h).anim_frame, 20, "Down: previous keyframe");
    press(&mut h, VIEWPORT, Key::ArrowDown);
    assert_eq!(state(&mut h).anim_frame, 1);
    press(&mut h, VIEWPORT, Key::ArrowUp);
    assert_eq!(state(&mut h).anim_frame, 20, "Up: next keyframe");
}

#[test]
#[ignore = "renders with wgpu; run with --ignored"]
fn space_plays_at_the_fps_and_esc_returns_to_the_start_frame() {
    let mut h = editor("Animation");
    for _ in 0..4 {
        press(&mut h, VIEWPORT, Key::ArrowRight);
    }
    assert_eq!(state(&mut h).anim_frame, 5);
    press(&mut h, VIEWPORT, Key::Space);
    h.run_steps(60); // one second at 60 Hz
    let f = state(&mut h).anim_frame;
    assert!((25..=33).contains(&f), "about 24 frames later, got {f}");
    // Space pauses where it is.
    press(&mut h, VIEWPORT, Key::Space);
    let paused = state(&mut h).anim_frame;
    h.run_steps(30);
    assert_eq!(state(&mut h).anim_frame, paused);
    // Esc during playback restores the frame playback began from.
    press(&mut h, VIEWPORT, Key::Space);
    h.run_steps(30);
    assert_ne!(state(&mut h).anim_frame, paused);
    press(&mut h, VIEWPORT, Key::Escape);
    assert_eq!(state(&mut h).anim_frame, paused);
}

#[test]
#[ignore = "renders with wgpu; run with --ignored"]
fn animation_workspace_screenshots() {
    let mut h = editor("Animation");
    press(&mut h, VIEWPORT, Key::I);
    for _ in 0..11 {
        press(&mut h, VIEWPORT, Key::ArrowRight);
    }
    {
        let s = state(&mut h);
        let e = s.scene.get_mut(SELECTED).unwrap();
        e.position = olvin_editor::glam::Vec3::new(4.0, 1.5, -2.0);
        e.rotation.y = 120.0;
    }
    press(&mut h, VIEWPORT, Key::I);
    for _ in 0..12 {
        press(&mut h, VIEWPORT, Key::ArrowRight);
    }
    {
        let s = state(&mut h);
        let e = s.scene.get_mut(SELECTED).unwrap();
        e.position = olvin_editor::glam::Vec3::new(0.0, 0.5, 0.0);
        e.rotation.y = 20.0;
    }
    press(&mut h, VIEWPORT, Key::I);
    for _ in 0..8 {
        press(&mut h, VIEWPORT, Key::ArrowLeft);
    }
    h.run_steps(30);
    shot(&mut h, "10-animation-workspace.png");
}

/// Keys a crate: location x 0 -> 10 -> 0 (frames 1, 12, 24), rotation y 0 -> 120 -> 20.
fn seed(s: &mut EditorState) {
    for (f, x, ry, y) in [
        (1.0, 0.0, 0.0, 0.5),
        (12.0, 10.0, 120.0, 3.0),
        (24.0, 0.0, 20.0, 0.5),
    ] {
        s.scene
            .animation
            .insert_key(SELECTED, Prop::Location, 0, f, x);
        s.scene
            .animation
            .insert_key(SELECTED, Prop::Location, 1, f, y);
        s.scene
            .animation
            .insert_key(SELECTED, Prop::Rotation, 1, f, ry);
    }
}

/// The Graph Editor tab next to the Dope Sheet's.
const GRAPH_TAB: egui::Pos2 = pos2(455.0, 477.0);

fn click(h: &mut Harness<'static, EditorApp>, at: egui::Pos2) {
    h.hover_at(at);
    h.run_steps(2);
    h.drag_at(at);
    h.run_steps(2);
    h.drop_at(at);
    h.run_steps(3);
}

/// Dope Sheet pane (calibrated on the Animation workspace at 1600x940).
const SHEET: egui::Pos2 = pos2(800.0, 560.0);

#[test]
#[ignore = "renders with wgpu; run with --ignored"]
fn dope_sheet_and_graph_editor_screenshots() {
    let mut h = editor("Animation");
    seed(state(&mut h));
    h.run_steps(5);
    press(&mut h, SHEET, Key::Home);
    shot(&mut h, "11-dopesheet.png");
    click(&mut h, GRAPH_TAB);
    h.run_steps(5);
    press(&mut h, SHEET, Key::Home);
    shot(&mut h, "12-graph-editor.png");
}

fn drag(h: &mut Harness<'static, EditorApp>, from: egui::Pos2, to: egui::Pos2) {
    h.hover_at(from);
    h.run_steps(2);
    h.drag_at(from);
    h.run_steps(2);
    h.hover_at(from + (to - from) * 0.5);
    h.run_steps(2);
    h.hover_at(to);
    h.run_steps(3);
    h.drop_at(to);
    h.run_steps(3);
}

/// Dope Sheet columns after "View All" on the seed keys: frames 1, 12, 24.
const F1: f32 = 496.0;
const F12: f32 = 852.0;
const F24: f32 = 1240.0;
/// Rows: Summary, object, group, X Location, Y Location, Y Euler Rotation.
const SUMMARY: f32 = 567.0;
const X_LOC: f32 = 633.0;
const Y_ROT: f32 = 677.0;

fn seeded_sheet() -> Harness<'static, EditorApp> {
    let mut h = editor("Animation");
    seed(state(&mut h));
    h.run_steps(5);
    press(&mut h, SHEET, Key::Home);
    h
}

fn frames(s: &EditorState, prop: Prop, index: u8) -> Vec<f32> {
    s.scene
        .animation
        .curve(SELECTED, prop, index)
        .map(|c| c.keys.iter().map(|k| k.frame).collect())
        .unwrap_or_default()
}

#[test]
#[ignore = "renders with wgpu; run with --ignored"]
fn dope_sheet_click_shift_click_and_summary_selection() {
    let mut h = seeded_sheet();
    click(&mut h, pos2(F12, X_LOC));
    assert_eq!(
        state(&mut h).scene.animation.selected_count(),
        1,
        "one channel's key"
    );
    h.hover_at(pos2(F24, X_LOC));
    h.run_steps(2);
    // Shift+click through a raw pointer event with the modifier held.
    let at = pos2(F24, X_LOC);
    h.event_modifiers(
        egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::SHIFT,
        },
        Modifiers::SHIFT,
    );
    h.run_steps(2);
    h.event_modifiers(
        egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::SHIFT,
        },
        Modifiers::SHIFT,
    );
    h.run_steps(3);
    assert_eq!(
        state(&mut h).scene.animation.selected_count(),
        2,
        "shift adds"
    );
    // A summary key selects the frame in every channel.
    click(&mut h, pos2(F12, SUMMARY));
    assert_eq!(state(&mut h).scene.animation.selected_count(), 3);
    // Empty space deselects; A selects all, Alt+A none.
    click(&mut h, pos2(1000.0, X_LOC));
    assert_eq!(state(&mut h).scene.animation.selected_count(), 0);
    press(&mut h, SHEET, Key::A);
    assert_eq!(state(&mut h).scene.animation.selected_count(), 9);
    press_mod(&mut h, SHEET, Modifiers::ALT, Key::A);
    assert_eq!(state(&mut h).scene.animation.selected_count(), 0);
    shot(&mut h, "13-dopesheet-selected.png");
}

#[test]
#[ignore = "renders with wgpu; run with --ignored"]
fn dope_sheet_drag_moves_keys_by_whole_frames_and_undo_restores() {
    let mut h = seeded_sheet();
    // 3.4 frames of mouse travel: keys land on a whole frame.
    drag(&mut h, pos2(F12, SUMMARY), pos2(F12 + 3.4 * 32.4, SUMMARY));
    for (prop, i) in [
        (Prop::Location, 0),
        (Prop::Location, 1),
        (Prop::Rotation, 1),
    ] {
        assert_eq!(frames(state(&mut h), prop, i), [1.0, 15.0, 24.0]);
    }
    // One undo step for the whole drag.
    state(&mut h).undo();
    h.run_steps(3);
    assert_eq!(frames(state(&mut h), Prop::Location, 0), [1.0, 12.0, 24.0]);
    state(&mut h).redo();
    h.run_steps(3);
    assert_eq!(frames(state(&mut h), Prop::Location, 0), [1.0, 15.0, 24.0]);
}

#[test]
#[ignore = "renders with wgpu; run with --ignored"]
fn dope_sheet_g_with_typed_number_and_esc_cancels() {
    let mut h = seeded_sheet();
    click(&mut h, pos2(F12, SUMMARY));
    press(&mut h, SHEET, Key::G);
    h.event(egui::Event::Text("5".into()));
    h.run_steps(3);
    // Live preview while the operation runs.
    assert_eq!(frames(state(&mut h), Prop::Location, 0), [1.0, 17.0, 24.0]);
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(frames(state(&mut h), Prop::Location, 0), [1.0, 17.0, 24.0]);
    // Esc puts everything back.
    press(&mut h, SHEET, Key::G);
    h.event(egui::Event::Text("-3".into()));
    h.run_steps(3);
    assert_eq!(frames(state(&mut h), Prop::Location, 0), [1.0, 14.0, 24.0]);
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert_eq!(frames(state(&mut h), Prop::Location, 0), [1.0, 17.0, 24.0]);
}

#[test]
#[ignore = "renders with wgpu; run with --ignored"]
fn dope_sheet_scale_x_delete_copy_paste_and_interpolation() {
    let mut h = seeded_sheet();
    // S scales around the current frame (1).
    press(&mut h, SHEET, Key::A);
    press(&mut h, SHEET, Key::S);
    h.event(egui::Event::Text("2".into()));
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(frames(state(&mut h), Prop::Location, 0), [1.0, 23.0, 47.0]);
    state(&mut h).undo();
    h.run_steps(3);
    assert_eq!(frames(state(&mut h), Prop::Location, 0), [1.0, 12.0, 24.0]);
    // X deletes the selected keys (and does not delete the object).
    click(&mut h, pos2(F12, SUMMARY));
    press(&mut h, SHEET, Key::X);
    assert_eq!(frames(state(&mut h), Prop::Location, 0), [1.0, 24.0]);
    assert!(
        state(&mut h).scene.get(SELECTED).is_some(),
        "the crate itself stays"
    );
    state(&mut h).undo();
    h.run_steps(3);
    // Copy frame 12, paste at the current frame (move the playhead first).
    click(&mut h, pos2(F12, SUMMARY));
    h.hover_at(SHEET);
    h.run_steps(2);
    h.event(egui::Event::Copy);
    h.run_steps(3);
    state(&mut h).anim_frame = 18;
    h.run_steps(3);
    h.hover_at(SHEET);
    h.run_steps(2);
    h.event(egui::Event::Paste(String::new()));
    h.run_steps(3);
    assert_eq!(
        frames(state(&mut h), Prop::Location, 0),
        [1.0, 12.0, 18.0, 24.0]
    );
    // T: interpolation menu, pick Constant for the selected (pasted) keys.
    press(&mut h, pos2(F12, X_LOC), Key::T);
    click(&mut h, pos2(F12 + 40.0, X_LOC + 20.0));
    let interp = state(&mut h)
        .scene
        .animation
        .curve(SELECTED, Prop::Location, 0)
        .unwrap()
        .keys[2]
        .interp;
    assert_eq!(
        interp,
        olvin_editor::olvin_scene::action::Interp::Constant,
        "T menu applied to the selected keys"
    );
}

#[test]
#[ignore = "renders with wgpu; run with --ignored"]
fn dope_sheet_box_select_and_scrubbing_the_ruler() {
    let mut h = seeded_sheet();
    drag(&mut h, pos2(1000.0, X_LOC - 8.0), pos2(1300.0, Y_ROT + 8.0));
    let s = state(&mut h);
    assert_eq!(
        s.scene.animation.selected_count(),
        3,
        "frame 24 in three channels"
    );
    assert!(
        s.scene
            .animation
            .curves
            .iter()
            .all(|c| c.keys[2].is_selected())
    );
    // Dragging in the ruler sets the frame, to whole numbers.
    drag(&mut h, pos2(F12, 543.0), pos2(F12 + 2.6 * 32.4, 543.0));
    assert_eq!(state(&mut h).anim_frame, 15);
}

#[test]
#[ignore = "renders with wgpu; run with --ignored"]
fn graph_editor_selects_and_drags_keys_and_handles() {
    let mut h = seeded_sheet();
    click(&mut h, GRAPH_TAB);
    press(&mut h, SHEET, Key::Home);
    // The rotation key at frame 12 sits at the top (calibrated: x 847, y 586).
    let key = pos2(847.0, 586.0);
    click(&mut h, key);
    let sel = state(&mut h)
        .scene
        .animation
        .curve(SELECTED, Prop::Rotation, 1)
        .unwrap()
        .keys[1]
        .select;
    assert_eq!(sel, olvin_editor::olvin_scene::action::SEL_ALL);
    // Drag it down: the value changes, the frame stays whole.
    drag(&mut h, key, pos2(847.0, 640.0));
    let k = state(&mut h)
        .scene
        .animation
        .curve(SELECTED, Prop::Rotation, 1)
        .unwrap()
        .keys[1];
    assert!(k.value < 100.0 && k.value > 20.0, "value {}", k.value);
    assert_eq!(k.frame, 12.0);
    state(&mut h).undo();
    h.run_steps(3);
    // A handle drag turns the automatic handle into an aligned one.
    let handle = pos2(957.0, 586.0);
    drag(&mut h, handle, pos2(957.0, 640.0));
    let k = state(&mut h)
        .scene
        .animation
        .curve(SELECTED, Prop::Rotation, 1)
        .unwrap()
        .keys[1];
    use olvin_editor::olvin_scene::action::HandleType;
    assert_eq!(
        (k.left_type, k.right_type),
        (HandleType::Aligned, HandleType::Aligned)
    );
    assert!(
        k.right[1] < k.value,
        "handle pulled below the key: {:?}",
        k.right
    );
    assert!(
        k.left[1] > k.value,
        "the twin tilts the other way: {:?}",
        k.left
    );
    shot(&mut h, "14-graph-handle-dragged.png");
}
