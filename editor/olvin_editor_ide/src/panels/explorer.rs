//! Project explorer: the game project's files as a tree.

use std::path::PathBuf;

use olvin_editor::egui::{self, Align2, RichText, Sense};
use olvin_editor::{EditorState, Panel, theme};

use crate::dialogs::Dialog;
use crate::files;
use crate::ide::{self, Command, Ide};

#[derive(Default)]
pub struct Explorer;

const ROW: f32 = 21.0;
const INDENT: f32 = 14.0;

impl Panel for Explorer {
    fn ui(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        ide::with(|ide| {
            ide.frame(ui, state);
            draw(ui, state, ide);
        });
    }
}

enum Action {
    Open(PathBuf),
    Toggle(PathBuf),
    Select(PathBuf),
    Dialog(Dialog),
    Run(Command),
    Copy(String),
}

fn draw(ui: &mut egui::Ui, state: &mut EditorState, ide: &mut Ide) {
    let Some(root) = ide.root.clone() else {
        ui.add_space(8.0);
        ui.label(RichText::new("No project open.").color(theme::palette(ui.ctx()).muted));
        ui.label(
            RichText::new("Start the editor with a game project folder (olvin-hub does that).")
                .small()
                .color(theme::palette(ui.ctx()).muted),
        );
        return;
    };
    let mut actions = Vec::new();

    ui.horizontal(|ui| {
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().to_uppercase())
            .unwrap_or_default();
        ui.label(RichText::new(name).strong().size(12.0))
            .on_hover_text(root.display().to_string());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let small = |ui: &mut egui::Ui, glyph: &str, tip: &str| {
                ui.add(
                    egui::Button::new(glyph)
                        .frame(false)
                        .min_size(egui::vec2(20.0, 20.0)),
                )
                .on_hover_text(tip)
                .clicked()
            };
            if small(ui, "⊟", "Collapse All") {
                actions.push(Action::Run(Command::CollapseAll));
            }
            if small(ui, "⟳", "Refresh") {
                actions.push(Action::Run(Command::RefreshExplorer));
            }
            if small(ui, "◎", "Reveal Active File") {
                actions.push(Action::Run(Command::RevealActive));
            }
            if small(ui, "🗁", "New Folder…") {
                actions.push(Action::Run(Command::NewFolder));
            }
            if small(ui, "🗋", "New File…") {
                actions.push(Action::Run(Command::NewFile));
            }
        });
    });
    ui.add_space(2.0);

    let rows = files::visible(&ide.entries, &ide.expanded);
    let active = ide.active_path().map(PathBuf::from);
    let mut scroll = egui::ScrollArea::vertical().auto_shrink(false);
    if std::mem::take(&mut ide.reveal)
        && let Some(target) = &ide.selected
        && let Some(idx) = rows.iter().position(|e| &e.path == target)
    {
        let spacing = ui.spacing().item_spacing.y;
        let offset = idx as f32 * (ROW + spacing) - ui.available_height() / 2.0;
        scroll = scroll.vertical_scroll_offset(offset.max(0.0));
    }
    // Registered before the rows so they stay on top: a widget added later
    // would swallow their clicks.
    let empty = ui.interact(
        ui.available_rect_before_wrap(),
        ui.id().with("explorer_bg"),
        Sense::click(),
    );
    scroll.show_rows(ui, ROW, rows.len(), |ui, range| {
        for e in &rows[range] {
            let open = e.is_dir && ide.expanded.contains(&e.path);
            let width = ui.available_width();
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, ROW), Sense::click());
            let selected = ide.selected.as_ref() == Some(&e.path);
            let is_active = active.as_ref() == Some(&e.path);
            let bg = if selected {
                Some(theme::ACCENT.gamma_multiply(0.35))
            } else if is_active {
                Some(theme::palette(ui.ctx()).surface_raised)
            } else if resp.hovered() {
                Some(theme::palette(ui.ctx()).surface_raised.gamma_multiply(0.8))
            } else {
                None
            };
            if let Some(bg) = bg {
                ui.painter().rect_filled(rect, 4.0, bg);
            }
            let x = rect.left() + 4.0 + e.depth as f32 * INDENT;
            let y = rect.center().y;
            let p = ui.painter();
            if e.is_dir {
                p.text(
                    egui::pos2(x + 5.0, y),
                    Align2::CENTER_CENTER,
                    if open { "⏷" } else { "⏵" },
                    egui::FontId::proportional(10.0),
                    theme::palette(ui.ctx()).muted,
                );
            }
            let (glyph, color) = files::icon(e.name(), e.is_dir, open);
            p.text(
                egui::pos2(x + 20.0, y),
                Align2::CENTER_CENTER,
                glyph,
                egui::FontId::proportional(13.0),
                color,
            );
            let dirty = !e.is_dir && ide.docs.iter().any(|d| d.dirty && d.path == e.path);
            let name_color = theme::palette(ui.ctx()).text;
            let text_rect = p.text(
                egui::pos2(x + 31.0, y),
                Align2::LEFT_CENTER,
                e.name(),
                egui::TextStyle::Body.resolve(ui.style()),
                name_color,
            );
            if dirty {
                p.text(
                    egui::pos2(text_rect.right() + 6.0, y),
                    Align2::LEFT_CENTER,
                    "⏺",
                    egui::FontId::proportional(10.0),
                    theme::palette(ui.ctx()).muted,
                );
            }

            if resp.clicked() {
                actions.push(Action::Select(e.path.clone()));
                actions.push(if e.is_dir {
                    Action::Toggle(e.path.clone())
                } else {
                    Action::Open(e.path.clone())
                });
            }
            if resp.middle_clicked() && !e.is_dir {
                actions.push(Action::Open(e.path.clone()));
            }
            resp.context_menu(|ui| {
                actions.push(Action::Select(e.path.clone()));
                entry_menu(ui, e, &root, &mut actions);
            });
        }
    });

    // Right click on empty space: create in the root.
    empty.context_menu(|ui| {
        if ui.button("New File…").clicked() {
            actions.push(Action::Dialog(Dialog::New {
                dir: root.clone(),
                folder: false,
                name: String::new(),
                error: None,
            }));
        }
        if ui.button("New Folder…").clicked() {
            actions.push(Action::Dialog(Dialog::New {
                dir: root.clone(),
                folder: true,
                name: String::new(),
                error: None,
            }));
        }
        if ui.button("New Surface Shader…").clicked() {
            actions.push(Action::Run(Command::NewSurfaceShader));
        }
    });

    for action in actions {
        match action {
            Action::Open(path) => {
                ide.open(&path, state);
            }
            Action::Toggle(path) => {
                if !ide.expanded.remove(&path) {
                    ide.expanded.insert(path);
                }
            }
            Action::Select(path) => ide.selected = Some(path),
            Action::Dialog(d) => ide.dialog = Some(d),
            Action::Run(cmd) => ide.run(cmd, state),
            Action::Copy(text) => ui.ctx().copy_text(text),
        }
    }
}

fn entry_menu(
    ui: &mut egui::Ui,
    e: &files::Entry,
    root: &std::path::Path,
    actions: &mut Vec<Action>,
) {
    let dir = if e.is_dir {
        e.path.clone()
    } else {
        e.path.parent().unwrap_or(root).to_path_buf()
    };
    if !e.is_dir && ui.button("Open").clicked() {
        actions.push(Action::Open(e.path.clone()));
    }
    if ui.button("New File…").clicked() {
        actions.push(Action::Dialog(Dialog::New {
            dir: dir.clone(),
            folder: false,
            name: String::new(),
            error: None,
        }));
    }
    if ui.button("New Folder…").clicked() {
        actions.push(Action::Dialog(Dialog::New {
            dir,
            folder: true,
            name: String::new(),
            error: None,
        }));
    }
    ui.separator();
    if ui.button("Rename…").clicked() {
        actions.push(Action::Dialog(Dialog::Rename {
            path: e.path.clone(),
            name: e.name().to_owned(),
            error: None,
        }));
    }
    if ui
        .button(RichText::new("Delete…").color(ui.visuals().error_fg_color))
        .clicked()
    {
        actions.push(Action::Dialog(Dialog::Delete {
            path: e.path.clone(),
        }));
    }
    ui.separator();
    if ui.button("Copy Path").clicked() {
        actions.push(Action::Copy(e.path.display().to_string()));
    }
    if ui.button("Copy Relative Path").clicked() {
        actions.push(Action::Copy(e.rel.clone()));
    }
}
