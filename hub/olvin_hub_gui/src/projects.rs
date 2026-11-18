//! The Projects page: header, toolbar (search, sort, view) and the project
//! list in one of four layouts picked by width (wide table, medium table,
//! compact cards, grid).

use std::cmp::Reverse;
use std::time::{Duration, SystemTime};

use eframe::egui::{
    self, Align2, Color32, CursorIcon, Id, Painter, Rangef, Rect, Sense, Stroke, StrokeKind, Ui,
    UiBuilder, Vec2, pos2, vec2,
};

use crate::chrome::page_header;
use crate::ui::{self, FS_MD, FS_SM, Palette, RADIUS_LG, RADIUS_MD, RADIUS_SM, anim};
use crate::widgets::{
    self, Btn, Icon, bold, card_bg, chip_line, mono, paint_icon, pill_switch, sans, truncated,
};
use crate::{Hub, Modal, Row, RowAction, Sort, View, ago};

/// Width-driven list layouts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    /// ≥ 900 px: name+path | modified | modules | actions.
    Wide,
    /// 600–900 px: narrower modules column that collapses into `+N`.
    Medium,
    /// < 600 px: one card per project, meta on a second line.
    Compact,
}

impl Mode {
    fn for_width(w: f32) -> Self {
        if w >= 900.0 {
            Self::Wide
        } else if w >= 600.0 {
            Self::Medium
        } else {
            Self::Compact
        }
    }
}

/// Horizontal column ranges shared by the table header and its rows, so
/// both always line up.
#[derive(Clone, Copy)]
struct Columns {
    name: Rangef,
    modified: Rangef,
    modules: Rangef,
    actions: Rangef,
}

const ROW_PAD: f32 = 16.0;
const COL_GAP: f32 = 20.0;

impl Columns {
    fn new(x: Rangef, mode: Mode) -> Self {
        let (min, max) = (x.min + ROW_PAD, x.max - ROW_PAD);
        let w = max - min;
        let (actions, modified, modules) = match mode {
            Mode::Wide => (108.0, 110.0, (w * 0.3).clamp(200.0, 340.0)),
            Mode::Medium => (32.0, 84.0, (w * 0.26).clamp(110.0, 190.0)),
            Mode::Compact => (32.0, 0.0, 0.0),
        };
        let actions = Rangef::new(max - actions, max);
        let modules = Rangef::new(actions.min - COL_GAP - modules, actions.min - COL_GAP);
        let modified = Rangef::new(modules.min - COL_GAP - modified, modules.min - COL_GAP);
        let name_end = if mode == Mode::Compact {
            actions.min - 12.0
        } else {
            modified.min - COL_GAP
        };
        Self {
            name: Rangef::new(min, name_end),
            modified,
            modules,
            actions,
        }
    }
}

impl Hub {
    pub(crate) fn projects_page(&mut self, ui: &mut Ui, now: f64) {
        let ctx = ui.ctx().clone();
        let p = ui::palette(&ctx);
        let count = self.rows.len();
        let subtitle = match count {
            0 => "Your games live here".to_owned(),
            1 => "1 project".to_owned(),
            n => format!("{n} projects"),
        };
        let mut new_project = false;
        let mut add = false;
        page_header(ui, "Projects", &subtitle, |ui| {
            // Right-to-left in the wide header (primary ends up rightmost),
            // left-to-right when stacked (primary first).
            new_project = ui
                .add(Btn::primary("New project").icon(Icon::Plus))
                .on_hover_text("Create a game project (Ctrl+N)")
                .clicked();
            add = ui
                .add(Btn::secondary("Add").icon(Icon::FolderPlus))
                .on_hover_text("Add an existing project from disk")
                .clicked();
        });
        if new_project {
            let modal = self.new_project_modal();
            self.open_modal(modal, now);
        }
        if add {
            self.open_modal(
                Modal::AddExisting {
                    path: String::new(),
                },
                now,
            );
        }
        self.stale_banner(ui);
        if count > 0 {
            self.toolbar(ui);
            ui.add_space(14.0);
        }

        let filter = self.search.trim().to_lowercase();
        let mut order: Vec<usize> = (0..self.rows.len())
            .filter(|&i| {
                filter.is_empty()
                    || self.rows[i].name.to_lowercase().contains(&filter)
                    || self.rows[i]
                        .root
                        .display()
                        .to_string()
                        .to_lowercase()
                        .contains(&filter)
            })
            .collect();
        match self.sort {
            Sort::Modified => order.sort_by_key(|&i| Reverse(self.rows[i].modified)),
            Sort::Name => order.sort_by_key(|&i| self.rows[i].name.to_lowercase()),
        }
        // The most recently touched project (within a week) gets the accent tile.
        let week = SystemTime::now() - Duration::from_secs(7 * 86_400);
        let recent = (0..self.rows.len())
            .filter(|&i| self.rows[i].modified.is_some_and(|m| m > week))
            .max_by_key(|&i| self.rows[i].modified);

        if count == 0 {
            if empty_state(
                ui,
                &p,
                "No projects yet. Create one (Ctrl+N).",
                "New project",
            ) {
                let modal = self.new_project_modal();
                self.open_modal(modal, now);
            }
            return;
        }
        if order.is_empty() {
            let msg = format!("No projects match “{}”.", self.search.trim());
            if empty_state(ui, &p, &msg, "Clear search") {
                self.search.clear();
            }
            return;
        }

        let width = ui.available_width();
        let mode = Mode::for_width(width);
        let running = self.game.is_some();
        let mut action: Option<(usize, RowAction)> = None;
        if self.view == View::List && mode != Mode::Compact {
            let cols = Columns::new(
                Rangef::new(ui.cursor().min.x, ui.cursor().min.x + width),
                mode,
            );
            if let Some(sort) = table_header(ui, cols, self.sort, &p) {
                self.sort = sort;
                self.save_prefs();
            }
        }
        let page_at = self.page_at;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.add_space(4.0);
                match self.view {
                    View::List => {
                        let cols = Columns::new(ui.max_rect().x_range(), mode);
                        for (n, &i) in order.iter().enumerate() {
                            let appear = widgets::entrance(&ctx, page_at, n);
                            let row = &self.rows[i];
                            if let Some(a) =
                                list_row(ui, row, i, mode, cols, appear, recent == Some(i), running)
                            {
                                action = Some((i, a));
                            }
                            ui.add_space(if mode == Mode::Compact { 10.0 } else { 8.0 });
                        }
                    }
                    View::Grid => {
                        let w = ui.available_width();
                        let gap = 16.0;
                        let columns = ((w + gap) / (250.0 + gap)).floor().max(1.0) as usize;
                        let card_w = (w - gap * (columns - 1) as f32) / columns as f32;
                        for (line, chunk) in order.chunks(columns).enumerate() {
                            let (strip, _) = ui.allocate_exact_size(vec2(w, 212.0), Sense::hover());
                            for (k, &i) in chunk.iter().enumerate() {
                                let n = line * columns + k;
                                let appear = widgets::entrance(&ctx, page_at, n);
                                let rect = Rect::from_min_size(
                                    pos2(strip.min.x + k as f32 * (card_w + gap), strip.min.y),
                                    vec2(card_w, 212.0),
                                );
                                let row = &self.rows[i];
                                if let Some(a) =
                                    grid_card(ui, rect, row, i, appear, recent == Some(i), running)
                                {
                                    action = Some((i, a));
                                }
                            }
                            ui.add_space(gap);
                        }
                    }
                }
                ui.add_space(24.0);
            });

        if let Some((i, a)) = action {
            let root = self.rows[i].root.clone();
            self.row_action(root, a, now);
        }
    }

    fn toolbar(&mut self, ui: &mut Ui) {
        let w = ui.available_width();
        let stacked = w < 560.0;
        let mut sort = self.sort;
        let mut view = self.view;
        let switches = |ui: &mut Ui, sort: &mut Sort, view: &mut View| {
            let a = pill_switch(
                ui,
                Id::new("sort-switch"),
                sort,
                &[(Sort::Modified, None, "Recent"), (Sort::Name, None, "Name")],
            );
            let b = pill_switch(
                ui,
                Id::new("view-switch"),
                view,
                &[
                    (View::List, Some(Icon::List), ""),
                    (View::Grid, Some(Icon::Grid), ""),
                ],
            );
            a || b
        };
        let changed = if stacked {
            search_field(ui, &mut self.search, w);
            ui.add_space(8.0);
            ui.horizontal(|ui| switches(ui, &mut sort, &mut view)).inner
        } else {
            ui.allocate_ui_with_layout(
                vec2(w, 36.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    search_field(ui, &mut self.search, (w - 260.0).clamp(180.0, 360.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Right-to-left: view first so it ends up rightmost.
                        let b = pill_switch(
                            ui,
                            Id::new("view-switch"),
                            &mut view,
                            &[
                                (View::List, Some(Icon::List), ""),
                                (View::Grid, Some(Icon::Grid), ""),
                            ],
                        );
                        let a = pill_switch(
                            ui,
                            Id::new("sort-switch"),
                            &mut sort,
                            &[(Sort::Modified, None, "Recent"), (Sort::Name, None, "Name")],
                        );
                        a || b
                    })
                    .inner
                },
            )
            .inner
        };
        if changed {
            if view != self.view {
                // Re-run the entrance for the new layout.
                self.page_at = ui.input(|i| i.time);
            }
            self.sort = sort;
            self.view = view;
            self.save_prefs();
        }
    }
}

fn search_field(ui: &mut Ui, text: &mut String, width: f32) {
    let p = ui::palette(ui.ctx());
    let response = ui.add(
        egui::TextEdit::singleline(text)
            .hint_text(egui::RichText::new("Search projects").color(p.muted))
            .desired_width(width - 40.0)
            .margin(egui::Margin {
                left: 32,
                right: 10,
                top: 8,
                bottom: 8,
            }),
    );
    let r = Rect::from_center_size(
        pos2(response.rect.min.x + 17.0, response.rect.center().y),
        Vec2::splat(15.0),
    );
    let color = if response.has_focus() {
        p.accent
    } else {
        p.muted
    };
    paint_icon(ui.painter(), r, Icon::Search, color);
}

/// One line of text and one action (STYLE.md §10). Returns true on click.
pub(crate) fn empty_state(ui: &mut Ui, p: &Palette, text: &str, action: &str) -> bool {
    ui.add_space(48.0);
    ui.vertical_centered(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
        ui.painter()
            .circle_stroke(r.center() - vec2(6.0, 0.0), 18.0, Stroke::new(6.0, p.ghost));
        ui.painter().rect_filled(
            Rect::from_min_size(pos2(r.center().x + 18.0, r.min.y + 14.0), vec2(6.0, 36.0)),
            0.0,
            p.accent.gamma_multiply(0.6),
        );
        ui.add_space(14.0);
        ui.label(egui::RichText::new(text).size(FS_MD + 2.0).color(p.text_2));
        ui.add_space(12.0);
        ui.add(Btn::secondary(action)).clicked()
    })
    .inner
}

/// Caps headers over the table columns; NAME and MODIFIED sort on click.
fn table_header(ui: &mut Ui, cols: Columns, sort: Sort, p: &Palette) -> Option<Sort> {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    let painter = ui.painter();
    let mut picked = None;
    for (label, range, key) in [
        ("Name", cols.name, Some(Sort::Name)),
        ("Modified", cols.modified, Some(Sort::Modified)),
        ("Modules", cols.modules, None),
    ] {
        // Text starts where the row's text starts (after the tile in the name column).
        let x = if key == Some(Sort::Name) {
            range.min + 54.0
        } else {
            range.min
        };
        let active = key == Some(sort);
        let g = widgets::caps_galley(painter, label, if active { p.text } else { p.muted });
        let r = Rect::from_min_size(pos2(x, rect.center().y - g.size().y / 2.0), g.size());
        if let Some(key) = key {
            let resp = ui
                .interact(r.expand(4.0), Id::new(("col", label)), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text(format!("Sort by {}", label.to_lowercase()));
            if resp.clicked() {
                picked = Some(key);
            }
        }
        painter.galley(r.min, g, p.muted);
        if active {
            // Small down arrow: newest / A first.
            let c = pos2(r.max.x + 8.0, r.center().y);
            painter.add(egui::Shape::convex_polygon(
                vec![
                    c + vec2(-3.5, -2.0),
                    c + vec2(3.5, -2.0),
                    c + vec2(0.0, 2.5),
                ],
                p.accent,
                Stroke::NONE,
            ));
        }
    }
    painter.hline(rect.x_range(), rect.max.y + 3.0, Stroke::new(1.0, p.line));
    ui.add_space(8.0);
    picked
}

/// Letter tile: accent when it's the most recent project.
fn paint_tile(painter: &Painter, rect: Rect, row: &Row, recent: bool, p: &Palette, font: f32) {
    let letter = row
        .name
        .chars()
        .next()
        .unwrap_or('?')
        .to_uppercase()
        .to_string();
    let radius = (rect.height() * 0.26) as u8;
    if row.missing {
        painter.rect(
            rect,
            radius,
            p.glass,
            Stroke::new(1.0, p.line),
            StrokeKind::Inside,
        );
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            letter,
            bold(font),
            p.faint,
        );
        return;
    }
    if recent {
        let glow = egui::Shadow {
            offset: [0, 0],
            blur: 15,
            spread: 0,
            color: p.accent_glow.gamma_multiply(0.6),
        };
        painter.add(glow.as_shape(rect, radius));
    }
    let fill = anim::lerp_color(p.surface_raised, p.text, 0.06);
    painter.rect(
        rect,
        radius,
        fill,
        Stroke::new(1.0, if recent { p.accent_border } else { p.line }),
        StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        letter,
        bold(font),
        if recent { p.accent } else { p.text },
    );
}

/// Name and path lines; the path is mono, cut with `…`, with the full path
/// on hover. Returns the path's rect.
fn paint_name_path(
    painter: &Painter,
    x: Rangef,
    name_y: f32,
    path_y: f32,
    row: &Row,
    p: &Palette,
) -> Rect {
    let name = truncated(painter, &row.name, bold(FS_MD + 2.0), p.text, x.span());
    painter.galley(pos2(x.min, name_y - name.size().y / 2.0), name, p.text);
    let path = row.root.display().to_string();
    let (text, color) = if row.missing {
        (format!("Not found · {path}"), p.error)
    } else {
        (path, p.muted)
    };
    let g = truncated(painter, &text, mono(FS_SM - 1.0), color, x.span());
    let r = Rect::from_min_size(pos2(x.min, path_y - g.size().y / 2.0), g.size());
    painter.galley(r.min, g, color);
    r
}

fn modified_text(row: &Row) -> String {
    row.modified.map(ago).unwrap_or_else(|| "—".into())
}

/// The ⋯ menu and (wide rows) the Open button, in a child ui so the
/// row's entrance fade applies to them.
fn row_actions(
    ui: &mut Ui,
    rect: Rect,
    row: &Row,
    running: bool,
    appear: f32,
    with_open: bool,
) -> (Option<RowAction>, bool) {
    let mut action = None;
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    child.multiply_opacity(appear);
    child.spacing_mut().item_spacing.x = 6.0;
    let more = child
        .add(Btn::icon_only(Icon::More))
        .on_hover_text("More actions");
    let menu_open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&more));
    egui::Popup::menu(&more).show(|ui| {
        ui.set_min_width(200.0);
        let items = [
            (
                Icon::Pencil,
                "Open in editor",
                RowAction::Open,
                !row.missing,
            ),
            (Icon::Play, "Run", RowAction::Run, !running && !row.missing),
            (
                Icon::Play,
                "Run release build",
                RowAction::Release,
                !running && !row.missing,
            ),
            (Icon::Package, "Modules…", RowAction::Modules, !row.missing),
            (
                Icon::Folder,
                "Show in folder",
                RowAction::Reveal,
                !row.missing,
            ),
        ];
        for (icon, label, a, enabled) in items {
            if ui.add_enabled(enabled, MenuItem(icon, label)).clicked() {
                action = Some(a);
            }
        }
        ui.separator();
        if ui
            .add(MenuItem(Icon::Trash, "Remove from list"))
            .on_hover_text("Files stay on disk")
            .clicked()
        {
            action = Some(RowAction::Forget);
        }
    });
    if with_open
        && child
            .add_enabled(!row.missing, Btn::secondary("Open").small())
            .on_hover_text("Open in the editor")
            .clicked()
    {
        action = Some(RowAction::Open);
    }
    (action, menu_open)
}

/// A popup menu row: icon and label, full width, hover fill.
struct MenuItem<'a>(Icon, &'a str);

impl egui::Widget for MenuItem<'_> {
    fn ui(self, ui: &mut Ui) -> egui::Response {
        let p = ui::palette(ui.ctx());
        let w = ui.available_width().max(180.0);
        let (rect, response) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
        let enabled = ui.is_enabled();
        let hover = anim::hover_t(
            ui.ctx(),
            response.id.with("h"),
            response.hovered() && enabled,
        );
        ui.painter().rect_filled(
            rect,
            RADIUS_SM,
            anim::lerp_color(Color32::TRANSPARENT, p.line, hover),
        );
        let color = if enabled {
            anim::lerp_color(p.text_2, p.text, hover)
        } else {
            p.faint
        };
        paint_icon(
            ui.painter(),
            Rect::from_center_size(pos2(rect.min.x + 16.0, rect.center().y), Vec2::splat(15.0)),
            self.0,
            color,
        );
        ui.painter().text(
            pos2(rect.min.x + 32.0, rect.center().y),
            Align2::LEFT_CENTER,
            self.1,
            sans(FS_MD),
            color,
        );
        if enabled {
            response.on_hover_cursor(CursorIcon::PointingHand)
        } else {
            response
        }
    }
}

/// Hover lift (STYLE.md §6: translateY(-4px), stronger shadow), none with reduce motion.
fn lift(ctx: &egui::Context, hover: f32, px: f32) -> f32 {
    if ui::reduce_motion(ctx) {
        0.0
    } else {
        -px * hover
    }
}

/// One project in list view (table row or compact card). A click anywhere
/// outside the buttons opens the project.
#[allow(clippy::too_many_arguments)]
fn list_row(
    ui: &mut Ui,
    row: &Row,
    index: usize,
    mode: Mode,
    cols: Columns,
    appear: f32,
    recent: bool,
    running: bool,
) -> Option<RowAction> {
    let ctx = ui.ctx().clone();
    let p = ui::palette(&ctx);
    let height = if mode == Mode::Compact { 76.0 } else { 66.0 };
    let w = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(w, height), Sense::click());
    let id = Id::new(("project", index));
    if !ui.is_rect_visible(rect.expand(30.0)) {
        return None;
    }
    let hovered = response.contains_pointer();
    let hover = anim::hover_t(&ctx, id.with("hover"), hovered);
    let dy = 16.0 * (1.0 - appear) + lift(&ctx, hover, 3.0);
    let r = rect.translate(vec2(0.0, dy));
    let mut painter = ui.painter().clone();
    painter.multiply_opacity(appear);
    card_bg(&painter, r, RADIUS_MD, hover, false, &p);

    let tile_size = if mode == Mode::Compact { 44.0 } else { 40.0 };
    let tile = Rect::from_min_size(
        pos2(r.min.x + ROW_PAD, r.center().y - tile_size / 2.0),
        Vec2::splat(tile_size),
    );
    paint_tile(&painter, tile, row, recent, &p, 18.0);
    let text_x = Rangef::new(tile.max.x + 14.0, cols.name.max);
    let path_rect = if mode == Mode::Compact {
        let path = paint_name_path(
            &painter,
            text_x,
            r.center().y - 18.0,
            r.center().y + 1.0,
            row,
            &p,
        );
        let meta = match row.modules.len() {
            0 if row.missing => String::new(),
            0 => format!("{} · no modules", modified_text(row)),
            1 => format!("{} · 1 module", modified_text(row)),
            n => format!("{} · {n} modules", modified_text(row)),
        };
        let g = truncated(&painter, &meta, sans(FS_SM), p.text_2, text_x.span());
        painter.galley(pos2(text_x.min, r.center().y + 12.0), g, p.text_2);
        path
    } else {
        let path = paint_name_path(
            &painter,
            text_x,
            r.center().y - 10.0,
            r.center().y + 11.0,
            row,
            &p,
        );
        let g = truncated(
            &painter,
            &modified_text(row),
            sans(FS_MD),
            p.text_2,
            cols.modified.span(),
        );
        painter.galley(
            pos2(cols.modified.min, r.center().y - g.size().y / 2.0),
            g,
            p.text_2,
        );
        let modules = Rect::from_x_y_ranges(cols.modules, r.y_range());
        if row.modules.is_empty() && !row.missing {
            painter.text(
                pos2(modules.min.x, modules.center().y),
                Align2::LEFT_CENTER,
                "—",
                sans(FS_MD),
                p.faint,
            );
        } else {
            chip_line(&painter, modules, &row.modules, &p);
        }
        path
    };

    let actions = Rect::from_x_y_ranges(cols.actions, r.y_range());
    let (action, menu_open) = row_actions(ui, actions, row, running, appear, mode == Mode::Wide);
    if menu_open {
        ctx.request_repaint();
    }
    finish_row(&ctx, response, row, path_rect, action)
}

/// Tooltip for the path and the open-on-click behaviour shared by rows and cards.
fn finish_row(
    ctx: &egui::Context,
    response: egui::Response,
    row: &Row,
    path_rect: Rect,
    action: Option<RowAction>,
) -> Option<RowAction> {
    let over_path = ctx
        .pointer_hover_pos()
        .is_some_and(|pos| path_rect.contains(pos));
    let response = if over_path {
        response.on_hover_text(row.root.display().to_string())
    } else if !row.missing {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    };
    if action.is_none() && response.clicked() && !row.missing {
        return Some(RowAction::Open);
    }
    action
}

/// One project in grid view: preview strip with the letter, name, path,
/// modules and modified time.
fn grid_card(
    ui: &mut Ui,
    rect: Rect,
    row: &Row,
    index: usize,
    appear: f32,
    recent: bool,
    running: bool,
) -> Option<RowAction> {
    let ctx = ui.ctx().clone();
    let p = ui::palette(&ctx);
    let id = Id::new(("project-card", index));
    let response = ui.interact(rect, id, Sense::click());
    if !ui.is_rect_visible(rect.expand(30.0)) {
        return None;
    }
    let hover = anim::hover_t(&ctx, id.with("hover"), response.contains_pointer());
    let dy = 18.0 * (1.0 - appear) + lift(&ctx, hover, 4.0);
    let r = rect.translate(vec2(0.0, dy));
    let mut painter = ui.painter().clone();
    painter.multiply_opacity(appear);
    card_bg(&painter, r, RADIUS_LG, hover, false, &p);

    // Preview strip: 16:9-ish band with a soft vertical gradient.
    let preview = Rect::from_min_max(r.min + vec2(8.0, 8.0), pos2(r.max.x - 8.0, r.min.y + 96.0));
    let top = anim::lerp_color(p.surface_raised, p.text, 0.05);
    let bottom = anim::lerp_color(p.surface_raised, p.bg, 0.5);
    let mut mesh = egui::epaint::Mesh::default();
    for (pos, c) in [
        (preview.left_top(), top),
        (preview.right_top(), top),
        (preview.right_bottom(), bottom),
        (preview.left_bottom(), bottom),
    ] {
        mesh.colored_vertex(pos, c);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.rect_filled(preview, RADIUS_MD, bottom);
    painter
        .with_clip_rect(preview.shrink(1.0))
        .add(egui::Shape::mesh(mesh));
    painter.rect_stroke(
        preview,
        RADIUS_MD,
        Stroke::new(1.0, p.line_soft),
        StrokeKind::Inside,
    );
    let tile = Rect::from_center_size(preview.center(), Vec2::splat(48.0));
    paint_tile(&painter, tile, row, recent, &p, 22.0);

    let x = Rangef::new(r.min.x + 16.0, r.max.x - 16.0);
    let path_rect = paint_name_path(
        &painter,
        x,
        preview.max.y + 20.0,
        preview.max.y + 40.0,
        row,
        &p,
    );
    let chips = Rect::from_x_y_ranges(x, Rangef::new(preview.max.y + 52.0, preview.max.y + 76.0));
    if row.modules.is_empty() {
        painter.text(
            pos2(x.min, chips.center().y),
            Align2::LEFT_CENTER,
            if row.missing { "" } else { "No modules" },
            sans(FS_SM),
            p.muted,
        );
    } else {
        chip_line(&painter, chips, &row.modules, &p);
    }
    let foot_y = r.max.y - 22.0;
    painter.text(
        pos2(x.min, foot_y),
        Align2::LEFT_CENTER,
        modified_text(row),
        sans(FS_SM),
        p.text_2,
    );

    let actions = Rect::from_min_max(
        pos2(x.max - 80.0, foot_y - 16.0),
        pos2(x.max + 6.0, foot_y + 16.0),
    );
    let (action, menu_open) = row_actions(ui, actions, row, running, appear, false);
    if menu_open {
        ctx.request_repaint();
    }
    finish_row(&ctx, response, row, path_rect, action)
}
