//! The Projects page: header, toolbar (search, sort, view) and the project
//! list in one of four layouts picked by width (wide table, medium table,
//! compact cards, grid).

mod parts;
mod rows;

use std::cmp::Reverse;
use std::time::{Duration, SystemTime};

use eframe::egui::{self, CursorIcon, Id, Rangef, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};

use crate::chrome::page_header;
use crate::ui::{self, FS_MD, Palette};
use crate::widgets::{self, Btn, Icon, paint_icon, pill_switch};
use crate::{Hub, Modal, RowAction, Sort, View};

use parts::*;
use rows::*;

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
