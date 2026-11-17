use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{Align, Layout, RichText, Sense, TextEdit};

use crate::{EditorState, Panel, theme};

const RESCAN: Duration = Duration::from_secs(1);
const TILE: egui::Vec2 = egui::vec2(84.0, 76.0);

struct Entry {
    name: String,
    path: PathBuf,
    is_dir: bool,
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
}

impl Panel for Assets {
    fn ui(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        let Some(root) = state.assets_dir() else {
            ui.centered_and_justified(|ui| {
                ui.weak("No project open: run olvin-editor <project folder> or open it from the hub")
            });
            return;
        };
        let dir = root.join(&self.current);
        let stale = self.scanned.as_ref().is_none_or(|(d, t)| *d != dir || t.elapsed() > RESCAN);
        if stale {
            self.rescan(&dir);
        }

        self.toolbar(ui, state, &dir);
        ui.separator();

        let filter = self.filter.trim().to_lowercase();
        let mut open = None;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            if self.entries.is_empty() {
                ui.weak("Empty folder. Right-click or \"+\" to create a folder or a scene");
            }
            ui.horizontal_wrapped(|ui| {
                for entry in self.entries.iter().filter(|e| filter.is_empty() || e.name.to_lowercase().contains(&filter)) {
                    let selected = self.selected.as_ref() == Some(&entry.path);
                    let response = tile(ui, entry, selected);
                    if response.clicked() {
                        self.selected = Some(entry.path.clone());
                    }
                    if response.double_clicked() {
                        open = Some((entry.path.clone(), entry.is_dir));
                    }
                    response.context_menu(|ui| {
                        if is_scene(&entry.path) && ui.button("Open scene").clicked() {
                            state.request_open(entry.path.clone());
                        }
                        if ui.button("Show in file manager").clicked() {
                            reveal(entry.path.parent().unwrap_or(&entry.path));
                        }
                    });
                }
            });
            let rest = ui.allocate_response(ui.available_size().max(egui::vec2(0.0, 30.0)), Sense::click());
            rest.context_menu(|ui| create_menu(ui, &mut self.creating));
        });

        if let Some((path, is_dir)) = open {
            if is_dir {
                if let Ok(rel) = path.strip_prefix(&root) {
                    self.current = rel.to_path_buf();
                }
                self.scanned = None;
            } else if is_scene(&path) {
                state.request_open(path);
            } else {
                reveal(&path);
            }
        }
    }
}

impl Assets {
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
        self.entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        self.scanned = Some((dir.to_path_buf(), Instant::now()));
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, state: &mut EditorState, dir: &Path) {
        ui.horizontal(|ui| {
            if ui.add_enabled(!self.current.as_os_str().is_empty(), egui::Button::new("⬆")).on_hover_text("Up").clicked() {
                self.current.pop();
            }
            // Breadcrumbs: assets / a / b
            if ui.link("assets").clicked() {
                self.current = PathBuf::new();
            }
            let parts: Vec<std::ffi::OsString> =
                self.current.components().map(|c| c.as_os_str().to_owned()).collect();
            for (i, part) in parts.iter().enumerate() {
                ui.weak("/");
                if ui.link(part.to_string_lossy()).clicked() {
                    self.current = parts[..=i].iter().collect();
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.menu_button("+", |ui| create_menu(ui, &mut self.creating));
                if ui.button("⟳").on_hover_text("Refresh").clicked() {
                    self.scanned = None;
                }
                ui.add(TextEdit::singleline(&mut self.filter).hint_text("🔍 Filter").desired_width(140.0));
            });
        });

        if let Some((is_folder, mut name)) = self.creating.take() {
            let mut keep = true;
            ui.horizontal(|ui| {
                ui.label(if is_folder { "New folder:" } else { "New scene:" });
                let edit = ui.add(TextEdit::singleline(&mut name).desired_width(180.0));
                edit.request_focus();
                let submit = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
                if submit && !name.trim().is_empty() {
                    let name = name.trim().to_owned();
                    if is_folder {
                        if let Err(e) = std::fs::create_dir_all(dir.join(&name)) {
                            state.error(format!("could not create folder: {e}"));
                        }
                    } else {
                        let file = if name.ends_with(".scene.ron") { name } else { format!("{name}.scene.ron") };
                        let path = dir.join(file);
                        if path.exists() {
                            state.error(format!("{} already exists", path.display()));
                        } else if olvin_scene::file::SceneFile::default().save(&path).is_ok() {
                            state.request_open(path);
                        }
                    }
                    keep = false;
                    self.scanned = None;
                } else if cancel || (edit.lost_focus() && !submit) {
                    keep = false;
                }
            });
            if keep {
                self.creating = Some((is_folder, name));
            }
        }
    }

}

fn create_menu(ui: &mut egui::Ui, creating: &mut Option<(bool, String)>) {
    if ui.button("📁 Folder").clicked() {
        *creating = Some((true, String::new()));
    }
    if ui.button("🎬 Scene").clicked() {
        *creating = Some((false, String::new()));
    }
}

fn tile(ui: &mut egui::Ui, entry: &Entry, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(TILE, Sense::click());
    let visuals = ui.style().interact_selectable(&response, selected);
    if selected || response.hovered() {
        let fill = if selected { theme::ACCENT.gamma_multiply(0.55) } else { visuals.weak_bg_fill };
        ui.painter().rect_filled(rect, 4.0, fill);
    }
    let painter = ui.painter();
    painter.text(
        rect.center_top() + egui::vec2(0.0, 26.0),
        egui::Align2::CENTER_CENTER,
        icon(entry),
        egui::FontId::proportional(28.0),
        ui.visuals().text_color(),
    );
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
    let ext = entry.path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    match ext.as_str() {
        _ if is_scene(&entry.path) => "🎬",
        "png" | "jpg" | "jpeg" | "tga" | "bmp" | "webp" | "ktx2" => "🖼",
        "gltf" | "glb" | "obj" | "fbx" => "📦",
        "wav" | "ogg" | "mp3" | "flac" => "🔊",
        "ron" | "toml" | "json" => "📝",
        _ => "📄",
    }
}

fn is_scene(path: &Path) -> bool {
    path.to_string_lossy().ends_with(".scene.ron")
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
