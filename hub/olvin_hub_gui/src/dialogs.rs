//! Modal dialogs: New project, Add project, Modules. Each pops in with
//! `--ease-pop` on a glass-ish surface with `--radius-lg`.

use eframe::egui::{
    self, Align2, Color32, CursorIcon, Id, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};
use olvin_hub::{Catalog, Project, name_problem};

use crate::ui::{self, FS_MD, FS_SM, FS_XL, Palette, RADIUS_LG, RADIUS_MD, anim};
use crate::widgets::{Btn, Icon, bold, card_bg, mono, paint_icon, sans, since, truncated};
use crate::{Hub, Modal, TEMPLATES};

/// Shows `content` in a modal that pops in. Returns (content result, should close).
fn dialog<R>(
    ctx: &egui::Context,
    id: &str,
    opened_at: f64,
    width: f32,
    content: impl FnOnce(&mut Ui) -> R,
) -> (R, bool) {
    let p = ui::palette(ctx);
    let t = since(ctx, opened_at, 0.32, |t| t);
    let pop = anim::ease_pop(t);
    let fade = anim::ease_out((t * 1.6).min(1.0));
    let id = Id::new(id);
    let area =
        egui::Modal::default_area(id).anchor(Align2::CENTER_CENTER, vec2(0.0, 22.0 * (1.0 - pop)));
    let response = egui::Modal::new(id)
        .area(area)
        .frame(egui::Frame::NONE)
        .backdrop_color(Color32::from_black_alpha((150.0 * fade) as u8))
        .show(ctx, |ui| {
            ui.multiply_opacity(fade);
            let dark = p.dark;
            let fill = if dark {
                // Glass-strong over the backdrop, but opaque enough to read (STYLE.md §4).
                Color32::from_rgba_unmultiplied(
                    p.surface_raised.r(),
                    p.surface_raised.g(),
                    p.surface_raised.b(),
                    242,
                )
            } else {
                p.surface_raised
            };
            let inner = egui::Frame::new()
                .fill(fill)
                .stroke(Stroke::new(1.0, p.line))
                .corner_radius(RADIUS_LG)
                .shadow(egui::Shadow {
                    offset: [0, 10],
                    blur: 40,
                    spread: 0,
                    color: Color32::from_black_alpha(if dark { 204 } else { 60 }),
                })
                .inner_margin(if width < 420.0 { 18 } else { 24 })
                .show(ui, |ui| {
                    ui.set_width(width);
                    content(ui)
                });
            // Light top edge of glass panels.
            let r = inner.response.rect;
            ui.painter().hline(
                (r.min.x + 20.0)..=(r.max.x - 20.0),
                r.min.y + 0.5,
                Stroke::new(1.0, p.line_top),
            );
            inner.inner
        });
    let close = response.should_close();
    (response.inner, close)
}

/// Title, subtitle and a close button. Returns true when close was clicked.
fn title(ui: &mut Ui, title: &str, subtitle: &str) -> bool {
    let p = ui::palette(ui.ctx());
    let w = ui.available_width();
    let mut close = false;
    ui.allocate_ui_with_layout(
        vec2(w, 30.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.label(
                egui::RichText::new(title)
                    .size(FS_XL)
                    .family(ui::bold())
                    .color(p.text),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                close = ui
                    .add(Btn::icon_only(Icon::Close))
                    .on_hover_text("Close (Esc)")
                    .clicked();
            });
        },
    );
    if !subtitle.is_empty() {
        ui.label(egui::RichText::new(subtitle).color(p.muted));
    }
    ui.add_space(12.0);
    close
}

fn caps(ui: &mut Ui, text: &str) {
    let p = ui::palette(ui.ctx());
    ui.add_space(4.0);
    ui.label(ui::caps(ui.ctx(), text).color(p.muted));
    ui.add_space(2.0);
}

/// Dialog buttons, right-aligned. The row is bounded to one line: a bare
/// right-to-left layout in a modal would claim the whole screen height.
fn buttons_row(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let w = ui.available_width();
    ui.allocate_ui_with_layout(
        vec2(w, 36.0),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            add(ui);
        },
    );
}

/// A toggle card: title, a muted second line and a check circle. Returns
/// the click response (tooltip is up to the caller).
fn toggle_card(
    ui: &mut Ui,
    rect: Rect,
    id: Id,
    title: &str,
    sub: &str,
    on: bool,
    p: &Palette,
) -> egui::Response {
    let ctx = ui.ctx().clone();
    let response = ui
        .interact(rect, id, Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    let hover = anim::hover_t(&ctx, id.with("h"), response.hovered());
    let sel = anim::fade(&ctx, id.with("on"), on);
    let painter = ui.painter();
    card_bg(painter, rect, RADIUS_MD, hover * 0.7, false, p);
    if sel > 0.0 {
        painter.rect_filled(rect, RADIUS_MD, p.accent.gamma_multiply(0.07 * sel));
        painter.rect_stroke(
            rect,
            RADIUS_MD,
            Stroke::new(1.0, p.accent_border.gamma_multiply(0.5 + 0.5 * sel)),
            StrokeKind::Inside,
        );
    }
    if response.has_focus() {
        painter.rect_stroke(
            rect.expand(2.0),
            RADIUS_MD + 2,
            Stroke::new(2.0, p.accent),
            StrokeKind::Outside,
        );
    }
    let check = pos2(rect.max.x - 20.0, rect.min.y + 20.0);
    painter.circle_stroke(
        check,
        8.0,
        Stroke::new(1.0, anim::lerp_color(p.line_top, p.accent, sel)),
    );
    if sel > 0.0 {
        painter.circle_filled(check, 8.0 * (0.6 + 0.4 * sel), p.accent.gamma_multiply(sel));
        paint_icon(
            painter,
            Rect::from_center_size(check, Vec2::splat(12.0)),
            Icon::Check,
            p.on_accent.gamma_multiply(sel),
        );
    }
    let text_w = rect.width() - 52.0;
    let g = truncated(painter, title, bold(FS_MD), p.text, text_w);
    painter.galley(pos2(rect.min.x + 14.0, rect.min.y + 11.0), g, p.text);
    if !sub.is_empty() {
        let g = truncated(
            painter,
            sub,
            sans(FS_SM - 1.0),
            p.muted,
            rect.width() - 28.0,
        );
        painter.galley(pos2(rect.min.x + 14.0, rect.min.y + 31.0), g, p.muted);
    }
    response
}

/// Cards in a grid of `min_w`-wide columns; `card(ui, rect, index)` paints one.
fn card_grid(
    ui: &mut Ui,
    count: usize,
    min_w: f32,
    h: f32,
    mut card: impl FnMut(&mut Ui, Rect, usize),
) {
    let w = ui.available_width();
    let gap = 10.0;
    let cols = ((w + gap) / (min_w + gap)).floor().max(1.0) as usize;
    let card_w = (w - gap * (cols - 1) as f32) / cols as f32;
    for start in (0..count).step_by(cols) {
        let (strip, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
        for k in 0..cols.min(count - start) {
            let rect = Rect::from_min_size(
                pos2(strip.min.x + k as f32 * (card_w + gap), strip.min.y),
                vec2(card_w, h),
            );
            card(ui, rect, start + k);
        }
        if start + cols < count {
            ui.add_space(gap);
        }
    }
}

/// Adds `id` with what it requires, or removes it with what requires it.
fn toggle_module(catalog: &Catalog, modules: &mut Vec<String>, id: &str) {
    if modules.iter().any(|m| m == id) {
        let mut gone = vec![id.to_owned()];
        while let Some(next) = catalog.modules.iter().find(|m| {
            modules.contains(&m.id)
                && !gone.contains(&m.id)
                && m.requires.iter().any(|r| gone.contains(r))
        }) {
            gone.push(next.id.clone());
        }
        modules.retain(|m| !gone.contains(m));
    } else if let Ok(chain) = catalog.resolve(id) {
        for m in chain {
            if !modules.contains(&m.id) {
                modules.push(m.id.clone());
            }
        }
    }
    // Catalog order keeps the list stable.
    let order: Vec<String> = catalog
        .modules
        .iter()
        .filter(|m| modules.contains(&m.id))
        .map(|m| m.id.clone())
        .collect();
    *modules = order;
}

impl Hub {
    pub(crate) fn modals(&mut self, ctx: &egui::Context, _now: f64) {
        let Some(mut modal) = self.modal.take() else {
            return;
        };
        let p = ui::palette(ctx);
        let screen = ctx.content_rect();
        let mut keep = true;
        match &mut modal {
            Modal::NewProject {
                template,
                name,
                location,
                modules,
            } => {
                let width = (screen.width() - 80.0).clamp(280.0, 680.0);
                let mut create = false;
                let catalog = &self.catalog;
                let ((), close) = dialog(ctx, "new-project", self.modal_at, width, |ui| {
                    if title(
                        ui,
                        "New project",
                        "Pick a template, then adjust the modules.",
                    ) {
                        keep = false;
                    }
                    caps(ui, "Template");
                    let narrow = width < 520.0;
                    let (min_w, h) = if narrow { (width, 52.0) } else { (180.0, 92.0) };
                    card_grid(ui, TEMPLATES.len(), min_w, h, |ui, rect, i| {
                        let t = &TEMPLATES[i];
                        let id = Id::new(("template", i));
                        let response = ui
                            .interact(rect, id, Sense::click())
                            .on_hover_cursor(CursorIcon::PointingHand)
                            .on_hover_text(t.description);
                        let hover = anim::hover_t(ui.ctx(), id.with("h"), response.hovered());
                        let on = *template == i;
                        let sel = anim::fade(ui.ctx(), id.with("on"), on);
                        let painter = ui.painter();
                        card_bg(painter, rect, RADIUS_MD, hover * 0.7, on, &p);
                        if sel > 0.0 {
                            painter.add(
                                egui::Shadow {
                                    offset: [0, 0],
                                    blur: 15,
                                    spread: 0,
                                    color: p.accent_glow.gamma_multiply(0.5 * sel),
                                }
                                .as_shape(rect, RADIUS_MD),
                            );
                            painter.rect_stroke(
                                rect,
                                RADIUS_MD,
                                Stroke::new(1.5, p.accent.gamma_multiply(sel)),
                                StrokeKind::Inside,
                            );
                        }
                        painter.text(
                            rect.min + vec2(14.0, 12.0),
                            Align2::LEFT_TOP,
                            t.name,
                            bold(FS_MD + 1.0),
                            if on { p.text } else { p.text_2 },
                        );
                        let mut job = egui::text::LayoutJob::simple(
                            t.description.to_owned(),
                            sans(FS_SM - 1.0),
                            p.muted,
                            rect.width() - 28.0,
                        );
                        job.wrap.max_rows = if narrow { 1 } else { 3 };
                        painter.galley(
                            rect.min + vec2(14.0, 33.0),
                            painter.layout_job(job),
                            p.muted,
                        );
                        if response.clicked() && !on {
                            *template = i;
                            *modules = t.modules.iter().map(|m| m.to_string()).collect();
                        }
                    });
                    ui.add_space(10.0);

                    caps(ui, &format!("Modules · {} selected", modules.len()));
                    let max_h = (screen.height() - 600.0).clamp(104.0, 236.0);
                    egui::ScrollArea::vertical()
                        .id_salt("new-project-modules")
                        .max_height(max_h)
                        .show(ui, |ui| {
                            let mut flip = None;
                            card_grid(ui, catalog.modules.len(), 190.0, 52.0, |ui, rect, i| {
                                let m = &catalog.modules[i];
                                let on = modules.contains(&m.id);
                                let mut tip = m.description.clone();
                                if !m.requires.is_empty() {
                                    tip.push_str(&format!("\nRequires: {}", m.requires.join(", ")));
                                }
                                if toggle_card(
                                    ui,
                                    rect,
                                    Id::new(("np-module", i)),
                                    &m.name,
                                    &m.category,
                                    on,
                                    &p,
                                )
                                .on_hover_text(tip)
                                .clicked()
                                {
                                    flip = Some(m.id.clone());
                                }
                            });
                            if let Some(id) = flip {
                                toggle_module(catalog, modules, &id);
                            }
                        });
                    ui.add_space(10.0);

                    caps(ui, "Name");
                    let name_edit = ui.add(
                        egui::TextEdit::singleline(name)
                            .hint_text("my_game")
                            .desired_width(f32::INFINITY)
                            .margin(egui::Margin::symmetric(10, 8)),
                    );
                    if ui.memory(|m| m.focused().is_none()) {
                        name_edit.request_focus();
                    }
                    caps(ui, "Location");
                    ui.add(
                        egui::TextEdit::singleline(location)
                            .desired_width(f32::INFINITY)
                            .font(egui::TextStyle::Monospace)
                            .margin(egui::Margin::symmetric(10, 8)),
                    );
                    ui.add_space(4.0);
                    let error = name_problem(name);
                    let line = match &error {
                        Some(e) => (e.clone(), p.error, sans(FS_SM)),
                        None if name.trim().is_empty() => (String::new(), p.muted, sans(FS_SM)),
                        None => (
                            format!(
                                "→ {}/{}",
                                location.trim().trim_end_matches('/'),
                                name.trim()
                            ),
                            p.muted,
                            mono(FS_SM - 1.0),
                        ),
                    };
                    let g = truncated(ui.painter(), &line.0, line.2, line.1, ui.available_width());
                    let (r, _) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
                    ui.painter()
                        .galley(pos2(r.min.x, r.center().y - g.size().y / 2.0), g, line.1);
                    ui.add_space(12.0);

                    let valid =
                        !name.trim().is_empty() && error.is_none() && !location.trim().is_empty();
                    let enter = valid && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    buttons_row(ui, |ui| {
                        create = ui
                            .add_enabled(valid, Btn::primary("Create project").icon(Icon::Plus))
                            .clicked()
                            || enter;
                        if ui.add(Btn::secondary("Cancel")).clicked() {
                            keep = false;
                        }
                    });
                });
                if close {
                    keep = false;
                }
                if create {
                    let (name, location, modules) =
                        (name.clone(), location.clone(), modules.clone());
                    if self.create_project(&name, &location, &modules) {
                        keep = false;
                    }
                }
            }
            Modal::AddExisting { path } => {
                let width = (screen.width() - 80.0).clamp(280.0, 500.0);
                let mut add = false;
                let ((), close) = dialog(ctx, "add-existing", self.modal_at, width, |ui| {
                    if title(
                        ui,
                        "Add project",
                        "Pick the folder with the game's Cargo.toml.",
                    ) {
                        keep = false;
                    }
                    caps(ui, "Folder");
                    let edit = ui.add(
                        egui::TextEdit::singleline(path)
                            .hint_text("/home/…/my_game")
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY)
                            .margin(egui::Margin::symmetric(10, 8)),
                    );
                    if ui.memory(|m| m.focused().is_none()) {
                        edit.request_focus();
                    }
                    ui.add_space(16.0);
                    let valid = !path.trim().is_empty();
                    let enter = valid && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    buttons_row(ui, |ui| {
                        add = ui
                            .add_enabled(valid, Btn::primary("Add").icon(Icon::FolderPlus))
                            .clicked()
                            || enter;
                        if ui.add(Btn::secondary("Cancel")).clicked() {
                            keep = false;
                        }
                    });
                });
                if close {
                    keep = false;
                }
                if add {
                    let path = path.clone();
                    if self.add_existing(&path) {
                        keep = false;
                    }
                }
            }
            Modal::Modules { root } => {
                let Ok(mut project) = Project::open(&*root) else {
                    self.toast_error(format!(
                        "Could not open {}. Check that the folder still exists.",
                        root.display()
                    ));
                    return;
                };
                let width = (screen.width() - 80.0).clamp(280.0, 720.0);
                let mut toggled = None;
                let catalog = &self.catalog;
                let subtitle = format!(
                    "{} · Changes go straight to its Cargo.toml.",
                    project.name()
                );
                let ((), close) = dialog(ctx, "modules", self.modal_at, width, |ui| {
                    if title(ui, "Modules", &subtitle) {
                        keep = false;
                    }
                    let max_h = (screen.height() - 260.0).clamp(160.0, 520.0);
                    egui::ScrollArea::vertical()
                        .id_salt("project-modules")
                        .max_height(max_h)
                        .show(ui, |ui| {
                            for category in catalog.categories() {
                                caps(ui, category);
                                let list: Vec<_> = catalog
                                    .modules
                                    .iter()
                                    .filter(|m| m.category == category)
                                    .collect();
                                card_grid(ui, list.len(), 210.0, 56.0, |ui, rect, i| {
                                    let m = list[i];
                                    let on = project.is_installed(m);
                                    let mut tip = m.description.clone();
                                    if !m.requires.is_empty() {
                                        tip.push_str(&format!(
                                            "\nRequires: {}",
                                            m.requires.join(", ")
                                        ));
                                    }
                                    if toggle_card(
                                        ui,
                                        rect,
                                        Id::new(("module", &m.id)),
                                        &m.name,
                                        &m.description,
                                        on,
                                        &p,
                                    )
                                    .on_hover_text(tip)
                                    .clicked()
                                    {
                                        toggled = Some((m.id.clone(), !on));
                                    }
                                });
                                ui.add_space(10.0);
                            }
                        });
                    ui.add_space(12.0);
                    buttons_row(ui, |ui| {
                        if ui.add(Btn::primary("Done")).clicked() {
                            keep = false;
                        }
                    });
                });
                if close {
                    keep = false;
                }
                if let Some((id, on)) = toggled {
                    let result = if on {
                        project
                            .add(&self.catalog, &id)
                            .map(|added| format!("Installed {}", added.join(", ")))
                    } else {
                        project
                            .remove(&self.catalog, &id, false)
                            .map(|_| format!("Removed {id}"))
                    }
                    .and_then(|msg| project.save().map(|()| msg));
                    self.report(result, |m| m);
                    self.refresh();
                }
            }
        }
        if keep {
            self.modal = Some(modal);
        }
    }
}
