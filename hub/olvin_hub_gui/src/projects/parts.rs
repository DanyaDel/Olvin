//! Painted pieces of a project row: letter tile, name and path, modified
//! time and the action buttons.

use eframe::egui::{
    self, Align2, Color32, CursorIcon, Painter, Rangef, Rect, Sense, Stroke, StrokeKind, Ui,
    UiBuilder, Vec2, pos2, vec2,
};

use crate::ui::{self, FS_MD, FS_SM, Palette, RADIUS_SM, anim};
use crate::widgets::{Btn, Icon, bold, mono, paint_icon, sans, truncated};
use crate::{Row, RowAction, ago};

/// Letter tile: accent when it's the most recent project.
pub(super) fn paint_tile(
    painter: &Painter,
    rect: Rect,
    row: &Row,
    recent: bool,
    p: &Palette,
    font: f32,
) {
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
pub(super) fn paint_name_path(
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

pub(super) fn modified_text(row: &Row) -> String {
    row.modified.map(ago).unwrap_or_else(|| "—".into())
}

/// The ⋯ menu and (wide rows) the Open button, in a child ui so the
/// row's entrance fade applies to them.
pub(super) fn row_actions(
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
pub(super) struct MenuItem<'a>(Icon, &'a str);

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
