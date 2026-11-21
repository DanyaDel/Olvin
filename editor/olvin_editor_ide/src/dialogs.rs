//! Modal dialogs: new file/folder, rename, delete, new surface shader, close
//! with unsaved changes.

use std::path::{Path, PathBuf};

use olvin_editor::EditorState;
use olvin_editor::egui::{self, Key, RichText};
use olvin_editor::theme;

use crate::files;
use crate::ide::{Ide, create_file};

pub enum Dialog {
    New {
        dir: PathBuf,
        folder: bool,
        name: String,
        error: Option<String>,
    },
    Rename {
        path: PathBuf,
        name: String,
        error: Option<String>,
    },
    Delete {
        path: PathBuf,
    },
    NewShader {
        name: String,
        error: Option<String>,
    },
    CloseDirty {
        path: PathBuf,
    },
}

enum Outcome {
    Keep,
    Close,
}

pub fn show(ide: &mut Ide, ctx: &egui::Context, state: &mut EditorState) {
    let Some(mut dialog) = ide.dialog.take() else {
        return;
    };
    let mut outcome = Outcome::Keep;
    let modal = egui::Modal::new(egui::Id::new("olvin_ide_dialog")).show(ctx, |ui| {
        ui.set_width(380.0);
        let enter = ui.input(|i| i.key_pressed(Key::Enter));
        match &mut dialog {
            Dialog::New {
                dir,
                folder,
                name,
                error,
            } => {
                let what = if *folder { "New Folder" } else { "New File" };
                ui.heading(what);
                ui.label(RichText::new(format!("in {}/", ide.rel(dir))).color(theme::palette(ui.ctx()).muted));
                let ok = name_field(ui, name, error, enter);
                if buttons(ui, "Create") || ok {
                    let path = dir.join(name.trim());
                    let result = match files::valid_name(name) {
                        Err(e) => Err(e.to_owned()),
                        Ok(()) if *folder => {
                            if path.exists() {
                                Err("already exists".to_owned())
                            } else {
                                std::fs::create_dir_all(&path).map_err(|e| e.to_string())
                            }
                        }
                        Ok(()) => create_file(&path, ""),
                    };
                    match result {
                        Ok(()) => {
                            state.info(format!("created {}", ide.rel(&path)));
                            ide.rescan();
                            ide.expanded.insert(dir.clone());
                            if *folder {
                                ide.expanded.insert(path.clone());
                                ide.selected = Some(path);
                            } else {
                                ide.open(&path, state);
                                ide.reveal_active();
                            }
                            outcome = Outcome::Close;
                        }
                        Err(e) => *error = Some(e),
                    }
                }
            }
            Dialog::Rename { path, name, error } => {
                ui.heading("Rename");
                ui.label(RichText::new(ide.rel(path)).color(theme::palette(ui.ctx()).muted));
                let ok = name_field(ui, name, error, enter);
                if buttons(ui, "Rename") || ok {
                    let to = path.parent().unwrap_or(Path::new("")).join(name.trim());
                    let result = match files::valid_name(name) {
                        Err(e) => Err(e.to_owned()),
                        Ok(()) if to == *path => Ok(()),
                        Ok(()) if to.exists() => Err("already exists".to_owned()),
                        Ok(()) => std::fs::rename(&*path, &to).map_err(|e| e.to_string()),
                    };
                    match result {
                        Ok(()) => {
                            state.info(format!("renamed {} -> {}", ide.rel(path), ide.rel(&to)));
                            ide.renamed(path, &to);
                            ide.selected = Some(to);
                            ide.rescan();
                            outcome = Outcome::Close;
                        }
                        Err(e) => *error = Some(e),
                    }
                }
            }
            Dialog::Delete { path } => {
                let folder = path.is_dir();
                ui.heading(if folder {
                    "Delete Folder"
                } else {
                    "Delete File"
                });
                ui.label(format!(
                    "Delete “{}”{}?",
                    ide.rel(path),
                    if folder { " and everything in it" } else { "" }
                ));
                ui.label(RichText::new("This can't be undone.").color(ui.visuals().warn_fg_color));
                ui.add_space(6.0);
                if danger_buttons(ui, "Delete") || enter {
                    let result = if folder {
                        std::fs::remove_dir_all(&*path)
                    } else {
                        std::fs::remove_file(&*path)
                    };
                    match result {
                        Ok(()) => {
                            state.info(format!("deleted {}", ide.rel(path)));
                            ide.deleted(path);
                            ide.selected = None;
                            ide.rescan();
                        }
                        Err(e) => state.error(format!("could not delete {}: {e}", path.display())),
                    }
                    outcome = Outcome::Close;
                }
            }
            Dialog::NewShader { name, error } => {
                ui.heading("New Surface Shader");
                ui.label(
                    RichText::new(
                        "Creates assets/shaders/<name>.wgsl from the surface shader contract.",
                    )
                    .color(theme::palette(ui.ctx()).muted),
                );
                let ok = name_field(ui, name, error, enter);
                if buttons(ui, "Create") || ok {
                    match ide.new_surface_shader(name.trim(), state) {
                        Ok(()) => outcome = Outcome::Close,
                        Err(e) => *error = Some(e),
                    }
                }
            }
            Dialog::CloseDirty { path } => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                ui.heading("Unsaved Changes");
                ui.label(format!("Save the changes to “{name}” before closing?"));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let i = ide.find_doc(path);
                    if ui.button("Save").clicked() || enter {
                        if let Some(i) = i
                            && ide.save(i, state)
                        {
                            ide.close(i, true);
                        }
                        outcome = Outcome::Close;
                    }
                    if ui.button("Don't Save").clicked() {
                        if let Some(i) = i {
                            ide.close(i, true);
                        }
                        outcome = Outcome::Close;
                    }
                    if ui.button("Cancel").clicked() {
                        outcome = Outcome::Close;
                    }
                });
            }
        }
    });
    if modal.should_close() {
        outcome = Outcome::Close;
    }
    if matches!(outcome, Outcome::Close) {
        // The Enter that confirmed must not reach the editor that just got
        // focus (it would type a newline into the new file).
        ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter));
        // The dialog had the keyboard; hand it back so Ctrl+S / Ctrl+W work.
        // Two passes later: the closed modal still blocks focus changes.
        ide.refocus_in = 2;
    }
    if matches!(outcome, Outcome::Keep) && ide.dialog.is_none() {
        ide.dialog = Some(dialog);
    }
}

/// Name field with focus and error line; true if Enter was pressed in it.
fn name_field(
    ui: &mut egui::Ui,
    name: &mut String,
    error: &mut Option<String>,
    enter: bool,
) -> bool {
    ui.add_space(4.0);
    let edit = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
    if !edit.has_focus() && !edit.lost_focus() {
        edit.request_focus();
    }
    if edit.changed() {
        *error = None;
    }
    if let Some(e) = error {
        ui.label(RichText::new(e.as_str()).color(ui.visuals().error_fg_color));
    }
    ui.add_space(6.0);
    edit.lost_focus() && enter
}

/// OK / Cancel row; true if OK. Cancel (and Esc) close through `should_close`.
fn buttons(ui: &mut egui::Ui, ok: &str) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        clicked = ui.button(RichText::new(ok).strong()).clicked();
        if ui.button("Cancel").clicked() {
            ui.close();
        }
    });
    clicked
}

fn danger_buttons(ui: &mut egui::Ui, ok: &str) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        clicked = ui
            .button(
                RichText::new(ok)
                    .strong()
                    .color(ui.visuals().error_fg_color),
            )
            .clicked();
        if ui.button("Cancel").clicked() {
            ui.close();
        }
    });
    clicked
}
