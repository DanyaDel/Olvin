//! Code editor panel: tabs, a `TextEdit` with a highlighting layouter, line
//! numbers, problem squiggles, find/replace and editing helpers.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;

use olvin_editor::egui::text::{CCursor, CCursorRange};
use olvin_editor::egui::text_edit::TextEditState;
use olvin_editor::egui::{self, Align2, Color32, Event, Key, RichText, Shape, Stroke, TextStyle};
use olvin_editor::{EditorState, Panel, theme};

use crate::diagnostics::Severity;
use crate::document::Conflict;
use crate::files;
use crate::highlight;
use crate::ide::{self, Command, EditorRequest, Ide, Marker};
use crate::keys::KeyRouter;
use crate::lang::{Lang, Token, tokenize};
use crate::text_ops::{self, FindOptions};

#[derive(Default)]
pub struct CodeEditor {
    find: FindBar,
    /// (text hash, lang) → tokens.
    tokens: (u64, Option<Lang>, Vec<Token>),
    /// Longest line in chars, for the horizontal scroll width.
    widest: (u64, usize),
    scroll_to: Option<usize>,
}

#[derive(Default)]
struct FindBar {
    open: bool,
    replace: bool,
    query: String,
    replacement: String,
    opts: FindOptions,
    current: Option<usize>,
    focus: bool,
    /// (text hash, query, opts) the matches belong to.
    key: u64,
    matches: Vec<Range<usize>>,
}

/// What a key press in the editor asks the IDE (outside the text) to do.
enum KeyAction {
    OpenFind { replace: bool, seed: Option<String> },
    CloseFind,
    Run(Command),
}

pub fn text_hash(text: &str) -> u64 {
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

impl Panel for CodeEditor {
    fn ui(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        ide::with(|ide| {
            ide.frame(ui, state);
            self.draw(ui, state, ide);
        });
    }
}

/// Check / Build / Run / Stop and the last result; also used by Problems.
pub fn build_toolbar(ui: &mut egui::Ui, ide: &mut Ide, state: &mut EditorState) {
    let running = ide.job_running();
    let mut cmd = None;
    ui.add_enabled_ui(!running, |ui| {
        if ui.button("✔ Check").on_hover_text("cargo check").clicked() {
            cmd = Some(Command::Check);
        }
        if ui
            .button("🔨 Build")
            .on_hover_text("cargo build (Ctrl+Shift+B)")
            .clicked()
        {
            cmd = Some(Command::Build);
        }
        if ui
            .button(RichText::new("▶ Run").color(theme::AXIS_Y))
            .on_hover_text("cargo run: starts the game")
            .clicked()
        {
            cmd = Some(Command::Run);
        }
    });
    if ui
        .add_enabled(
            running,
            egui::Button::new(RichText::new("⏹ Stop").color(ui.visuals().error_fg_color)),
        )
        .on_hover_text("Stop the build or the game")
        .clicked()
    {
        cmd = Some(Command::Stop);
    }
    match (&ide.job, ide.last_job) {
        (Some(job), _) if job.running() => {
            ui.spinner();
            ui.label(
                RichText::new(format!(
                    "{}… {:.0} s",
                    job.kind.label(),
                    job.started.elapsed().as_secs_f32()
                ))
                .color(theme::TEXT_WEAK),
            );
        }
        (_, Some((kind, ok))) => {
            let (e, w) = ide.counts();
            let color = if ok {
                theme::AXIS_Y
            } else {
                ui.visuals().error_fg_color
            };
            ui.label(
                RichText::new(format!(
                    "{} {}",
                    kind.label(),
                    if ok { "ok" } else { "failed" }
                ))
                .color(color),
            )
            .on_hover_text(format!("{e} errors, {w} warnings"));
        }
        _ => {}
    }
    if let Some(c) = cmd {
        ide.run(c, state);
    }
}

impl CodeEditor {
    fn draw(&mut self, ui: &mut egui::Ui, state: &mut EditorState, ide: &mut Ide) {
        // Requests from the palette / other panels.
        let mut focus = false;
        let mut comment = false;
        while let Some(r) = ide.requests.pop_front() {
            match r {
                EditorRequest::Find => self.open_find(false, None),
                EditorRequest::Replace => self.open_find(true, None),
                EditorRequest::ToggleComment => comment = true,
                EditorRequest::Focus => focus = true,
            }
        }

        self.tabs(ui, ide, state);

        let Some(idx) = ide.active.filter(|&i| i < ide.docs.len()) else {
            welcome(ui);
            return;
        };

        bars(ui, ide, idx, state);
        if self.find.open {
            self.find_bar(ui, ide, idx);
        }
        let status_id = ui.id().with("ide_code_status");
        let markers = ide.markers(&ide.docs[idx]);
        let edit_id = ui.id().with(("ide_code", &ide.docs[idx].path));

        egui::Panel::bottom(status_id)
            .resizable(false)
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(4, 2)))
            .show(ui, |ui| status_bar(ui, ide, idx, edit_id, &markers));

        let actions = self.editor(ui, ide, idx, edit_id, focus, comment, &markers);
        for a in actions {
            match a {
                KeyAction::OpenFind { replace, seed } => self.open_find(replace, seed),
                KeyAction::CloseFind => self.find.open = false,
                KeyAction::Run(c) => ide.run(c, state),
            }
        }
    }

    fn open_find(&mut self, replace: bool, seed: Option<String>) {
        self.find.open = true;
        self.find.replace = replace;
        self.find.focus = true;
        if let Some(s) = seed.filter(|s| !s.is_empty() && !s.contains('\n')) {
            self.find.query = s;
        }
    }

    fn tabs(&mut self, ui: &mut egui::Ui, ide: &mut Ide, state: &mut EditorState) {
        let mut activate = None;
        let mut close = None;
        let mut reveal = false;
        ui.horizontal(|ui| {
            let toolbar_w = 330.0_f32.min(ui.available_width() * 0.5);
            egui::ScrollArea::horizontal()
                .id_salt("ide_tabs")
                .max_width(ui.available_width() - toolbar_w)
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for (i, doc) in ide.docs.iter().enumerate() {
                            let active = ide.active == Some(i);
                            let (glyph, color) = files::icon(&doc.name(), false, false);
                            let mut job = egui::text::LayoutJob::default();
                            job.append(
                                glyph,
                                0.0,
                                egui::TextFormat::simple(
                                    TextStyle::Body.resolve(ui.style()),
                                    color,
                                ),
                            );
                            let name_color = if active {
                                Color32::WHITE
                            } else {
                                theme::TEXT_WEAK
                            };
                            job.append(
                                &doc.name(),
                                5.0,
                                egui::TextFormat::simple(
                                    TextStyle::Body.resolve(ui.style()),
                                    name_color,
                                ),
                            );
                            let tab = ui
                                .add(egui::Button::selectable(active, job))
                                .on_hover_text(ide.rel(&doc.path));
                            if tab.clicked() {
                                activate = Some(i);
                            }
                            if tab.middle_clicked() {
                                close = Some(i);
                            }
                            tab.context_menu(|ui| {
                                if ui.button("Close").clicked() {
                                    close = Some(i);
                                }
                                if ui.button("Reveal in Explorer").clicked() {
                                    activate = Some(i);
                                    reveal = true;
                                }
                                if ui.button("Copy Path").clicked() {
                                    ui.ctx().copy_text(doc.path.display().to_string());
                                }
                            });
                            let glyph = if doc.dirty { "⏺" } else { "×" };
                            let x = ui
                                .add(
                                    egui::Button::new(RichText::new(glyph).color(theme::TEXT_WEAK))
                                        .frame(false)
                                        .small(),
                                )
                                .on_hover_text(if doc.dirty {
                                    "Unsaved changes: close"
                                } else {
                                    "Close"
                                });
                            if x.clicked() {
                                close = Some(i);
                            }
                            ui.add_space(6.0);
                        }
                    });
                });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                // Right-to-left: reversed.
                ui.horizontal(|ui| build_toolbar(ui, ide, state));
            });
        });
        if let Some(i) = activate {
            ide.active = Some(i);
            if reveal {
                ide.reveal_active();
            }
        }
        if let Some(i) = close {
            ide.close(i, false);
        }
    }

    fn find_bar(&mut self, ui: &mut egui::Ui, ide: &mut Ide, idx: usize) {
        let doc = &mut ide.docs[idx];
        let f = &mut self.find;
        let mut go: Option<bool> = None; // Some(backwards)
        let mut replace_one = false;
        let mut replace_all = false;
        let mut focused = false;
        egui::Frame::new()
            .fill(theme::RAISED)
            .corner_radius(6)
            .inner_margin(egui::Margin::symmetric(6, 4))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let q = ui.add(
                        egui::TextEdit::singleline(&mut f.query)
                            .hint_text("Find")
                            .desired_width(200.0),
                    );
                    if std::mem::take(&mut f.focus) {
                        q.request_focus();
                    }
                    focused |= q.has_focus();
                    if q.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        go = Some(ui.input(|i| i.modifiers.shift));
                        q.request_focus();
                    }
                    ui.toggle_value(&mut f.opts.case_sensitive, "Aa")
                        .on_hover_text("Match Case");
                    ui.toggle_value(&mut f.opts.whole_word, "ab")
                        .on_hover_text("Match Whole Word");
                    let count = match (f.current, f.matches.len()) {
                        (_, 0) if f.query.is_empty() => String::new(),
                        (_, 0) => "No results".to_owned(),
                        (Some(c), n) => format!("{} of {n}", c + 1),
                        (None, n) => format!("{n} results"),
                    };
                    ui.label(RichText::new(count).color(theme::TEXT_WEAK));
                    if ui
                        .small_button("⬆")
                        .on_hover_text("Previous (Shift+Enter)")
                        .clicked()
                    {
                        go = Some(true);
                    }
                    if ui.small_button("⬇").on_hover_text("Next (Enter)").clicked() {
                        go = Some(false);
                    }
                    ui.toggle_value(&mut f.replace, "🔁")
                        .on_hover_text("Toggle Replace (Ctrl+H)");
                    if ui.small_button("×").on_hover_text("Close (Esc)").clicked() {
                        f.open = false;
                    }
                });
                if f.replace {
                    ui.horizontal(|ui| {
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut f.replacement)
                                .hint_text("Replace")
                                .desired_width(200.0),
                        );
                        focused |= r.has_focus();
                        let read_only = doc.read_only.is_some();
                        ui.add_enabled_ui(!read_only, |ui| {
                            replace_one = ui.button("Replace").clicked();
                            replace_all = ui.button("Replace All").clicked();
                        });
                    });
                }
            });
        if focused {
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                f.open = false;
            }
            ui.ctx()
                .plugin_or_default::<KeyRouter>()
                .lock()
                .code_focused = true;
        }

        // Matches for the current text.
        let key = {
            let mut h = DefaultHasher::new();
            doc.text.hash(&mut h);
            f.query.hash(&mut h);
            f.opts.case_sensitive.hash(&mut h);
            f.opts.whole_word.hash(&mut h);
            h.finish()
        };
        if key != f.key {
            f.key = key;
            f.matches = text_ops::find_all(&doc.text, &f.query, f.opts);
            f.current = f.current.filter(|&c| c < f.matches.len());
        }

        let edit_id = ui.id().with(("ide_code", &doc.path));
        let cursor_byte = |doc_text: &str, end: bool| {
            TextEditState::load(ui.ctx(), edit_id)
                .and_then(|s| s.cursor.char_range())
                .map(|r| {
                    let [a, b] = r.sorted_cursors();
                    text_ops::char_to_byte(doc_text, if end { b.index.0 } else { a.index.0 })
                })
                .unwrap_or(0)
        };
        if replace_all && !f.query.is_empty() {
            let (text, n) = text_ops::replace_all(&doc.text, &f.query, &f.replacement, f.opts);
            if n > 0 {
                doc.set_text(text);
            }
            f.current = None;
            f.key = 0;
            return;
        }
        if replace_one
            && let Some(c) = f.current
            && let Some(m) = f.matches.get(c).cloned()
        {
            let mut text = doc.text.clone();
            text.replace_range(m.clone(), &f.replacement);
            doc.set_text(text);
            let at = m.start + f.replacement.len();
            f.matches = text_ops::find_all(&doc.text, &f.query, f.opts);
            f.key = 0;
            f.current = text_ops::next_match(&f.matches, at, false);
            if let Some(c) = f.current {
                self.scroll_to = Some(select(ui, edit_id, &doc.text, f.matches[c].clone()));
            }
            return;
        }
        if let Some(back) = go {
            let from = match f.current.and_then(|c| f.matches.get(c)) {
                Some(m) if back => m.start,
                Some(m) => m.end,
                None => cursor_byte(&doc.text, !back),
            };
            f.current = text_ops::next_match(&f.matches, from, back);
            if let Some(c) = f.current {
                let m = f.matches[c].clone();
                let text = doc.text.clone();
                self.scroll_to = Some(select(ui, edit_id, &text, m));
            }
        }
    }
}

/// Selects a byte range in the editor; returns the char index to scroll to.
fn select(ui: &egui::Ui, edit_id: egui::Id, text: &str, bytes: Range<usize>) -> usize {
    let a = text_ops::byte_to_char(text, bytes.start);
    let b = text_ops::byte_to_char(text, bytes.end);
    let mut st = TextEditState::load(ui.ctx(), edit_id).unwrap_or_default();
    st.cursor
        .set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
    st.store(ui.ctx(), edit_id);
    a
}

impl CodeEditor {
    #[allow(clippy::too_many_arguments)]
    fn editor(
        &mut self,
        ui: &mut egui::Ui,
        ide: &mut Ide,
        idx: usize,
        edit_id: egui::Id,
        focus: bool,
        comment: bool,
        markers: &[Marker],
    ) -> Vec<KeyAction> {
        let ctx = ui.ctx().clone();
        let doc = &mut ide.docs[idx];
        let read_only = doc.read_only.is_some();
        let mut actions = Vec::new();

        if let Some(ci) = doc.goto.take() {
            let mut st = TextEditState::load(&ctx, edit_id).unwrap_or_default();
            st.cursor
                .set_char_range(Some(CCursorRange::one(CCursor::new(ci))));
            st.store(&ctx, edit_id);
            self.scroll_to = Some(ci);
        }
        if focus {
            ctx.memory_mut(|m| m.request_focus(edit_id));
        }

        let has_focus = ctx.memory(|m| m.has_focus(edit_id));
        if has_focus {
            ctx.plugin_or_default::<KeyRouter>().lock().code_focused = true;
        }

        // --- keys, handled before the TextEdit sees them -------------------
        let mut st = TextEditState::load(&ctx, edit_id).unwrap_or_default();
        let mut sel = st.cursor.char_range().map(|r| {
            let [a, b] = r.sorted_cursors();
            a.index.0..b.index.0
        });
        let mut changed = false;
        let mut sel_changed = false;
        let comment_prefix = doc.lang.line_comment();
        if comment
            && !read_only
            && let (Some(s), Some(prefix)) = (sel.clone(), comment_prefix)
        {
            let e = text_ops::toggle_comment(&doc.text, s, prefix);
            doc.text = e.text;
            sel = Some(e.sel);
            changed = true;
        }
        if has_focus && let Some(s0) = sel.clone() {
            let mut s = s0;
            let text = &mut doc.text;
            let find_open = self.find.open;
            let mut apply = |e: text_ops::Edited, text: &mut String, s: &mut Range<usize>| {
                *text = e.text;
                *s = e.sel;
                changed = true;
            };
            ctx.input_mut(|i| {
                i.events.retain(|ev| match ev {
                    Event::Key {
                        key,
                        pressed: true,
                        modifiers: m,
                        ..
                    } => {
                        let cmd = m.command && !m.alt;
                        match key {
                            Key::F if cmd && !m.shift => {
                                let seed = text_ops_slice(text, &s);
                                actions.push(KeyAction::OpenFind {
                                    replace: false,
                                    seed,
                                });
                                false
                            }
                            Key::H if cmd && !m.shift => {
                                let seed = text_ops_slice(text, &s);
                                actions.push(KeyAction::OpenFind {
                                    replace: true,
                                    seed,
                                });
                                false
                            }
                            Key::G if cmd && !m.shift => {
                                actions.push(KeyAction::Run(Command::GoToLine));
                                false
                            }
                            Key::W if cmd && !m.shift => {
                                actions.push(KeyAction::Run(Command::CloseTab));
                                false
                            }
                            Key::Tab if cmd => {
                                actions.push(KeyAction::Run(if m.shift {
                                    Command::PrevTab
                                } else {
                                    Command::NextTab
                                }));
                                false
                            }
                            Key::Slash if cmd && !read_only => {
                                if let Some(prefix) = comment_prefix {
                                    let e = text_ops::toggle_comment(text, s.clone(), prefix);
                                    apply(e, text, &mut s);
                                }
                                false
                            }
                            Key::D if cmd && !m.shift => {
                                if let Some(r) = next_occurrence(text, &s) {
                                    s = r;
                                    sel_changed = true;
                                }
                                false
                            }
                            Key::Tab if !m.alt && !read_only => {
                                let e = if m.shift {
                                    text_ops::outdent(text, s.clone())
                                } else {
                                    text_ops::indent(text, s.clone())
                                };
                                apply(e, text, &mut s);
                                false
                            }
                            Key::Enter if !m.command && !m.alt && !read_only => {
                                let e = text_ops::newline(text, s.clone());
                                apply(e, text, &mut s);
                                false
                            }
                            Key::Backspace if m.is_none() && !read_only => {
                                match text_ops::backspace_pair(text, s.clone()) {
                                    Some(e) => {
                                        apply(e, text, &mut s);
                                        false
                                    }
                                    None => true,
                                }
                            }
                            Key::Escape if find_open => {
                                actions.push(KeyAction::CloseFind);
                                false
                            }
                            _ => true,
                        }
                    }
                    Event::Text(t) if !read_only && t.chars().count() == 1 => {
                        let ch = t.chars().next().unwrap_or(' ');
                        match text_ops::type_char(text, s.clone(), ch) {
                            Some(e) => {
                                apply(e, text, &mut s);
                                false
                            }
                            None => true,
                        }
                    }
                    _ => true,
                });
            });
            sel = Some(s);
        }
        if changed {
            doc.edited();
        }
        if (changed || sel_changed)
            && let Some(s) = &sel
        {
            st.cursor.set_char_range(Some(CCursorRange::two(
                CCursor::new(s.start),
                CCursor::new(s.end),
            )));
            st.store(&ctx, edit_id);
            self.scroll_to = Some(s.end);
        }

        // --- layout ------------------------------------------------------------
        let font = TextStyle::Monospace.resolve(ui.style());
        let (row_h, char_w) = ui.fonts_mut(|f| (f.row_height(&font), f.glyph_width(&font, '0')));
        let hash = text_hash(&doc.text);
        if self.widest.0 != hash {
            let widest = doc
                .text
                .split('\n')
                .map(|l| l.chars().count() + l.matches('\t').count() * 3)
                .max()
                .unwrap_or(0);
            self.widest = (hash, widest);
        }
        let lines = doc.text.matches('\n').count() + 1;
        let digits = lines.to_string().len().max(3);
        let gutter = (digits as f32 * char_w + 24.0).min(120.0);

        let lang = doc.lang;
        let highlight_on = !read_only;
        let (find_key_hash, matches, current) = if self.find.open {
            (hash, self.find.matches.clone(), self.find.current)
        } else {
            (0, Vec::new(), None)
        };
        let tokens = &mut self.tokens;
        let font_for_layout = font.clone();
        let mut layouter = move |ui: &egui::Ui, buf: &dyn egui::TextBuffer, _wrap: f32| {
            let text = buf.as_str();
            let h = text_hash(text);
            if highlight_on && (tokens.0 != h || tokens.1 != Some(lang)) {
                *tokens = (h, Some(lang), tokenize(lang, text));
            }
            let toks: &[Token] = if highlight_on && tokens.0 == h {
                &tokens.2
            } else {
                &[]
            };
            let m: &[Range<usize>] = if h == find_key_hash { &matches } else { &[] };
            let job = highlight::layout_job(text, toks, m, current, font_for_layout.clone());
            ui.fonts_mut(|f| f.layout_job(job))
        };

        let mut scroll_to = self.scroll_to.take();
        let widest = self.widest.1;
        let code_bg = ui.visuals().code_bg_color;
        egui::ScrollArea::both()
            .id_salt(("ide_code_scroll", &doc.path))
            .auto_shrink(false)
            .show(ui, |ui| {
                let bg_idx = ui.painter().add(Shape::Noop);
                let avail = ui.available_size();
                let width = (widest as f32 * char_w + gutter + 48.0).max(avail.x);
                let rows = ((avail.y - 8.0) / row_h).max(1.0) as usize;
                let frame = egui::Frame::NONE.inner_margin(egui::Margin {
                    left: gutter as i8,
                    right: 8,
                    top: 4,
                    bottom: 4,
                });
                let mut ro_text: &str = doc.text.as_str();
                let out = if read_only {
                    egui::TextEdit::multiline(&mut ro_text)
                } else {
                    egui::TextEdit::multiline(&mut doc.text)
                }
                .id(edit_id)
                .lock_focus(true)
                .frame(frame)
                .desired_width(width)
                .desired_rows(rows)
                .layouter(&mut layouter)
                .show(ui);
                let resp = out.response.response.clone();
                if resp.changed() && !read_only {
                    doc.edited();
                }
                let galley = &out.galley;
                let gpos = out.galley_pos;
                let clip = ui.clip_rect();
                let full = egui::Rect::from_min_max(
                    egui::pos2(clip.left(), resp.rect.top()),
                    egui::pos2(clip.right(), resp.rect.bottom().max(clip.bottom())),
                );
                let mut bg = vec![Shape::rect_filled(full, 0.0, code_bg)];

                let cursor = out.state.cursor.char_range().map(|r| r.primary);
                let cursor_line = cursor.map(|c| {
                    let r = galley.pos_from_cursor(c);
                    ((r.center().y) / row_h).floor() as usize
                });
                if let (Some(c), true) = (cursor, resp.has_focus()) {
                    let r = galley.pos_from_cursor(c).translate(gpos.to_vec2());
                    let line = egui::Rect::from_x_y_ranges(clip.x_range(), r.y_range());
                    bg.push(Shape::rect_filled(line, 0.0, Color32::from_white_alpha(9)));
                }
                ui.painter().set(bg_idx, Shape::Vec(bg));

                // Gutter.
                let gutter_rect =
                    egui::Rect::from_x_y_ranges(clip.left()..=gpos.x - 6.0, full.y_range());
                let p = ui.painter();
                p.rect_filled(gutter_rect, 0.0, code_bg);
                let first = ((clip.top() - gpos.y) / row_h).floor().max(0.0) as usize;
                let last =
                    (((clip.bottom() - gpos.y) / row_h).ceil() as usize + 1).min(galley.rows.len());
                for (i, row) in galley.rows.iter().enumerate().take(last).skip(first) {
                    let y = gpos.y + row.pos.y + row_h / 2.0;
                    let current = cursor_line == Some(i);
                    p.text(
                        egui::pos2(gpos.x - 14.0, y),
                        Align2::RIGHT_CENTER,
                        (i + 1).to_string(),
                        font.clone(),
                        if current {
                            theme::TEXT
                        } else {
                            theme::TEXT_WEAK.gamma_multiply(0.7)
                        },
                    );
                }

                // Problems: gutter dots and squiggles.
                let hover = ui.input(|i| i.pointer.hover_pos());
                let mut tip: Option<String> = None;
                let line_starts = if markers.is_empty() {
                    Vec::new()
                } else {
                    line_starts(&doc.text)
                };
                for m in markers {
                    let li = m.line.saturating_sub(1);
                    let Some(row) = galley.rows.get(li) else {
                        continue;
                    };
                    let color = severity_color(ui, m.severity);
                    let y = gpos.y + row.pos.y + row_h / 2.0;
                    let dot = egui::pos2(clip.left() + 7.0, y);
                    p.circle_filled(dot, 3.5, color);
                    let line_len = row.char_count_excluding_newline().0;
                    let col = (m.column.saturating_sub(1)).min(line_len);
                    let end = col.saturating_add(m.len).min(line_len).max(col + 1);
                    let base = line_starts[li];
                    let a = galley
                        .pos_from_cursor(CCursor::new(base + col))
                        .translate(gpos.to_vec2());
                    let mut x1 = galley
                        .pos_from_cursor(CCursor::new(base + end.min(line_len)))
                        .translate(gpos.to_vec2())
                        .left();
                    if end > line_len || x1 <= a.left() {
                        x1 = a.left() + char_w;
                    }
                    let yb = a.bottom() - 1.0;
                    squiggle(p, a.left(), x1, yb, color);
                    let hit = egui::Rect::from_min_max(
                        egui::pos2(a.left(), a.top()),
                        egui::pos2(x1, a.bottom()),
                    );
                    let gutter_hit = egui::Rect::from_center_size(dot, egui::vec2(14.0, row_h));
                    if hover.is_some_and(|h| hit.contains(h) || gutter_hit.contains(h)) {
                        tip = Some(m.message.clone());
                    }
                }
                if let Some(t) = tip {
                    resp.clone().on_hover_text_at_pointer(t);
                }

                if let Some(ci) = scroll_to.take() {
                    let r = galley
                        .pos_from_cursor(CCursor::new(ci))
                        .translate(gpos.to_vec2());
                    ui.scroll_to_rect(
                        r.expand2(egui::vec2(gutter + 40.0, row_h * 3.0)),
                        Some(egui::Align::Center),
                    );
                }
            });
        actions
    }
}

fn text_ops_slice(text: &str, s: &Range<usize>) -> Option<String> {
    let a = text_ops::char_to_byte(text, s.start);
    let b = text_ops::char_to_byte(text, s.end);
    if a < b {
        Some(text[a..b].to_owned())
    } else {
        text_ops::word_at(text, s.start).map(|r| text[r].to_owned())
    }
}

/// Ctrl+D: the word at the cursor, or the next occurrence of the selection.
pub fn next_occurrence(text: &str, s: &Range<usize>) -> Option<Range<usize>> {
    if s.start == s.end {
        let r = text_ops::word_at(text, s.start)?;
        return Some(text_ops::byte_to_char(text, r.start)..text_ops::byte_to_char(text, r.end));
    }
    let a = text_ops::char_to_byte(text, s.start);
    let b = text_ops::char_to_byte(text, s.end);
    let needle = &text[a..b];
    let opts = FindOptions {
        case_sensitive: true,
        whole_word: false,
    };
    let matches = text_ops::find_all(text, needle, opts);
    let i = text_ops::next_match(&matches, b, false)?;
    let m = &matches[i];
    Some(text_ops::byte_to_char(text, m.start)..text_ops::byte_to_char(text, m.end))
}

/// Char index of the start of every line.
fn line_starts(text: &str) -> Vec<usize> {
    let mut out = vec![0];
    for (ci, ch) in text.chars().enumerate() {
        if ch == '\n' {
            out.push(ci + 1);
        }
    }
    out
}

fn squiggle(p: &egui::Painter, x0: f32, x1: f32, y: f32, color: Color32) {
    let mut pts = Vec::new();
    let mut x = x0;
    let mut up = false;
    while x < x1 {
        pts.push(egui::pos2(x, if up { y - 1.5 } else { y + 1.0 }));
        x += 2.5;
        up = !up;
    }
    pts.push(egui::pos2(x1, y));
    p.add(Shape::line(pts, Stroke::new(1.0, color)));
}

pub fn severity_color(ui: &egui::Ui, s: Severity) -> Color32 {
    match s {
        Severity::Error => ui.visuals().error_fg_color,
        Severity::Warning => ui.visuals().warn_fg_color,
        Severity::Note => theme::ACCENT,
    }
}

fn welcome(ui: &mut egui::Ui) {
    ui.add_space(40.0);
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new("No file open")
                .heading()
                .color(theme::TEXT_WEAK),
        );
        ui.add_space(10.0);
        for (keys, what) in [
            ("Ctrl+E", "Go to File"),
            ("Ctrl+Shift+P", "Show All Commands"),
            ("Ctrl+S", "Save (while editing code)"),
            ("Ctrl+F / Ctrl+H", "Find / Replace"),
            ("Ctrl+G", "Go to Line"),
            ("Ctrl+/", "Toggle Comment"),
            ("Ctrl+Tab", "Next Tab"),
        ] {
            ui.label(RichText::new(format!("{what}   {keys}")).color(theme::TEXT_WEAK));
        }
    });
}

/// Conflict, read-only and shader status bars above the text.
fn bars(ui: &mut egui::Ui, ide: &mut Ide, idx: usize, state: &mut EditorState) {
    let doc = &mut ide.docs[idx];
    let bar = |ui: &mut egui::Ui, color: Color32, add: &mut dyn FnMut(&mut egui::Ui)| {
        egui::Frame::new()
            .fill(color.gamma_multiply(0.18))
            .corner_radius(6)
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| add(ui));
            });
    };
    let warn = ui.visuals().warn_fg_color;
    let err = ui.visuals().error_fg_color;
    match doc.conflict {
        Some(Conflict::Changed) => bar(ui, warn, &mut |ui| {
            ui.label(
                RichText::new("⚠ The file changed on disk and you have unsaved edits.").color(warn),
            );
            if ui.button("Reload from Disk").clicked()
                && let Err(e) = doc.reload()
            {
                state.error(format!("could not reload: {e}"));
            }
            if ui
                .button("Keep Mine")
                .on_hover_text("Saving will overwrite the disk version")
                .clicked()
            {
                doc.keep_mine();
            }
        }),
        Some(Conflict::Deleted) => bar(ui, err, &mut |ui| {
            ui.label(
                RichText::new("⛔ The file was deleted on disk. Save to recreate it.").color(err),
            );
        }),
        None => {}
    }
    if let Some(why) = doc.read_only {
        bar(ui, theme::TEXT_WEAK, &mut |ui| {
            ui.label(format!("🔒 {why}"));
        });
    }
}

fn status_bar(ui: &mut egui::Ui, ide: &Ide, idx: usize, edit_id: egui::Id, markers: &[Marker]) {
    let doc = &ide.docs[idx];
    ui.horizontal(|ui| {
        let small = |t: String| RichText::new(t).small().color(theme::TEXT_WEAK);
        if let Some(r) = TextEditState::load(ui.ctx(), edit_id).and_then(|s| s.cursor.char_range())
        {
            let (l, c) = text_ops::char_to_line_col(&doc.text, r.primary.index.0);
            let n = r.as_sorted_char_range();
            let selected = n.end.0 - n.start.0;
            let mut t = format!("Ln {l}, Col {c}");
            if selected > 0 {
                t.push_str(&format!(" ({selected} selected)"));
            }
            ui.label(small(t));
        }
        ui.label(small(doc.lang.name().to_owned()));
        ui.label(small("Spaces: 4".to_owned()));
        match &doc.shader {
            Some(e) if e.is_empty() && !doc.shader_pending() => {
                ui.label(
                    RichText::new("✔ Surface shader OK")
                        .small()
                        .color(theme::AXIS_Y),
                );
            }
            Some(e) if !e.is_empty() => {
                let text = format!(
                    "⛔ {} shader error(s): line {}: {}",
                    e.len(),
                    e[0].line,
                    e[0].message
                );
                ui.label(
                    RichText::new(text)
                        .small()
                        .color(ui.visuals().error_fg_color),
                );
            }
            Some(_) => {
                ui.label(small("checking shader…".to_owned()));
            }
            None => {}
        }
        let cargo = markers
            .iter()
            .filter(|m| m.severity == Severity::Error)
            .count();
        if cargo > 0 && doc.shader.is_none() {
            ui.label(
                RichText::new(format!("⛔ {cargo}"))
                    .small()
                    .color(ui.visuals().error_fg_color),
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_d_selects_word_then_next() {
        let text = "foo bar foo baz foo";
        assert_eq!(next_occurrence(text, &(1..1)), Some(0..3));
        assert_eq!(next_occurrence(text, &(0..3)), Some(8..11));
        assert_eq!(next_occurrence(text, &(16..19)), Some(0..3));
        assert_eq!(line_starts("a\nbc\n"), vec![0, 2, 5]);
    }
}
