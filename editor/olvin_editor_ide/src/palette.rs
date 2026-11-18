//! Command palette (Ctrl+Shift+P), quick open (Ctrl+E) and go to line
//! (Ctrl+G): one overlay at the top of the window. Like VS Code, typing `>`
//! switches to commands and `:` to a line number.

use olvin_editor::EditorState;
use olvin_editor::egui::{self, Align2, Key, RichText, text::LayoutJob};
use olvin_editor::theme;

use crate::files;
use crate::fuzzy;
use crate::ide::{Command, Ide};
use crate::text_ops;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Commands,
    Files,
    Line,
}

pub struct Palette {
    pub mode: Mode,
    pub query: String,
    pub selected: usize,
    opened: bool,
}

impl Palette {
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            query: String::new(),
            selected: 0,
            opened: false,
        }
    }
}

enum Item {
    Command(Command),
    File(usize),
}

/// `42` or `42:7` (also `:42`).
pub fn parse_line(query: &str) -> Option<(usize, usize)> {
    let q = query.trim().trim_start_matches(':');
    let mut parts = q.splitn(2, [':', ',']);
    let line = parts.next()?.trim().parse().ok()?;
    let col = parts
        .next()
        .and_then(|c| c.trim().parse().ok())
        .unwrap_or(1);
    Some((line, col))
}

const ROWS: usize = 12;

pub fn show(ide: &mut Ide, ctx: &egui::Context, state: &mut EditorState) {
    let Some(mut p) = ide.palette.take() else {
        return;
    };
    // Prefix switching.
    if p.mode == Mode::Files && p.query.starts_with('>') {
        p.mode = Mode::Commands;
        p.query.remove(0);
    } else if p.mode == Mode::Files && p.query.starts_with(':') {
        p.mode = Mode::Line;
        p.query.remove(0);
    }

    let files: Vec<&files::Entry> = ide.entries.iter().filter(|e| !e.is_dir).collect();
    let items: Vec<(Item, Vec<usize>)> = match p.mode {
        Mode::Commands => fuzzy::rank(&p.query, &Command::ALL, |c| c.label())
            .into_iter()
            .map(|(c, m)| (Item::Command(*c), m))
            .collect(),
        Mode::Files => {
            let indices: Vec<usize> = (0..files.len()).collect();
            fuzzy::rank(&p.query, &indices, |i| files[*i].rel.as_str())
                .into_iter()
                .take(200)
                .map(|(i, m)| (Item::File(*i), m))
                .collect()
        }
        Mode::Line => Vec::new(),
    };
    if !items.is_empty() {
        p.selected = p.selected.min(items.len() - 1);
    }

    let (up, down, enter, escape) = ctx.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
            i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
            i.consume_key(egui::Modifiers::NONE, Key::Enter),
            i.consume_key(egui::Modifiers::NONE, Key::Escape),
        )
    });
    if up && p.selected > 0 {
        p.selected -= 1;
    }
    if down && p.selected + 1 < items.len() {
        p.selected += 1;
    }

    let width = 560.0_f32.min(ctx.content_rect().width() - 32.0);
    let mut clicked: Option<usize> = None;
    let area = egui::Area::new(egui::Id::new("olvin_ide_palette"))
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_TOP, [0.0, 56.0])
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .fill(theme::RAISED)
                .show(ui, |ui| {
                    ui.set_width(width);
                    let hint = match p.mode {
                        Mode::Commands => "Type a command",
                        Mode::Files => "Search files by name (> for commands, : for a line)",
                        Mode::Line => "Line number, e.g. 42 or 42:7",
                    };
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut p.query)
                            .hint_text(hint)
                            .desired_width(f32::INFINITY),
                    );
                    if !p.opened {
                        edit.request_focus();
                        p.opened = true;
                    }
                    if edit.changed() {
                        p.selected = 0;
                    }
                    ui.add_space(4.0);
                    match p.mode {
                        Mode::Line => {
                            let total = ide
                                .active
                                .and_then(|i| ide.docs.get(i))
                                .map_or(0, |d| d.text.lines().count());
                            let msg = match parse_line(&p.query) {
                                Some((l, c)) => {
                                    format!("Go to line {l}, column {c} (of {total} lines)")
                                }
                                None => format!("Type a line number between 1 and {total}"),
                            };
                            ui.label(RichText::new(msg).color(theme::TEXT_WEAK));
                        }
                        _ if items.is_empty() => {
                            ui.label(RichText::new("No matches").color(theme::TEXT_WEAK));
                        }
                        _ => {
                            let start = p.selected.saturating_sub(ROWS - 1);
                            for (row, (item, hits)) in
                                items.iter().enumerate().skip(start).take(ROWS)
                            {
                                let (text, detail) = match item {
                                    Item::Command(c) => (c.label().to_owned(), c.keys().to_owned()),
                                    Item::File(i) => (files[*i].rel.clone(), String::new()),
                                };
                                let resp = row_ui(ui, &text, hits, &detail, row == p.selected);
                                if resp.clicked() {
                                    clicked = Some(row);
                                }
                                if resp.hovered()
                                    && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO)
                                {
                                    p.selected = row;
                                }
                            }
                            if items.len() > ROWS {
                                ui.label(
                                    RichText::new(format!("{} results", items.len()))
                                        .small()
                                        .color(theme::TEXT_WEAK),
                                );
                            }
                        }
                    }
                });
        });

    let clicked_outside = ctx.input(|i| i.pointer.any_pressed())
        && ctx
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|pos| !area.response.rect.contains(pos));
    if escape || clicked_outside {
        return; // closed
    }
    let chosen = clicked.or(enter.then_some(p.selected));
    match (p.mode, chosen) {
        (Mode::Line, _) if enter => {
            if let Some((line, col)) = parse_line(&p.query)
                && let Some(doc) = ide.active_doc()
            {
                doc.goto = Some(text_ops::line_col_to_char(&doc.text, line, col));
                ide.requests.push_back(crate::ide::EditorRequest::Focus);
            }
        }
        (Mode::Commands, Some(row)) if row < items.len() => {
            if let Item::Command(c) = items[row].0 {
                ide.run(c, state);
            }
        }
        (Mode::Files, Some(row)) if row < items.len() => {
            if let Item::File(i) = items[row].0 {
                let path = files[i].path.clone();
                ide.open(&path, state);
            }
        }
        _ => {
            // Still open (unless a command opened another palette).
            if ide.palette.is_none() {
                ide.palette = Some(p);
            }
        }
    }
}

fn row_ui(
    ui: &mut egui::Ui,
    text: &str,
    hits: &[usize],
    detail: &str,
    selected: bool,
) -> egui::Response {
    let height = 22.0;
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::click(),
    );
    if selected {
        ui.painter()
            .rect_filled(rect, 5.0, theme::ACCENT.gamma_multiply(0.45));
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 5.0, theme::SURFACE);
    }
    let font = egui::TextStyle::Body.resolve(ui.style());
    let mut job = LayoutJob::default();
    for (ci, ch) in text.chars().enumerate() {
        let hit = hits.contains(&ci);
        let color = if hit {
            theme::ACCENT.gamma_multiply(1.3)
        } else {
            theme::TEXT
        };
        let mut buf = [0u8; 4];
        job.append(
            ch.encode_utf8(&mut buf),
            0.0,
            egui::TextFormat {
                font_id: font.clone(),
                color,
                underline: if hit {
                    egui::Stroke::new(1.0, color)
                } else {
                    egui::Stroke::NONE
                },
                ..Default::default()
            },
        );
    }
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let y = rect.center().y - galley.size().y / 2.0;
    ui.painter()
        .galley(egui::pos2(rect.left() + 8.0, y), galley, theme::TEXT);
    if !detail.is_empty() {
        ui.painter().text(
            egui::pos2(rect.right() - 8.0, rect.center().y),
            Align2::RIGHT_CENTER,
            detail,
            egui::TextStyle::Small.resolve(ui.style()),
            theme::TEXT_WEAK,
        );
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_queries() {
        assert_eq!(parse_line("42"), Some((42, 1)));
        assert_eq!(parse_line(" 42:7 "), Some((42, 7)));
        assert_eq!(parse_line(":5"), Some((5, 1)));
        assert_eq!(parse_line("x"), None);
        assert_eq!(parse_line(""), None);
    }
}
