//! Headless tests driving the panel through egui input events.

use olvin_editor::NewEntity;
use olvin_editor::egui::{self, Event, Key, Modifiers, PointerButton, Pos2, Rect};
use olvin_editor::glam::Vec2;
use olvin_editor::{EditorState, Panel};
use olvin_mesh::PolyMesh;

use super::{UvEditor, p2};
use crate::select::SelectMode;

fn frame(ctx: &egui::Context, panel: &mut UvEditor, state: &mut EditorState, events: Vec<Event>) {
    // egui takes modifiers from ModifiersChanged events, not from key events.
    let modifiers = events
        .iter()
        .find_map(|e| match e {
            Event::Key { modifiers, .. } => Some(*modifiers),
            _ => None,
        })
        .unwrap_or_default();
    let mut events = events;
    events.insert(0, Event::ModifiersChanged(modifiers));
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 700.0))),
        events,
        ..Default::default()
    };
    ctx.run_ui(input, |ui| panel.ui(ui, state))
        .textures_delta
        .clear();
}

fn key(key: Key, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

fn mesh(state: &EditorState, id: u64) -> &PolyMesh {
    state.scene.get(id).unwrap().poly.as_ref().unwrap()
}

#[test]
fn draws_headless_and_runs_hotkeys() {
    let mut state = EditorState::new(None);
    let ctx = egui::Context::default();
    let mut panel = UvEditor::default();
    // Nothing selected: just the hint.
    frame(&ctx, &mut panel, &mut state, vec![]);
    let id = state.create(NewEntity::Empty, None);
    state.scene.get_mut(id).unwrap().poly = Some(PolyMesh::cube(1.0));
    let at = Pos2::new(400.0, 400.0);
    let hover = || Event::PointerMoved(at);
    for _ in 0..2 {
        frame(&ctx, &mut panel, &mut state, vec![hover()]);
    }
    assert!(!mesh(&state, id).has_uvs(), "drawing does not write UVs");
    // A selects everything, U unwraps it into six islands.
    frame(
        &ctx,
        &mut panel,
        &mut state,
        vec![hover(), key(Key::A, Modifiers::NONE)],
    );
    assert_eq!(panel.sel.corners.len(), 24);
    frame(
        &ctx,
        &mut panel,
        &mut state,
        vec![hover(), key(Key::U, Modifiers::NONE)],
    );
    assert!(mesh(&state, id).has_uvs());
    assert_eq!(mesh(&state, id).islands().len(), 6);
    let before = mesh(&state, id).uvs.clone();

    // S, type 0.5, Enter: everything scales about the pivot.
    frame(
        &ctx,
        &mut panel,
        &mut state,
        vec![hover(), key(Key::S, Modifiers::NONE)],
    );
    assert!(panel.modal.is_some() && state.busy);
    frame(
        &ctx,
        &mut panel,
        &mut state,
        vec![hover(), Event::Text("0.5".into())],
    );
    frame(
        &ctx,
        &mut panel,
        &mut state,
        vec![hover(), key(Key::Enter, Modifiers::NONE)],
    );
    assert!(panel.modal.is_none());
    let after = &mesh(&state, id).uvs;
    let span = |uvs: &Vec<Vec<Vec2>>| {
        let (l, h) = uvs
            .iter()
            .flatten()
            .fold((Vec2::MAX, Vec2::MIN), |(l, h), &t| (l.min(t), h.max(t)));
        h - l
    };
    assert!((span(after) * 2.0).abs_diff_eq(span(&before), 1e-4));

    // G, move the mouse, Esc: cancelled, nothing changed.
    let scaled = mesh(&state, id).clone();
    frame(
        &ctx,
        &mut panel,
        &mut state,
        vec![hover(), key(Key::G, Modifiers::NONE)],
    );
    frame(
        &ctx,
        &mut panel,
        &mut state,
        vec![Event::PointerMoved(Pos2::new(460.0, 380.0))],
    );
    assert_ne!(mesh(&state, id), &scaled);
    frame(
        &ctx,
        &mut panel,
        &mut state,
        vec![key(Key::Escape, Modifiers::NONE)],
    );
    assert_eq!(mesh(&state, id), &scaled);

    // G X 0.25 Enter: a typed move along U.
    frame(
        &ctx,
        &mut panel,
        &mut state,
        vec![hover(), key(Key::G, Modifiers::NONE)],
    );
    frame(
        &ctx,
        &mut panel,
        &mut state,
        vec![
            hover(),
            key(Key::X, Modifiers::NONE),
            Event::Text("0.25".into()),
        ],
    );
    frame(
        &ctx,
        &mut panel,
        &mut state,
        vec![hover(), key(Key::Enter, Modifiers::NONE)],
    );
    let moved = &mesh(&state, id).uvs;
    for (a, b) in moved.iter().flatten().zip(scaled.uvs.iter().flatten()) {
        assert!(a.abs_diff_eq(*b + Vec2::new(0.25, 0.0), 1e-5));
    }

    // Island mode, Alt+A, then Ctrl+P packs back into 0..1; every mode draws.
    for (k, mode) in [
        (Key::Num2, SelectMode::Edge),
        (Key::Num3, SelectMode::Face),
        (Key::Num4, SelectMode::Island),
    ] {
        frame(
            &ctx,
            &mut panel,
            &mut state,
            vec![hover(), key(k, Modifiers::NONE)],
        );
        assert_eq!(panel.mode, mode);
    }
    frame(
        &ctx,
        &mut panel,
        &mut state,
        vec![hover(), key(Key::A, Modifiers::ALT)],
    );
    assert!(panel.sel.is_empty());
    frame(
        &ctx,
        &mut panel,
        &mut state,
        vec![hover(), key(Key::P, Modifiers::COMMAND)],
    );
    let m = mesh(&state, id);
    assert!(
        m.uvs
            .iter()
            .flatten()
            .all(|t| t.x >= -1e-4 && t.x <= 1.0001 && t.y >= -1e-4 && t.y <= 1.0001)
    );
    m.validate().unwrap();
    frame(
        &ctx,
        &mut panel,
        &mut state,
        vec![
            hover(),
            key(Key::F, Modifiers::NONE),
            key(Key::Home, Modifiers::NONE),
        ],
    );
}

#[test]
fn click_and_box_select_on_canvas() {
    let mut state = EditorState::new(None);
    let ctx = egui::Context::default();
    let mut panel = UvEditor::default();
    let id = state.create(NewEntity::Empty, None);
    let mut plane = PolyMesh::plane(2.0);
    plane.reset_uvs(&[0]);
    state.scene.get_mut(id).unwrap().poly = Some(plane);
    frame(
        &ctx,
        &mut panel,
        &mut state,
        vec![Event::PointerMoved(Pos2::new(400.0, 400.0))],
    );
    frame(
        &ctx,
        &mut panel,
        &mut state,
        vec![Event::PointerMoved(Pos2::new(400.0, 400.0))],
    );
    // Where is UV (0, 0) on screen? Use the panel's own view.
    let canvas_center = panel.last_origin;
    let corner = p2(panel.view.to_screen(canvas_center, Vec2::ZERO));
    let press = |pressed| Event::PointerButton {
        pos: corner,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    frame(
        &ctx,
        &mut panel,
        &mut state,
        vec![Event::PointerMoved(corner), press(true)],
    );
    frame(&ctx, &mut panel, &mut state, vec![press(false)]);
    frame(&ctx, &mut panel, &mut state, vec![]);
    assert_eq!(panel.sel.corners.len(), 1, "clicked the (0, 0) corner");

    // Box from outside the square across everything selects all 4 corners.
    let a = p2(panel.view.to_screen(canvas_center, Vec2::splat(-0.05)));
    let b = p2(panel.view.to_screen(canvas_center, Vec2::splat(1.05)));
    let button = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    let mut events = vec![vec![Event::PointerMoved(a), button(a, true)]];
    for k in 1..=6 {
        events.push(vec![Event::PointerMoved(a + (b - a) * (k as f32 / 6.0))]);
    }
    events.push(vec![button(b, false)]);
    events.push(vec![]);
    for e in events {
        frame(&ctx, &mut panel, &mut state, e);
    }
    assert_eq!(panel.sel.corners.len(), 4);
}
