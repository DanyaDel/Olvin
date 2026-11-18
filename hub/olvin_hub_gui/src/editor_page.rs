//! The Editor page: the installation card, editor elements as cards with
//! install/remove actions (applied in one rebuild) and the build log.

use eframe::egui::{
    self, Align2, Id, Rangef, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2,
};

use crate::Hub;
use crate::chrome::{page_header, section};
use crate::ui::{self, FS_MD, FS_SM, Palette, RADIUS_LG, RADIUS_MD, anim, logo};
use crate::widgets::{self, Btn, Icon, Kind, bold, card_bg, mono, sans, truncated};

/// What an element card shows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ElementState {
    Builtin,
    Installed,
    Available,
    WillInstall,
    WillRemove,
}

impl Hub {
    pub(crate) fn editor_page(&mut self, ui: &mut Ui, _now: f64) {
        let ctx = ui.ctx().clone();
        let p = ui::palette(&ctx);
        page_header(
            ui,
            "Editor",
            "Installed separately. Its elements are built in with it.",
            |_| {},
        );
        self.stale_banner(ui);

        let Some(install) = &self.editor else {
            crate::projects::empty_state(
                ui,
                &p,
                "No data folder for the editor on this system.",
                "Open settings",
            )
            .then(|| self.page = crate::Page::Settings);
            return;
        };
        let built = install.is_built();
        let installed = install.installed(&self.catalog);
        let root = install.root().display().to_string();
        let building = self.build.is_some();
        let pending = self.editor_pending.clone();
        let wanted = pending.clone().unwrap_or_else(|| self.editor_selection());
        let dirty =
            pending.as_ref().is_some_and(|w| *w != installed) || (!built && pending.is_some());

        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                let (mut open, mut rebuild) = (false, false);
                // --- installation card ---
                let w = ui.available_width();
                let narrow = w < 560.0;
                let h = if narrow { 168.0 } else { 116.0 };
                let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
                let hover = anim::hover_t(
                    &ctx,
                    Id::new("install-card"),
                    ui.rect_contains_pointer(rect),
                );
                card_bg(ui.painter(), rect, RADIUS_LG, hover * 0.5, false, &p);
                let mut card = ui.new_child(
                    UiBuilder::new()
                        .max_rect(rect.shrink2(vec2(22.0, 20.0)))
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                card.horizontal(|ui| {
                    logo::mark(ui, 44.0);
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("Olvin Editor")
                                    .size(18.0)
                                    .family(ui::bold())
                                    .color(p.text),
                            );
                            ui.label(
                                egui::RichText::new(env!("CARGO_PKG_VERSION"))
                                    .family(egui::FontFamily::Monospace)
                                    .size(FS_SM)
                                    .color(p.muted),
                            );
                        });
                        let (text, color) = match (building, built) {
                            (true, _) => ("Building", p.warn),
                            (false, true) => ("Installed", p.accent),
                            (false, false) => ("Not installed", p.muted),
                        };
                        widgets::status(ui, text, color);
                    });
                    if !narrow {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            (open, rebuild) = install_buttons(ui, built, building, dirty);
                        });
                    }
                });
                card.add_space(10.0);
                let path_w = card.available_width();
                let g = truncated(card.painter(), &root, mono(FS_SM - 1.0), p.muted, path_w);
                let (r, resp) = card.allocate_exact_size(g.size(), Sense::hover());
                card.painter().galley(r.min, g, p.muted);
                resp.on_hover_text(&root);
                if narrow {
                    card.add_space(12.0);
                    card.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        (open, rebuild) = install_buttons(ui, built, building, dirty);
                    });
                }
                ui.add_space(20.0);

                // --- elements ---
                section(ui, "Elements");
                let mut toggle: Option<String> = None;
                let w = ui.available_width();
                let gap = 14.0;
                let cols = ((w + gap) / (280.0 + gap)).floor().clamp(1.0, 3.0) as usize;
                let card_w = (w - gap * (cols - 1) as f32) / cols as f32;
                let card_h = 150.0;
                let mut cards: Vec<(String, String, String, ElementState)> = vec![(
                    String::new(),
                    "Core".into(),
                    "Scene view, hierarchy, inspector and assets. Always installed.".into(),
                    ElementState::Builtin,
                )];
                for m in &self.catalog.editor_modules {
                    let has = built && installed.contains(&m.id);
                    let want = wanted.contains(&m.id);
                    let state = match (has, want, pending.is_some()) {
                        (true, false, true) => ElementState::WillRemove,
                        (false, true, true) => ElementState::WillInstall,
                        (true, _, _) => ElementState::Installed,
                        _ => ElementState::Available,
                    };
                    cards.push((m.id.clone(), m.name.clone(), m.description.clone(), state));
                }
                let page_at = self.page_at;
                for (line, chunk) in cards.chunks(cols).enumerate() {
                    let (strip, _) = ui.allocate_exact_size(vec2(w, card_h), Sense::hover());
                    for (k, (id, name, desc, state)) in chunk.iter().enumerate() {
                        let n = line * cols + k;
                        let appear = widgets::entrance(&ctx, page_at, n + 1);
                        let rect = Rect::from_min_size(
                            pos2(strip.min.x + k as f32 * (card_w + gap), strip.min.y),
                            vec2(card_w, card_h),
                        );
                        if element_card(ui, rect, n, name, desc, *state, appear, building, &p) {
                            toggle = Some(id.clone());
                        }
                    }
                    ui.add_space(gap);
                }

                if let Some(id) = toggle {
                    let mut next = wanted.clone();
                    if next.contains(&id) {
                        next.retain(|x| *x != id);
                    } else {
                        next.push(id);
                    }
                    // Catalog order keeps comparisons stable.
                    next = self
                        .catalog
                        .editor_modules
                        .iter()
                        .filter(|m| next.contains(&m.id))
                        .map(|m| m.id.clone())
                        .collect();
                    self.editor_pending = if built && next == installed {
                        None
                    } else {
                        Some(next)
                    };
                }

                // --- build log ---
                if building || !self.build_log.is_empty() {
                    ui.add_space(8.0);
                    section(ui, "Build log");
                    self.build_log_card(ui, building, &p);
                }
                ui.add_space(24.0);

                if open {
                    self.open_editor(None);
                }
                if rebuild {
                    let wanted = self
                        .editor_pending
                        .clone()
                        .unwrap_or_else(|| self.editor_selection());
                    self.start_build(&wanted);
                }
            });

        // Pending changes: a floating bar at the bottom of the page.
        if dirty && !building {
            self.pending_bar(ui, &p);
        }
    }

    fn pending_bar(&mut self, ui: &mut Ui, p: &Palette) {
        let ctx = ui.ctx().clone();
        let n = {
            let installed = self
                .editor
                .as_ref()
                .map(|e| e.installed(&self.catalog))
                .unwrap_or_default();
            let wanted = self.editor_pending.clone().unwrap_or_default();
            wanted.iter().filter(|w| !installed.contains(w)).count()
                + installed.iter().filter(|i| !wanted.contains(i)).count()
        };
        let t = anim::fade(&ctx, Id::new("pending-bar"), true);
        let area = ui.max_rect();
        let w = area.width().min(560.0);
        let rect = Rect::from_min_size(
            pos2(
                area.center().x - w / 2.0,
                area.max.y - 72.0 + 20.0 * (1.0 - t),
            ),
            vec2(w, 56.0),
        );
        let mut bar = ui.new_child(
            UiBuilder::new()
                .max_rect(rect.shrink2(vec2(16.0, 10.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        bar.multiply_opacity(t);
        let painter = bar.painter().clone();
        painter.add(
            egui::Shadow {
                offset: [0, 10],
                blur: 40,
                spread: 0,
                color: egui::Color32::from_black_alpha(160),
            }
            .as_shape(rect, RADIUS_LG),
        );
        painter.rect(
            rect,
            RADIUS_LG,
            p.surface_raised,
            Stroke::new(1.0, p.line_top),
            StrokeKind::Inside,
        );
        let label = if n == 1 {
            "1 change".to_owned()
        } else {
            format!("{n} changes")
        };
        bar.label(egui::RichText::new(label).family(ui::bold()).color(p.text));
        bar.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if ui
                .add(Btn::primary("Apply").icon(Icon::Refresh))
                .on_hover_text("Rebuild the editor with these elements")
                .clicked()
            {
                let wanted = self.editor_pending.clone().unwrap_or_default();
                self.start_build(&wanted);
            }
            if ui.add(Btn::secondary("Discard")).clicked() {
                self.editor_pending = None;
            }
        });
    }

    fn build_log_card(&mut self, ui: &mut Ui, building: bool, p: &Palette) {
        egui::Frame::new()
            .fill(p.bg.gamma_multiply(0.7))
            .stroke(Stroke::new(1.0, p.line))
            .corner_radius(RADIUS_MD)
            .inner_margin(egui::Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    if building {
                        ui.spinner();
                    }
                    let last = self
                        .build_log
                        .iter()
                        .rev()
                        .find(|l| !l.trim().is_empty())
                        .cloned()
                        .unwrap_or_else(|| "Starting cargo…".into());
                    let w = ui.available_width() - 110.0;
                    let g = truncated(
                        ui.painter(),
                        last.trim(),
                        mono(FS_SM),
                        p.text_2,
                        w.max(60.0),
                    );
                    let (r, _) = ui.allocate_exact_size(g.size(), Sense::hover());
                    ui.painter().galley(r.min, g, p.text_2);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let label = if self.log_open {
                            "Hide log"
                        } else {
                            "Show log"
                        };
                        if ui.add(Btn::new(Kind::Ghost, label).small()).clicked() {
                            self.log_open = !self.log_open;
                        }
                    });
                });
                if self.log_open {
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical()
                        .max_height(260.0)
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            for line in &self.build_log {
                                ui.label(
                                    egui::RichText::new(line)
                                        .family(egui::FontFamily::Monospace)
                                        .size(FS_SM)
                                        .color(p.text_2),
                                );
                            }
                        });
                }
            });
    }
}

/// Open / Install and Rebuild. Returns (open, rebuild). The primary button
/// is added first: rightmost in the wide card's right-to-left row, first
/// when stacked.
fn install_buttons(ui: &mut Ui, built: bool, building: bool, dirty: bool) -> (bool, bool) {
    let mut open = false;
    let rebuild;
    if built {
        // The pending bar owns the primary action while it's shown.
        let kind = if dirty {
            Kind::Secondary
        } else {
            Kind::Primary
        };
        open = ui
            .add_enabled(!building, Btn::new(kind, "Open").icon(Icon::Play))
            .on_hover_text("Open the editor without a project")
            .clicked();
        rebuild = ui
            .add_enabled(!building, Btn::secondary("Rebuild").icon(Icon::Refresh))
            .on_hover_text("Build the editor again from the current engine")
            .clicked();
    } else {
        rebuild = ui
            .add_enabled(!building, Btn::primary("Install").icon(Icon::Plus))
            .on_hover_text("Build the editor with the selected elements")
            .clicked();
    }
    (open, rebuild)
}

/// An element card. Returns true when its Install/Remove button was clicked.
#[allow(clippy::too_many_arguments)]
fn element_card(
    ui: &mut Ui,
    rect: Rect,
    index: usize,
    name: &str,
    desc: &str,
    state: ElementState,
    appear: f32,
    building: bool,
    p: &Palette,
) -> bool {
    let ctx = ui.ctx().clone();
    let id = Id::new(("element", index));
    let hover = anim::hover_t(&ctx, id.with("hover"), ui.rect_contains_pointer(rect));
    let lift = if ui::reduce_motion(&ctx) {
        0.0
    } else {
        -4.0 * hover
    };
    let r = rect.translate(vec2(0.0, 16.0 * (1.0 - appear) + lift));
    let mut painter = ui.painter().clone();
    painter.multiply_opacity(appear);
    let marked = matches!(state, ElementState::WillInstall | ElementState::WillRemove);
    card_bg(&painter, r, RADIUS_LG, hover, marked, p);

    let inner = r.shrink2(vec2(18.0, 16.0));
    let (status, color, fill) = match state {
        ElementState::Builtin => ("Built in", p.muted, false),
        ElementState::Installed => ("Installed", p.accent, false),
        ElementState::Available => ("Not installed", p.muted, false),
        ElementState::WillInstall => ("Will install", p.info, true),
        ElementState::WillRemove => ("Will remove", p.warn, true),
    };
    let chip_w = widgets::chip_width(&painter, status);
    let title = truncated(
        &painter,
        name,
        bold(FS_MD + 2.0),
        p.text,
        inner.width() - chip_w - 10.0,
    );
    painter.galley(inner.min, title, p.text);
    widgets::paint_chip(
        &painter,
        pos2(inner.max.x - chip_w, inner.min.y),
        status,
        color,
        fill,
    );

    let mut job =
        egui::text::LayoutJob::simple(desc.to_owned(), sans(FS_SM), p.text_2, inner.width());
    job.wrap.max_rows = 3;
    let g = painter.layout_job(job);
    painter.galley(pos2(inner.min.x, inner.min.y + 28.0), g, p.text_2);

    if state == ElementState::Builtin {
        painter.text(
            pos2(inner.min.x, inner.max.y - 12.0),
            Align2::LEFT_CENTER,
            "Scene · Hierarchy · Inspector · Assets",
            sans(FS_SM),
            p.muted,
        );
        return false;
    }
    let (label, icon) = match state {
        ElementState::Installed | ElementState::WillInstall => ("Remove", Icon::Trash),
        _ => ("Install", Icon::Plus),
    };
    let label = if marked { "Undo" } else { label };
    let button = Rect::from_x_y_ranges(
        Rangef::new(inner.max.x - 140.0, inner.max.x),
        Rangef::new(inner.max.y - 30.0, inner.max.y),
    );
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(button)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    child.multiply_opacity(appear);
    let b = Btn::secondary(label).small();
    let b = if marked { b } else { b.icon(icon) };
    child.add_enabled(!building, b).clicked()
}
