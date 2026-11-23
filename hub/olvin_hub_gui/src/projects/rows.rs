//! The row and card layouts: table row / compact card and grid card.

use eframe::egui::{
    self, Align2, CursorIcon, Id, Rangef, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};

use crate::ui::{self, FS_MD, FS_SM, RADIUS_LG, RADIUS_MD, anim};
use crate::widgets::{card_bg, chip_line, sans, truncated};
use crate::{Row, RowAction};

use super::*;

/// Hover lift (STYLE.md §6: translateY(-4px), stronger shadow), none with reduce motion.
pub(super) fn lift(ctx: &egui::Context, hover: f32, px: f32) -> f32 {
    if ui::reduce_motion(ctx) {
        0.0
    } else {
        -px * hover
    }
}

/// One project in list view (table row or compact card). A click anywhere
/// outside the buttons opens the project.
#[allow(clippy::too_many_arguments)]
pub(super) fn list_row(
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
pub(super) fn finish_row(
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
pub(super) fn grid_card(
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
