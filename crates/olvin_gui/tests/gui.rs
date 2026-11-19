use glam::Vec2;
use olvin_core::{App, Events};
use olvin_gui::*;
use olvin_input::{Input, InputMap, KeyCode, MouseButton};

const VIEW: Vec2 = Vec2::new(800.0, 600.0);

/// 10 px per character, 20 px line: easy numbers.
struct Fixed;
impl TextMeasure for Fixed {
    fn measure(&self, text: &str, _size: f32) -> Vec2 {
        Vec2::new(10.0 * text.chars().count() as f32, 20.0)
    }
}

fn laid(ui: &mut Ui) {
    ui.layout(VIEW, &Fixed);
}

fn approx(a: Rect, min: (f32, f32), size: (f32, f32)) {
    let ok = (a.min - Vec2::new(min.0, min.1)).abs().max_element() < 0.01
        && (a.size() - Vec2::new(size.0, size.1)).abs().max_element() < 0.01;
    assert!(ok, "{a:?} != {min:?} {size:?}");
}

/// A centered column of three buttons.
fn menu() -> (Ui, [NodeId; 3]) {
    let mut ui = Ui::new();
    let col = ui.add(
        ui.root(),
        Node::panel()
            .column()
            .gap(10.0)
            .align(Align::Stretch)
            .centered(),
    );
    let ids = ["Play", "Options", "Quit"].map(|t| ui.add(col, Node::button(t).name(t)));
    laid(&mut ui);
    (ui, ids)
}

fn click_at(ui: &mut Ui, p: Vec2) {
    ui.update(&UiInput {
        pointer: Some(p),
        pointer_pressed: true,
        ..Default::default()
    });
    ui.update(&UiInput {
        pointer: Some(p),
        pointer_released: true,
        ..Default::default()
    });
}

#[test]
fn content_sizes_and_padding() {
    let mut ui = Ui::new();
    let b = ui.add(ui.root(), Node::button("Play")); // 40x20 text + 12 padding each side
    let l = ui.add(ui.root(), Node::label("Hello"));
    laid(&mut ui);
    approx(ui.rect(b), (0.0, 0.0), (64.0, 44.0));
    approx(ui.rect(l), (0.0, 0.0), (50.0, 20.0));
}

#[test]
fn row_gap_and_fill_weights() {
    let mut ui = Ui::new();
    let row = ui.add(
        ui.root(),
        Node::panel().row().gap(10.0).padding(5.0).px(400.0, 50.0),
    );
    let a = ui.add(row, Node::panel().px(100.0, 20.0));
    let b = ui.add(row, Node::panel().size(Dim::Fill(1.0), Dim::Fixed(20.0)));
    let c = ui.add(row, Node::panel().size(Dim::Fill(3.0), Dim::Fixed(20.0)));
    laid(&mut ui);
    // inner width 390, minus 100 and 2 gaps (20) = 270 free: 67.5 / 202.5.
    approx(ui.rect(a), (5.0, 5.0), (100.0, 20.0));
    approx(ui.rect(b), (115.0, 5.0), (67.5, 20.0));
    approx(ui.rect(c), (192.5, 5.0), (202.5, 20.0));
}

#[test]
fn column_justify_and_align() {
    let mut ui = Ui::new();
    let col = ui.add(
        ui.root(),
        Node::panel()
            .column()
            .justify(Justify::SpaceBetween)
            .align(Align::End)
            .px(100.0, 100.0),
    );
    let a = ui.add(col, Node::panel().px(30.0, 10.0));
    let b = ui.add(col, Node::panel().px(50.0, 10.0));
    laid(&mut ui);
    approx(ui.rect(a), (70.0, 0.0), (30.0, 10.0));
    approx(ui.rect(b), (50.0, 90.0), (50.0, 10.0));

    let mut ui = Ui::new();
    let col = ui.add(
        ui.root(),
        Node::panel()
            .row()
            .justify(Justify::Center)
            .align(Align::Center)
            .px(100.0, 40.0),
    );
    let a = ui.add(col, Node::panel().px(20.0, 10.0));
    laid(&mut ui);
    approx(ui.rect(a), (40.0, 15.0), (20.0, 10.0));
}

#[test]
fn content_panel_wraps_children() {
    let mut ui = Ui::new();
    let col = ui.add(ui.root(), Node::panel().column().gap(4.0).padding(8.0));
    ui.add(col, Node::label("abcd"));
    ui.add(col, Node::label("ab"));
    ui.add(col, Node::label("hidden").hidden());
    laid(&mut ui);
    approx(ui.rect(col), (0.0, 0.0), (56.0, 60.0)); // 40+16, 20+4+20+16
}

#[test]
fn anchors_pin_to_corners_and_center() {
    let mut ui = Ui::new();
    let br = ui.add(
        ui.root(),
        Node::panel()
            .px(100.0, 50.0)
            .anchor(Vec2::ONE, Vec2::ONE, Vec2::new(-10.0, -10.0)),
    );
    let c = ui.add(ui.root(), Node::panel().px(100.0, 100.0).centered());
    let full = ui.add(ui.root(), Node::panel().fill());
    laid(&mut ui);
    approx(ui.rect(br), (690.0, 540.0), (100.0, 50.0));
    approx(ui.rect(c), (350.0, 250.0), (100.0, 100.0));
    approx(ui.rect(full), (0.0, 0.0), (800.0, 600.0));
}

#[test]
fn hidden_subtrees_are_not_focusable() {
    let (mut ui, [play, ..]) = menu();
    assert!(ui.focusable(play));
    let col = ui.node(play).unwrap().rect();
    assert!(col.size().x > 0.0);
    ui.set_visible(ui.find("Play").unwrap(), false);
    laid(&mut ui);
    assert!(!ui.focusable(play));
    ui.set_enabled(ui.find("Quit").unwrap(), false);
    assert!(!ui.focusable(ui.find("Quit").unwrap()));
}

#[test]
fn directional_navigation_in_a_column() {
    let (mut ui, [play, options, quit]) = menu();
    ui.navigate(Dir::Down); // nothing focused: first
    assert_eq!(ui.focused(), Some(play));
    ui.navigate(Dir::Down);
    assert_eq!(ui.focused(), Some(options));
    ui.navigate(Dir::Right); // nothing to the right: stays
    assert_eq!(ui.focused(), Some(options));
    ui.navigate(Dir::Down);
    ui.navigate(Dir::Down); // end: stays
    assert_eq!(ui.focused(), Some(quit));
    ui.navigate(Dir::Up);
    assert_eq!(ui.focused(), Some(options));
}

#[test]
fn directional_navigation_in_a_grid_and_skips_disabled() {
    let mut ui = Ui::new();
    let grid = ui.add(ui.root(), Node::panel().column().gap(10.0));
    let mut ids = Vec::new();
    for r in 0..2 {
        let row = ui.add(grid, Node::panel().row().gap(10.0));
        for c in 0..3 {
            ids.push(ui.add(row, Node::button(&format!("{r}{c}"))));
        }
    }
    ui.set_enabled(ids[4], false);
    laid(&mut ui);
    ui.set_focus(ids[1]);
    ui.navigate(Dir::Down); // straight below is disabled: nearest enabled one below
    assert_ne!(ui.focused(), Some(ids[4]));
    assert!(matches!(ui.focused(), Some(f) if f == ids[3] || f == ids[5]));
    ui.set_focus(ids[0]);
    ui.navigate(Dir::Right);
    assert_eq!(ui.focused(), Some(ids[1]));
}

#[test]
fn tab_order_wraps() {
    let (mut ui, [play, options, quit]) = menu();
    ui.focus_next();
    assert_eq!(ui.focused(), Some(play));
    ui.focus_prev();
    assert_eq!(ui.focused(), Some(quit));
    ui.focus_next();
    assert_eq!(ui.focused(), Some(play));
    ui.focus_next();
    assert_eq!(ui.focused(), Some(options));
}

#[test]
fn pointer_click_needs_press_and_release_on_the_same_button() {
    let (mut ui, [play, options, _]) = menu();
    let on_play = ui.rect(play).center();
    let on_options = ui.rect(options).center();
    click_at(&mut ui, on_play);
    // the release frame carries the click
    assert!(ui.events().contains(&UiEvent::Click {
        node: play,
        name: Some("Play".into())
    }));

    // press on Play, release on Options: no click
    ui.update(&UiInput {
        pointer: Some(on_play),
        pointer_pressed: true,
        ..Default::default()
    });
    assert_eq!(ui.state(play), ButtonState::Pressed);
    ui.update(&UiInput {
        pointer: Some(on_options),
        pointer_released: true,
        ..Default::default()
    });
    assert!(
        !ui.events()
            .iter()
            .any(|e| matches!(e, UiEvent::Click { .. }))
    );

    // click on empty space does nothing
    click_at(&mut ui, Vec2::new(1.0, 1.0));
    assert!(ui.events().is_empty());
}

#[test]
fn hover_moves_focus_and_disabled_buttons_ignore_clicks() {
    let (mut ui, [play, options, _]) = menu();
    ui.update(&UiInput {
        pointer: Some(ui.rect(options).center()),
        ..Default::default()
    });
    assert_eq!(ui.focused(), Some(options));
    assert_eq!(ui.state(options), ButtonState::Focused);
    assert!(matches!(ui.events(), [UiEvent::Focus { name: Some(n), .. }] if n == "Options"));

    ui.set_enabled(play, false);
    assert_eq!(ui.state(play), ButtonState::Disabled);
    let on_play = ui.rect(play).center();
    click_at(&mut ui, on_play);
    assert!(
        !ui.events()
            .iter()
            .any(|e| matches!(e, UiEvent::Click { .. }))
    );
}

#[test]
fn accept_clicks_the_focused_button_and_cancel_is_reported() {
    let (mut ui, [_, options, _]) = menu();
    ui.set_focus(options);
    ui.update(&UiInput {
        accept_pressed: true,
        ..Default::default()
    });
    assert_eq!(ui.state(options), ButtonState::Pressed);
    ui.update(&UiInput {
        accept_released: true,
        ..Default::default()
    });
    assert_eq!(
        ui.events(),
        [UiEvent::Click {
            node: options,
            name: Some("Options".into())
        }]
    );
    ui.update(&UiInput {
        cancel: true,
        ..Default::default()
    });
    assert_eq!(ui.events(), [UiEvent::Cancel]);
    // a tap shorter than a frame
    ui.update(&UiInput {
        accept_pressed: true,
        accept_released: true,
        ..Default::default()
    });
    assert_eq!(ui.events().len(), 1);
}

#[test]
fn removing_a_focused_node_drops_focus() {
    let (mut ui, [play, options, _]) = menu();
    ui.set_focus(options);
    ui.remove(options);
    assert_eq!(ui.focused(), None);
    assert!(!ui.alive(options));
    assert_eq!(ui.find("Options"), None);
    laid(&mut ui);
    ui.navigate(Dir::Down);
    assert_eq!(ui.focused(), Some(play));
    ui.clear();
    assert_eq!(ui.find("Play"), None);
}

#[test]
fn nav_repeat_steps_then_repeats() {
    let mut r = NavRepeat::default();
    assert_eq!(r.update(Vec2::new(0.0, -1.0), 0.016), Some(Dir::Down));
    assert_eq!(r.update(Vec2::new(0.0, -1.0), 0.1), None);
    assert_eq!(r.update(Vec2::new(0.0, -1.0), 0.1), None);
    assert_eq!(r.update(Vec2::new(0.0, -1.0), 0.3), Some(Dir::Down)); // after 0.4 s
    assert_eq!(r.update(Vec2::new(0.0, -1.0), 0.12), Some(Dir::Down));
    assert_eq!(r.update(Vec2::ZERO, 0.016), None);
    assert_eq!(r.update(Vec2::new(0.3, 0.0), 0.016), None); // below threshold
    assert_eq!(r.update(Vec2::new(1.0, 0.0), 0.016), Some(Dir::Right));
}

#[test]
fn paint_emits_button_focus_ring_and_text() {
    let (mut ui, [play, ..]) = menu();
    ui.set_focus(play);
    let mut list = DrawList::default();
    ui.paint(&Theme::default(), &mut list);
    assert_eq!(list.texts.len(), 3);
    assert_eq!(list.texts[0].text, "Play");
    assert_eq!(list.texts[0].align, TextAlign::Center);
    // 3 buttons + 4 ring quads
    assert_eq!(list.quads.len(), 7);
    assert_eq!(list.order.len(), 10);
    let mut verts = Vec::new();
    list.vertices(&mut verts);
    assert_eq!(verts.len(), 7 * 6);
    assert_eq!(std::mem::size_of::<GuiVertex>(), 24);
    let (a, b) = (list.quads[0].rect.min, list.quads[0].rect.max);
    assert_eq!(verts[0].position, [a.x, a.y]);
    assert_eq!(verts[2].position, [b.x, b.y]);
}

#[test]
fn plugin_drives_the_ui_from_actions() {
    let mut app = App::new();
    app.add_plugin(olvin_input::InputPlugin {
        map: Some(InputMap::default()),
    });
    app.add_plugin(GuiPlugin);
    {
        let res = &mut app.resources;
        let mut gui = res.fetch_mut::<Gui>();
        gui.viewport = VIEW;
        gui.measure = Box::new(Fixed);
        let root = gui.ui.root();
        let col = gui
            .ui
            .add(root, Node::panel().column().gap(10.0).centered());
        gui.ui.add(col, Node::button("Play").name("play"));
        gui.ui.add(col, Node::button("Quit").name("quit"));
    }
    app.startup();
    // The plugin added UI bindings to the map.
    assert!(
        app.resources
            .fetch::<InputMap>()
            .actions
            .contains_key("UiAccept")
    );

    let mut cursor = EventsReader::default();
    app.update(); // first frame: layout
    // arrow down focuses the first button (a real gamepad poll would overwrite a hand-made GamepadState)
    app.resources
        .fetch_mut::<Input>()
        .set_key(KeyCode::ArrowDown, true);
    app.update();
    let focus = app.resources.fetch::<Gui>().ui.focused();
    assert_eq!(focus, app.resources.fetch::<Gui>().ui.find("play"));
    app.resources
        .fetch_mut::<Input>()
        .set_key(KeyCode::ArrowDown, false);

    // Enter confirms
    app.resources
        .fetch_mut::<Input>()
        .set_key(KeyCode::Enter, true);
    app.update();
    app.resources
        .fetch_mut::<Input>()
        .set_key(KeyCode::Enter, false);
    app.update();
    assert!(
        cursor
            .take(&app)
            .iter()
            .any(|e| matches!(e, UiEvent::Click { name: Some(n), .. } if n == "play"))
    );
    assert!(!app.resources.fetch::<Gui>().draw.quads.is_empty());

    // a mouse click on Quit
    let quit = app.resources.fetch::<Gui>().ui.find("quit").unwrap();
    let at = app.resources.fetch::<Gui>().ui.rect(quit).center();
    app.resources.fetch_mut::<Input>().cursor = at;
    app.resources
        .fetch_mut::<Input>()
        .set_button(MouseButton::Left, true);
    app.update();
    app.resources
        .fetch_mut::<Input>()
        .set_button(MouseButton::Left, false);
    app.update();
    assert!(
        cursor
            .take(&app)
            .iter()
            .any(|e| matches!(e, UiEvent::Click { name: Some(n), .. } if n == "quit"))
    );
}

#[derive(Default)]
struct EventsReader(olvin_core::EventCursor);
impl EventsReader {
    fn take(&mut self, app: &App) -> Vec<UiEvent> {
        let ev = app.resources.fetch::<Events<UiEvent>>();
        ev.read(&mut self.0).cloned().collect()
    }
}
