use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{Align, Layout, RichText, Sense, TextEdit};
use glam::Vec3;

use crate::{EditorState, Panel, theme};

const RESCAN: Duration = Duration::from_secs(1);
const TILE: egui::Vec2 = egui::vec2(84.0, 76.0);

struct Entry {
    name: String,
    path: PathBuf,
    is_dir: bool,
}

/// Drag and drop payload: an asset (absolute path) dragged out of the panel.
/// The Scene view and the Hierarchy accept it.
pub(crate) struct AssetDrag(pub PathBuf);

/// The asset released over `response` this frame. egui's own
/// `dnd_release_payload` takes whatever payload there is and throws it away
/// when the type differs, so a Hierarchy row checking for an object drag
/// would eat an asset drag: this one only looks.
pub(crate) fn dropped(response: &egui::Response) -> Option<std::sync::Arc<AssetDrag>> {
    if response.contains_pointer() && response.ctx.input(|i| i.pointer.any_released()) {
        egui::DragAndDrop::payload::<AssetDrag>(&response.ctx)
    } else {
        None
    }
}

/// Where an asset was dropped.
pub(crate) enum DropTarget {
    /// On the Scene view: the object under the pointer and the ground point.
    Viewport { pick: Option<u64>, at: Option<Vec3> },
    /// On a Hierarchy row (or the empty area: no parent).
    Hierarchy(Option<u64>),
}

#[derive(Default)]
pub struct Assets {
    /// Relative to the project's `assets/`.
    current: PathBuf,
    entries: Vec<Entry>,
    scanned: Option<(PathBuf, Instant)>,
    selected: Option<PathBuf>,
    filter: String,
    creating: Option<(bool, String)>,
    /// A tile being renamed (F2 / context menu) and its edit buffer.
    renaming: Option<(PathBuf, String)>,
    /// Waiting for the user to confirm a delete.
    deleting: Option<PathBuf>,
    /// Image previews: decoded by olvin_assets, uploaded to egui once.
    thumbs: olvin_assets::ThumbnailCache,
    textures: std::collections::HashMap<
        PathBuf,
        (std::sync::Arc<olvin_assets::ImageData>, egui::TextureHandle),
    >,
}

impl Panel for Assets {
    fn ui(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        let Some(root) = state.assets_dir() else {
            ui.centered_and_justified(|ui| {
                ui.weak(
                    "No project open: run olvin-editor <project folder> or open it from the hub",
                )
            });
            return;
        };
        // "Select Prefab Asset" in the Inspector: show the file's folder, select the tile.
        if let Some(path) = state.asset_focus.take()
            && let Some(parent) = path.parent().and_then(|p| p.strip_prefix(&root).ok())
        {
            self.current = parent.to_path_buf();
            self.selected = Some(path);
            self.scanned = None;
        }
        let dir = root.join(&self.current);
        let stale = self
            .scanned
            .as_ref()
            .is_none_or(|(d, t)| *d != dir || t.elapsed() > RESCAN);
        if stale {
            self.rescan(&dir);
        }

        self.os_drops(ui, state, &dir, &root);
        let hovered = ui.ui_contains_pointer();
        // Delete and Ctrl+D act on files here, not on scene objects.
        state.panel_owns_edit_keys |= hovered;
        let mut moves: Vec<(PathBuf, PathBuf)> = Vec::new();
        self.toolbar(ui, state, &dir, &root, &mut moves);
        ui.separator();

        let filter = self.filter.trim().to_lowercase();
        let mut open = None;
        let mut rename_to = None;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                if self.entries.is_empty() {
                    ui.weak("Empty folder. Right-click or \"+\" to create a folder or a scene");
                }
                let entries = std::mem::take(&mut self.entries);
                ui.horizontal_wrapped(|ui| {
                    for entry in entries
                        .iter()
                        .filter(|e| filter.is_empty() || e.name.to_lowercase().contains(&filter))
                    {
                        if let Some((path, name)) = &mut self.renaming
                            && *path == entry.path
                        {
                            if let Some(new) = rename_field(ui, name) {
                                rename_to = Some((entry.path.clone(), new));
                            }
                            continue;
                        }
                        let selected = self.selected.as_ref() == Some(&entry.path);
                        let thumb = self.thumbnail(ui.ctx(), &entry.path);
                        let response = tile(ui, entry, selected, thumb.as_ref());
                        if response.clicked() || response.drag_started() {
                            self.selected = Some(entry.path.clone());
                        }
                        if response.double_clicked() {
                            open = Some((entry.path.clone(), entry.is_dir));
                        }
                        if response.dragged() {
                            response.dnd_set_drag_payload(AssetDrag(entry.path.clone()));
                            drag_label(ui, &entry.name);
                        }
                        if entry.is_dir {
                            if let Some(d) = dropped(&response) {
                                moves.push((d.0.clone(), entry.path.join(file_name(&d.0))));
                            }
                            if response
                                .dnd_hover_payload::<AssetDrag>()
                                .is_some_and(|d| d.0 != entry.path)
                            {
                                ui.painter().rect_stroke(
                                    response.rect,
                                    4.0,
                                    (1.5, theme::palette(ui.ctx()).accent),
                                    egui::StrokeKind::Inside,
                                );
                            }
                        }
                        response.context_menu(|ui| {
                            if is_scene(&entry.path) && ui.button("Open scene").clicked() {
                                state.request_open(entry.path.clone());
                            }
                            if is_prefab(&entry.path) {
                                if ui.button("Instantiate").clicked() {
                                    drop_asset(
                                        state,
                                        &entry.path,
                                        DropTarget::Hierarchy(state.selection),
                                    );
                                    ui.close();
                                }
                                if ui
                                    .button("Open prefab")
                                    .on_hover_text("Edit it as a scene; instances follow on save")
                                    .clicked()
                                {
                                    state.request_open(entry.path.clone());
                                }
                            }
                            if is_model(&entry.path) && ui.button("Import into scene").clicked() {
                                import(state, &entry.path);
                            }
                            if is_image(&entry.path) && ui.button("Create material").clicked() {
                                material_from_texture(state, &root, &entry.path);
                            }
                            if ui.button("Rename  F2").clicked() {
                                self.renaming = Some((entry.path.clone(), entry.name.clone()));
                                ui.close();
                            }
                            if ui.button("Delete  Del").clicked() {
                                self.deleting = Some(entry.path.clone());
                                ui.close();
                            }
                            ui.separator();
                            if ui.button("Show in file manager").clicked() {
                                reveal(entry.path.parent().unwrap_or(&entry.path));
                            }
                        });
                    }
                });
                self.entries = entries;
                let rest = ui.allocate_response(
                    ui.available_size().max(egui::vec2(0.0, 30.0)),
                    Sense::click(),
                );
                if rest.clicked() {
                    self.selected = None;
                }
                // Dropping on the empty area moves into the folder being shown.
                if let Some(d) = dropped(&rest) {
                    moves.push((d.0.clone(), dir.join(file_name(&d.0))));
                }
                rest.context_menu(|ui| create_menu(ui, &mut self.creating));
            });

        if hovered
            && self.renaming.is_none()
            && !ui.ctx().egui_wants_keyboard_input()
            && let Some(sel) = self.selected.clone()
        {
            if ui.input(|i| i.key_pressed(egui::Key::F2)) {
                self.renaming = Some((sel.clone(), file_name(&sel)));
            }
            if ui.input(|i| i.key_pressed(egui::Key::Delete)) {
                self.deleting = Some(sel);
            }
        }
        if let Some((from, name)) = rename_to {
            self.renaming = None;
            let name = name.trim().to_owned();
            if name.is_empty() || name == file_name(&from) {
            } else if !valid_name(&name) {
                state.error(format!("\"{name}\" is not a valid name"));
            } else {
                moves.push((from.clone(), from.with_file_name(name)));
            }
        }
        for (from, to) in moves {
            if from == to {
                continue;
            }
            match move_asset(state, &root, &from, &to) {
                Ok(()) => {
                    if self.selected.as_ref() == Some(&from) {
                        self.selected = Some(to);
                    }
                }
                Err(e) => state.error(format!("move: {e}")),
            }
            self.scanned = None;
        }
        self.confirm_delete(ui.ctx(), state, &root);

        if let Some((path, is_dir)) = open {
            if is_dir {
                if let Ok(rel) = path.strip_prefix(&root) {
                    self.current = rel.to_path_buf();
                }
                self.scanned = None;
            } else if is_scene(&path) || is_prefab(&path) {
                state.request_open(path);
            } else if is_model(&path) {
                import(state, &path);
            } else {
                reveal(&path);
            }
        }
    }
}

impl Assets {
    /// An egui texture with the image's thumbnail (re-uploaded when it changes).
    fn thumbnail(&mut self, ctx: &egui::Context, path: &Path) -> Option<egui::TextureHandle> {
        if !is_image(path) {
            return None;
        }
        let image = self.thumbs.get(path, 96)?;
        if let Some((data, tex)) = self.textures.get(path)
            && std::sync::Arc::ptr_eq(data, &image)
        {
            return Some(tex.clone());
        }
        let color = egui::ColorImage::from_rgba_unmultiplied(
            [image.width as usize, image.height as usize],
            &image.pixels,
        );
        let tex = ctx.load_texture(path.to_string_lossy(), color, egui::TextureOptions::LINEAR);
        self.textures
            .insert(path.to_path_buf(), (image, tex.clone()));
        Some(tex)
    }

    /// Files dragged in from the OS file manager: a hint while they hover, and
    /// the copy on drop. They land in the folder this panel shows; the drop may
    /// happen anywhere in the window, as in most file managers.
    fn os_drops(&mut self, ui: &mut egui::Ui, state: &mut EditorState, dir: &Path, root: &Path) {
        let ctx = ui.ctx().clone();
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if hovering {
            let palette = crate::theme::palette(&ctx);
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("os-drop-hint"),
            ));
            let screen = ctx.content_rect();
            painter.rect_filled(screen, 0.0, palette.bg.gamma_multiply(0.7));
            let folder = self.current.display().to_string();
            painter.text(
                screen.center(),
                egui::Align2::CENTER_CENTER,
                format!("Drop to import into assets/{folder}"),
                egui::FontId::proportional(22.0),
                palette.accent,
            );
        }
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if dropped.is_empty() {
            return;
        }
        let report = crate::os_drop::import(&dropped, dir);
        if !report.imported.is_empty() {
            // Give every new file its `.meta` GUID, as a scan does.
            let _ = olvin_assets::AssetRoot::new(root).scan();
            self.selected = report.imported.first().cloned();
            self.scanned = None;
            state.info(format!(
                "imported {} into assets/{}{}",
                report.imported.len(),
                self.current.display(),
                if report.renamed > 0 {
                    format!(" ({} renamed, nothing overwritten)", report.renamed)
                } else {
                    String::new()
                }
            ));
        }
        for reason in report.skipped {
            state.warn(format!("not imported: {reason}"));
        }
    }

    fn rescan(&mut self, dir: &Path) {
        let _ = std::fs::create_dir_all(dir);
        self.entries = std::fs::read_dir(dir)
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .map(|e| Entry {
                        name: e.file_name().to_string_lossy().into_owned(),
                        path: e.path(),
                        is_dir: e.file_type().is_ok_and(|t| t.is_dir()),
                    })
                    .filter(|e| !e.name.starts_with('.'))
                    .collect()
            })
            .unwrap_or_default();
        self.entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        self.scanned = Some((dir.to_path_buf(), Instant::now()));
    }

    fn confirm_delete(&mut self, ctx: &egui::Context, state: &mut EditorState, root: &Path) {
        let Some(path) = self.deleting.clone() else {
            return;
        };
        let mut choice = None;
        egui::Modal::new(egui::Id::new("assets-delete")).show(ctx, |ui| {
            ui.heading("Delete Asset?");
            ui.label(format!(
                "Delete {} together with its .meta file? This cannot be undone.",
                file_name(&path)
            ));
            if path.is_dir() {
                ui.weak("Everything inside the folder goes too.");
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Delete").clicked() {
                    choice = Some(true);
                }
                if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    choice = Some(false);
                }
            });
        });
        if let Some(delete) = choice {
            self.deleting = None;
            if delete {
                match delete_asset(state, root, &path) {
                    Ok(()) => {
                        self.selected = None;
                        self.scanned = None;
                    }
                    Err(e) => state.error(format!("delete: {e}")),
                }
            }
        }
    }

    fn toolbar(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut EditorState,
        dir: &Path,
        root: &Path,
        moves: &mut Vec<(PathBuf, PathBuf)>,
    ) {
        ui.horizontal(|ui| {
            let up = ui
                .add_enabled(!self.current.as_os_str().is_empty(), egui::Button::new("⬆"))
                .on_hover_text("Up (drop a file here to move it up)");
            if up.clicked() {
                self.current.pop();
            }
            if let (Some(d), Some(parent)) = (
                dropped(&up),
                dir.parent()
                    .filter(|_| !self.current.as_os_str().is_empty()),
            ) {
                moves.push((d.0.clone(), parent.join(file_name(&d.0))));
            }
            // Breadcrumbs: assets / a / b; each one takes dropped files.
            let home = ui.link("assets");
            if home.clicked() {
                self.current = PathBuf::new();
            }
            if let Some(d) = dropped(&home) {
                moves.push((d.0.clone(), root.join(file_name(&d.0))));
            }
            let parts: Vec<std::ffi::OsString> = self
                .current
                .components()
                .map(|c| c.as_os_str().to_owned())
                .collect();
            for (i, part) in parts.iter().enumerate() {
                ui.weak("/");
                let crumb = ui.link(part.to_string_lossy());
                let target: PathBuf = parts[..=i].iter().collect();
                if crumb.clicked() {
                    self.current = target.clone();
                }
                if let Some(d) = dropped(&crumb) {
                    moves.push((d.0.clone(), root.join(&target).join(file_name(&d.0))));
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.menu_button("+", |ui| create_menu(ui, &mut self.creating));
                if ui.button("⟳").on_hover_text("Refresh").clicked() {
                    self.scanned = None;
                }
                ui.add(
                    TextEdit::singleline(&mut self.filter)
                        .hint_text("🔍 Filter")
                        .desired_width(140.0),
                );
            });
        });

        if let Some((is_folder, mut name)) = self.creating.take() {
            let mut keep = true;
            ui.horizontal(|ui| {
                ui.label(if is_folder {
                    "New folder:"
                } else {
                    "New scene:"
                });
                let edit = ui.add(TextEdit::singleline(&mut name).desired_width(180.0));
                // Ask before focusing: request_focus() undoes "lost focus".
                let lost = edit.lost_focus();
                let submit = lost && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
                if !lost && !cancel {
                    edit.request_focus();
                }
                if cancel || lost && !submit || submit && name.trim().is_empty() {
                    keep = false;
                } else if submit {
                    let name = name.trim().to_owned();
                    if !valid_name(&name) {
                        state.error(format!("\"{name}\" is not a valid name"));
                    } else if is_folder {
                        if let Err(e) = std::fs::create_dir_all(dir.join(&name)) {
                            state.error(format!("could not create folder: {e}"));
                        }
                    } else {
                        let file = if name.ends_with(".scene.ron") {
                            name
                        } else {
                            format!("{name}.scene.ron")
                        };
                        let path = dir.join(file);
                        if path.exists() {
                            state.error(format!("{} already exists", path.display()));
                        } else if let Err(e) = olvin_scene::file::SceneFile::default().save(&path) {
                            state.error(format!("could not create scene: {e}"));
                        } else {
                            state.request_open(path);
                        }
                    }
                    keep = false;
                    self.scanned = None;
                }
            });
            if keep {
                self.creating = Some((is_folder, name));
            }
        }
    }
}

/// A path that stays inside the current folder: normal components only
/// (no `..`, no leading `/`).
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && Path::new(name)
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

fn create_menu(ui: &mut egui::Ui, creating: &mut Option<(bool, String)>) {
    if ui.button("📁 Folder").clicked() {
        *creating = Some((true, String::new()));
    }
    if ui.button("🎬 Scene").clicked() {
        *creating = Some((false, String::new()));
    }
}

fn tile(
    ui: &mut egui::Ui,
    entry: &Entry,
    selected: bool,
    thumb: Option<&egui::TextureHandle>,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(TILE, Sense::click_and_drag());
    let visuals = ui.style().interact_selectable(&response, selected);
    if selected || response.hovered() {
        let fill = if selected {
            theme::palette(ui.ctx()).accent.gamma_multiply(0.55)
        } else {
            visuals.weak_bg_fill
        };
        ui.painter().rect_filled(rect, 4.0, fill);
    }
    let painter = ui.painter();
    if let Some(tex) = thumb {
        let size = tex.size_vec2();
        let fit = (44.0 / size.x.max(size.y)).min(1.0) * size;
        let r = egui::Rect::from_center_size(rect.center_top() + egui::vec2(0.0, 26.0), fit);
        painter.image(
            tex.id(),
            r,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    } else {
        painter.text(
            rect.center_top() + egui::vec2(0.0, 26.0),
            egui::Align2::CENTER_CENTER,
            icon(entry),
            egui::FontId::proportional(28.0),
            // Prefabs are blue here as in the Hierarchy.
            if is_prefab(&entry.path) {
                theme::palette(ui.ctx()).info
            } else {
                ui.visuals().text_color()
            },
        );
    }
    let name = if entry.name.chars().count() > 12 {
        format!("{}…", entry.name.chars().take(11).collect::<String>())
    } else {
        entry.name.clone()
    };
    painter.text(
        rect.center_bottom() - egui::vec2(0.0, 12.0),
        egui::Align2::CENTER_CENTER,
        name,
        egui::FontId::proportional(12.0),
        ui.visuals().text_color(),
    );
    response.on_hover_text(RichText::new(&entry.name).monospace())
}

fn icon(entry: &Entry) -> &'static str {
    if entry.is_dir {
        return "📁";
    }
    let ext = entry
        .path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        _ if is_scene(&entry.path) => "🎬",
        "png" | "jpg" | "jpeg" | "tga" | "bmp" | "webp" | "ktx2" => "🖼",
        "gltf" | "glb" | "obj" | "fbx" => "📦",
        "wav" | "ogg" | "mp3" | "flac" => "🔊",
        _ if entry.name.ends_with(".mat.ron") => "🎨",
        "wgsl" => "🧪",
        "hdr" | "exr" => "🌅",
        "rs" => "🦀",
        _ if is_prefab(&entry.path) => "◆",
        "ron" | "toml" | "json" => "📝",
        _ => "📄",
    }
}

fn is_model(path: &Path) -> bool {
    let p = path.to_string_lossy().to_lowercase();
    p.ends_with(".gltf") || p.ends_with(".glb")
}

fn is_image(path: &Path) -> bool {
    let p = path.to_string_lossy().to_lowercase();
    ["png", "jpg", "jpeg", "tga", "bmp", "webp", "hdr", "exr"]
        .iter()
        .any(|e| p.ends_with(&format!(".{e}")))
}

fn import(state: &mut EditorState, path: &Path) {
    if let Err(e) = state.import_model(path) {
        state.warn(format!("import: {e}"));
    }
}

/// `materials/<texture>.mat.ron` using the image as base colour.
fn material_from_texture(state: &mut EditorState, root: &Path, path: &Path) {
    let Ok(rel) = path.strip_prefix(root) else {
        return;
    };
    let rel = rel.to_string_lossy().replace('\\', "/");
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let desc = olvin_scene::material::MaterialDesc {
        base_color: [1.0; 4],
        base_texture: Some(rel),
        ..Default::default()
    };
    state.materials.refresh(Some(root));
    match state.materials.create(&name, desc) {
        Ok(m) => state.info(format!("created {m}")),
        Err(e) => state.warn(e),
    }
}

fn is_scene(path: &Path) -> bool {
    path.to_string_lossy().ends_with(".scene.ron")
}

fn is_prefab(path: &Path) -> bool {
    path.to_string_lossy().ends_with(".prefab.ron")
}

/// Opens a folder or file with the system handler.
fn reveal(path: &Path) {
    let opener = if cfg!(target_os = "windows") {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(opener).arg(path).spawn();
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The inline rename box of a tile; `Some(name)` when Enter or a click
/// elsewhere ends the edit, `Some("")` when Escape cancels it.
fn rename_field(ui: &mut egui::Ui, name: &mut String) -> Option<String> {
    let edit = ui.add_sized(egui::vec2(TILE.x + 40.0, 22.0), TextEdit::singleline(name));
    // Ask before focusing: request_focus() undoes "lost focus" in a frame.
    let done = edit.lost_focus();
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        return Some(String::new());
    }
    if done {
        return Some(name.clone());
    }
    edit.request_focus();
    None
}

/// The dragged asset's name next to the pointer.
fn drag_label(ui: &egui::Ui, name: &str) {
    if let Some(p) = ui.ctx().pointer_interact_pos() {
        ui.ctx()
            .layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::new("asset-drag"),
            ))
            .text(
                p + egui::vec2(12.0, 0.0),
                egui::Align2::LEFT_CENTER,
                name,
                egui::FontId::proportional(13.0),
                theme::palette(ui.ctx()).selected(),
            );
    }
}

/// Renames or moves a file or folder (its `.meta` and GUID go with it) and
/// rewrites every reference to it: scenes and materials on disk, and the open
/// scene with its history.
fn move_asset(state: &mut EditorState, root: &Path, from: &Path, to: &Path) -> Result<(), String> {
    let mut assets = olvin_assets::AssetRoot::new(root);
    let (Some(from_rel), Some(to_rel)) = (assets.relative(from), assets.relative(to)) else {
        return Err("outside assets/".into());
    };
    if to_rel == from_rel || to_rel.starts_with(&format!("{from_rel}/")) {
        return Err("cannot move a folder into itself".into());
    }
    assets.move_asset(&from_rel, &to_rel)?;
    if let Some(open) = state.scene_path.clone()
        && let Ok(rest) = open.strip_prefix(from)
    {
        state.scene_path = Some(to.join(rest));
    }
    rewrite_references(root, &from_rel, &to_rel);
    state.map_scenes(|scene| {
        if let Ok(text) = ron::to_string(scene)
            && let Some(new) = replace_path(&text, &from_rel, &to_rel)
            && let Ok(mut fixed) = ron::from_str::<olvin_scene::file::SceneFile>(&new)
        {
            // Which instance a prefab node belongs to is not part of the file text.
            for (e, old) in fixed.entities.iter_mut().zip(&scene.entities) {
                e.prefab_node = old.prefab_node.clone();
            }
            *scene = fixed;
        }
    });
    state.forget_materials(&from_rel);
    state.forget_prefabs(&from_rel);
    state.info(format!("moved {from_rel} to {to_rel}"));
    Ok(())
}

fn delete_asset(state: &mut EditorState, root: &Path, path: &Path) -> Result<(), String> {
    let mut assets = olvin_assets::AssetRoot::new(root);
    let rel = assets.relative(path).ok_or("outside assets/")?;
    unpack_instances(state, root, &rel);
    assets.delete_asset(&rel)?;
    state.forget_materials(&rel);
    state.forget_prefabs(&rel);
    state.info(format!("deleted {rel}"));
    Ok(())
}

/// `text` with the quoted path `from` (or anything under it) renamed to `to`;
/// `None` when it does not occur.
fn replace_path(text: &str, from: &str, to: &str) -> Option<String> {
    // Also inside the RON text of a prefab override (`"\"path\""`).
    let new = text
        .replace(&format!("\\\"{from}\\\""), &format!("\\\"{to}\\\""))
        .replace(&format!("\\\"{from}/"), &format!("\\\"{to}/"))
        .replace(&format!("\"{from}\""), &format!("\"{to}\""))
        .replace(&format!("\"{from}/"), &format!("\"{to}/"));
    (new != text).then_some(new)
}

/// Scene and material files name assets by path: follow a move.
fn rewrite_references(root: &Path, from: &str, to: &str) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if (is_scene(&path)
                || is_prefab(&path)
                || path.to_string_lossy().ends_with(".mat.ron"))
                && let Ok(text) = std::fs::read_to_string(&path)
                && let Some(new) = replace_path(&text, from, to)
            {
                let _ = std::fs::write(&path, new);
            }
        }
    }
}

/// Every `*.prefab.ron` under `dir`, relative to `root` and starting with `prefix`.
fn prefabs_under(root: &Path, dir: &Path, prefix: &str, out: &mut Vec<String>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            prefabs_under(root, &path, prefix, out);
        } else if is_prefab(&path)
            && let Ok(rel) = path.strip_prefix(root)
        {
            let rel = rel.to_string_lossy().replace('\\', "/");
            if rel == prefix || rel.starts_with(&format!("{prefix}/")) {
                out.push(rel);
            }
        }
    }
}

/// A prefab (or a folder of them) is about to be deleted: its instances stop
/// being instances and keep their objects, in the open scene and its history
/// and in every scene and prefab file that uses it.
fn unpack_instances(state: &mut EditorState, root: &Path, rel: &str) {
    let mut gone = Vec::new();
    prefabs_under(root, root, rel, &mut gone);
    for asset in &gone {
        state.map_scenes(|scene| scene.unpack_asset(asset));
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Some(own) = path
                .strip_prefix(root)
                .ok()
                .map(|r| r.to_string_lossy().replace('\\', "/"))
            else {
                continue;
            };
            if !(is_scene(&path) || is_prefab(&path)) || gone.contains(&own) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if !gone.iter().any(|a| text.contains(a.as_str())) {
                continue;
            }
            let mut load = |p: &str| olvin_scene::file::SceneFile::load(root.join(p));
            if let Ok(mut scene) = olvin_scene::file::SceneFile::load(&path) {
                // The files are still there: expand, detach, write back as a file.
                scene.expand_prefabs(&mut load);
                for asset in &gone {
                    scene.unpack_asset(asset);
                }
                if let Err(e) = scene.collapse_prefabs(&mut load).save(&path) {
                    state.warn(format!("{own}: {e}"));
                }
            }
        }
    }
}

/// An asset dropped on the Scene view or the Hierarchy: scenes open, prefabs
/// are instantiated (under the pointer or under a row), models
/// are imported (placed under the pointer or under a row), materials go to
/// the object under the pointer or on the row.
pub(crate) fn drop_asset(state: &mut EditorState, path: &Path, target: DropTarget) {
    let Some(root) = state.assets_dir() else {
        return;
    };
    if is_scene(path) {
        state.request_open(path.to_path_buf());
    } else if is_prefab(path) {
        let Some(rel) = olvin_assets::AssetRoot::new(&root).relative(path) else {
            return;
        };
        let (parent, at) = match target {
            DropTarget::Hierarchy(parent) => (parent, Vec3::ZERO),
            DropTarget::Viewport { at, .. } => (None, at.unwrap_or(Vec3::ZERO)),
        };
        if let Err(e) = state.instantiate_prefab(&rel, parent, at) {
            state.warn(e);
        }
    } else if is_model(path) {
        let before = state.scene.entities.len();
        if let Err(e) = state.import_model(path) {
            state.warn(format!("import: {e}"));
            return;
        }
        let roots: Vec<u64> = state.scene.entities[before..]
            .iter()
            .filter(|e| e.parent.is_none())
            .map(|e| e.id)
            .collect();
        for id in roots {
            match &target {
                DropTarget::Hierarchy(Some(parent)) => {
                    state.reparent(id, Some(*parent));
                }
                DropTarget::Viewport { at: Some(at), .. } => {
                    if let Some(e) = state.scene.get_mut(id) {
                        e.position += *at;
                    }
                }
                _ => {}
            }
        }
    } else if path.to_string_lossy().ends_with(".mat.ron") {
        let id = match target {
            DropTarget::Viewport { pick, .. } => pick,
            DropTarget::Hierarchy(id) => id,
        };
        let rel = olvin_assets::AssetRoot::new(&root).relative(path);
        match (id.and_then(|i| state.scene.get_mut(i)), rel) {
            (Some(e), Some(rel))
                if e.mesh.is_some()
                    || e.brush.is_some()
                    || e.poly.is_some()
                    || e.model.is_some() =>
            {
                e.material = Some(rel);
                let id = e.id;
                state.select(Some(id));
            }
            _ => state.warn("drop a material on an object with a mesh"),
        }
    } else {
        state.warn(format!(
            "{} cannot be dropped into the scene",
            file_name(path)
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NewEntity;
    use olvin_scene::file::Shape;

    const ROBOT: &str = include_str!("../../../../apps/vertical_slice/assets/models/robot.gltf");

    /// A project with a material, a scene using it and a model.
    fn project(name: &str) -> (PathBuf, EditorState, u64) {
        let root = std::env::temp_dir().join(format!("olvin-assets-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let assets = root.join("assets");
        std::fs::create_dir_all(assets.join("materials")).unwrap();
        std::fs::create_dir_all(assets.join("models")).unwrap();
        std::fs::write(assets.join("models/robot.gltf"), ROBOT).unwrap();
        olvin_scene::material::MaterialDesc::default()
            .save(assets.join("materials/red.mat.ron"))
            .unwrap();
        let mut state = EditorState::new(Some(root.clone()));
        let id = state.create(NewEntity::Primitive(Shape::Cube), None);
        state.scene.get_mut(id).unwrap().material = Some("materials/red.mat.ron".into());
        state.scene_path = Some(assets.join("scenes/a.scene.ron"));
        assert!(state.save());
        state.end_frame(false);
        (root, state, id)
    }

    #[test]
    fn moving_a_material_keeps_its_guid_and_fixes_references() {
        let (root, mut state, cube) = project("move");
        let assets = root.join("assets");
        let mut index = olvin_assets::AssetRoot::new(&assets);
        index.scan();
        let guid = index.guid_of("materials/red.mat.ron").unwrap();
        std::fs::create_dir_all(assets.join("shared")).unwrap();

        move_asset(
            &mut state,
            &assets,
            &assets.join("materials/red.mat.ron"),
            &assets.join("shared/red.mat.ron"),
        )
        .unwrap();

        assert!(assets.join("shared/red.mat.ron.meta").exists());
        assert!(!assets.join("materials/red.mat.ron.meta").exists());
        let mut index = olvin_assets::AssetRoot::new(&assets);
        index.scan();
        assert_eq!(index.guid_of("shared/red.mat.ron"), Some(guid));
        let id = cube;
        assert_eq!(
            state.scene.get(id).unwrap().material.as_deref(),
            Some("shared/red.mat.ron"),
            "the open scene follows"
        );
        let on_disk = olvin_scene::file::SceneFile::load(assets.join("scenes/a.scene.ron"));
        assert_eq!(
            on_disk.unwrap().get(cube).unwrap().material.as_deref(),
            Some("shared/red.mat.ron"),
            "other scene files follow"
        );
        assert!(!state.is_dirty(), "a rename is not an unsaved change");
        // A folder takes everything under it.
        move_asset(
            &mut state,
            &assets,
            &assets.join("shared"),
            &assets.join("materials/shared"),
        )
        .unwrap();
        assert_eq!(
            state.scene.get(id).unwrap().material.as_deref(),
            Some("materials/shared/red.mat.ron")
        );
        // Into itself, or onto an existing file: refused, nothing changes.
        assert!(
            move_asset(
                &mut state,
                &assets,
                &assets.join("materials"),
                &assets.join("materials/shared/inner"),
            )
            .is_err()
        );
        assert!(
            move_asset(
                &mut state,
                &assets,
                &assets.join("materials/shared/red.mat.ron"),
                &assets.join("models/robot.gltf"),
            )
            .is_err()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn delete_takes_the_meta_along() {
        let (root, mut state, _) = project("delete");
        let assets = root.join("assets");
        olvin_assets::AssetRoot::new(&assets).scan();
        let mat = assets.join("materials/red.mat.ron");
        assert!(assets.join("materials/red.mat.ron.meta").exists());
        delete_asset(&mut state, &assets, &mat).unwrap();
        assert!(!mat.exists());
        assert!(!assets.join("materials/red.mat.ron.meta").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn moving_a_prefab_fixes_instances_and_deleting_it_unpacks_them() {
        use olvin_scene::file::{EntityDesc, PrefabInstance, SceneFile};
        let (root, mut state, cube) = project("prefab");
        let assets = root.join("assets");
        let rel = state.create_prefab(cube).unwrap();
        state.end_frame(false);
        assert!(state.save());
        // Another scene with an instance whose override names the material.
        let mut inst = EntityDesc::new(5, "Other");
        let mut p = PrefabInstance::new(rel.clone());
        p.overrides.insert(
            String::new(),
            std::collections::BTreeMap::from([(
                "material".into(),
                "Some(\"materials/red.mat.ron\")".into(),
            )]),
        );
        inst.prefab = Some(p);
        let other = assets.join("scenes/other.scene.ron");
        SceneFile {
            entities: vec![inst],
            ..Default::default()
        }
        .save(&other)
        .unwrap();

        std::fs::create_dir_all(assets.join("shared")).unwrap();
        move_asset(
            &mut state,
            &assets,
            &assets.join(&rel),
            &assets.join("shared/Box.prefab.ron"),
        )
        .unwrap();
        let asset_of =
            |s: &SceneFile, id: u64| s.get(id).unwrap().prefab.as_ref().unwrap().asset.clone();
        assert_eq!(asset_of(&state.scene, cube), "shared/Box.prefab.ron");
        assert!(
            state.scene.get(cube).unwrap().prefab_node.is_some(),
            "still expanded"
        );
        assert_eq!(
            asset_of(
                &SceneFile::load(assets.join("scenes/a.scene.ron")).unwrap(),
                cube
            ),
            "shared/Box.prefab.ron"
        );
        assert_eq!(
            asset_of(&SceneFile::load(&other).unwrap(), 5),
            "shared/Box.prefab.ron"
        );
        assert!(!state.is_dirty(), "a move is not an unsaved change");
        assert!(
            assets.join("shared/Box.prefab.ron.meta").exists(),
            "the GUID goes along"
        );
        // A material named inside an override follows the material.
        move_asset(
            &mut state,
            &assets,
            &assets.join("materials/red.mat.ron"),
            &assets.join("materials/blue.mat.ron"),
        )
        .unwrap();
        let text = std::fs::read_to_string(&other).unwrap();
        assert!(
            text.contains("materials/blue.mat.ron") && !text.contains("red.mat"),
            "{text}"
        );
        // The instance still expands and shows no phantom override.
        assert!(state.prefab_report(cube).unwrap().overridden.is_empty());

        // Deleting the prefab keeps the objects, here and in other files.
        let n = state.scene.entities.len();
        delete_asset(&mut state, &assets, &assets.join("shared/Box.prefab.ron")).unwrap();
        assert!(
            state
                .scene
                .entities
                .iter()
                .all(|e| e.prefab.is_none() && e.prefab_node.is_none())
        );
        assert_eq!(state.scene.entities.len(), n);
        assert!(state.scene.get(cube).is_some());
        assert!(!state.is_dirty(), "the open scene was rewritten too");
        let saved = SceneFile::load(assets.join("scenes/a.scene.ron")).unwrap();
        assert_eq!(saved.entities.len(), n);
        assert!(saved.entities.iter().all(|e| e.prefab.is_none()));
        let other = SceneFile::load(&other).unwrap();
        assert!(other.entities.iter().all(|e| e.prefab.is_none()));
        assert_eq!(
            other.get(5).unwrap().name,
            "Other",
            "the instance's root stays an object"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dropping_a_prefab_instantiates_it_where_it_was_dropped() {
        let (root, mut state, cube) = project("prefab-drop");
        let assets = root.join("assets");
        let rel = state.create_prefab(cube).unwrap();
        let path = assets.join(&rel);
        let n = state.scene.entities.len();
        drop_asset(
            &mut state,
            &path,
            DropTarget::Viewport {
                pick: None,
                at: Some(Vec3::new(4.0, 0.0, 1.0)),
            },
        );
        assert_eq!(state.scene.entities.len(), n + 1);
        let placed = state.selection.unwrap();
        assert_eq!(
            state.scene.get(placed).unwrap().position,
            Vec3::new(4.0, 0.0, 1.0)
        );
        assert!(state.scene.get(placed).unwrap().prefab.is_some());
        // On a row it becomes a child.
        drop_asset(&mut state, &path, DropTarget::Hierarchy(Some(placed)));
        let child = state.selection.unwrap();
        assert_eq!(state.scene.get(child).unwrap().parent, Some(placed));
        // A prefab cannot be dropped into itself while it is the open scene.
        state.scene_path = Some(path.clone());
        let n = state.scene.entities.len();
        drop_asset(&mut state, &path, DropTarget::Hierarchy(None));
        assert_eq!(state.scene.entities.len(), n);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dropping_assets_assigns_materials_and_places_models() {
        let (root, mut state, cube) = project("drop");
        let assets = root.join("assets");
        state.scene.get_mut(cube).unwrap().material = None;
        drop_asset(
            &mut state,
            &assets.join("materials/red.mat.ron"),
            DropTarget::Viewport {
                pick: Some(cube),
                at: None,
            },
        );
        assert_eq!(
            state.scene.get(cube).unwrap().material.as_deref(),
            Some("materials/red.mat.ron")
        );
        // On empty space nothing is assigned.
        let n = state.scene.entities.len();
        drop_asset(
            &mut state,
            &assets.join("materials/red.mat.ron"),
            DropTarget::Viewport {
                pick: None,
                at: None,
            },
        );
        assert_eq!(state.scene.entities.len(), n);
        // A model lands where it was dropped; on a row it becomes a child.
        drop_asset(
            &mut state,
            &assets.join("models/robot.gltf"),
            DropTarget::Viewport {
                pick: None,
                at: Some(Vec3::new(5.0, 0.0, 3.0)),
            },
        );
        let imported: Vec<_> = state.scene.entities[n..]
            .iter()
            .filter(|e| e.parent.is_none())
            .collect();
        assert!(!imported.is_empty());
        assert!(imported.iter().all(|e| e.position.x >= 4.9));
        let n = state.scene.entities.len();
        drop_asset(
            &mut state,
            &assets.join("models/robot.gltf"),
            DropTarget::Hierarchy(Some(cube)),
        );
        assert!(
            state.scene.entities[n..]
                .iter()
                .any(|e| e.parent == Some(cube))
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
