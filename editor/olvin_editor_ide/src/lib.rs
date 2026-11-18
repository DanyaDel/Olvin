//! Editor element "IDE": a small VS Code inside the engine. Project explorer,
//! code editor with highlighting for Rust/WGSL/RON/TOML/Markdown, cargo
//! check/build/run with a Problems list and an Output view, surface shader
//! validation with naga, command palette and quick open.
//!
//! Logic lives in pure modules (`lang`, `text_ops`, `diagnostics`, `shader`,
//! `fuzzy`, `files`); `ide` is the shared state, `panels` draw it.

pub mod diagnostics;
mod dialogs;
pub mod document;
pub mod files;
pub mod fuzzy;
pub mod highlight;
pub mod ide;
pub mod job;
pub mod keys;
pub mod lang;
mod palette;
pub mod panels;
pub mod shader;
pub mod text_ops;

use olvin_editor::egui_dock::{DockState, NodeIndex};
use olvin_editor::{EditorPlugin, PanelKind, Registry, Tab, TabAlloc, WorkspaceTemplate};

pub const EXPLORER: &str = "ide.explorer";
pub const CODE: &str = "ide.code";
pub const PROBLEMS: &str = "ide.problems";
pub const OUTPUT: &str = "ide.output";

pub struct IdePlugin;

impl EditorPlugin for IdePlugin {
    fn name(&self) -> &'static str {
        "IDE"
    }

    fn build(&self, registry: &mut Registry) {
        registry
            .add_panel(PanelKind {
                id: EXPLORER,
                title: "🗁 Explorer",
                factory: || Box::<panels::explorer::Explorer>::default(),
            })
            .add_panel(PanelKind {
                id: CODE,
                title: "✏ Code",
                factory: || Box::<panels::code::CodeEditor>::default(),
            })
            .add_panel(PanelKind {
                id: PROBLEMS,
                title: "⚠ Problems",
                factory: || Box::<panels::problems::Problems>::default(),
            })
            .add_panel(PanelKind {
                id: OUTPUT,
                title: "📋 Output",
                factory: || Box::<panels::output::Output>::default(),
            })
            .add_workspace_template(WorkspaceTemplate {
                name: "Code",
                build: code_layout,
            });
    }
}

/// Explorer left, editor centre, Problems / Output below.
pub fn code_layout(a: &mut TabAlloc) -> DockState<Tab> {
    let mut dock = DockState::new(vec![a.tab(CODE)]);
    let tree = dock.main_surface_mut();
    let [center, _] = tree.split_left(NodeIndex::root(), 0.2, vec![a.tab(EXPLORER)]);
    tree.split_below(center, 0.72, vec![a.tab(PROBLEMS), a.tab(OUTPUT)]);
    dock
}

#[cfg(test)]
mod tests {
    use super::*;
    use olvin_editor::egui::{self, Event, Key, Modifiers};
    use olvin_editor::{EditorState, Panel};

    fn input(events: Vec<Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 800.0),
            )),
            events,
            ..Default::default()
        }
    }

    #[test]
    fn panels_draw_headless_with_a_project() {
        let dir = std::env::temp_dir().join(format!("olvin_ide_ui_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("assets/shaders")).unwrap();
        std::fs::create_dir_all(dir.join("target/debug")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"g\"\n").unwrap();
        std::fs::write(
            dir.join("src/main.rs"),
            "fn main() {\n    println!(\"hi\");\n}\n",
        )
        .unwrap();
        std::fs::write(dir.join("assets/shaders/s.wgsl"), shader::template()).unwrap();

        let mut state = EditorState::new(Some(dir.clone()));
        let ctx = egui::Context::default();
        let mut explorer = panels::explorer::Explorer;
        let mut code = panels::code::CodeEditor::default();
        let mut problems = panels::problems::Problems::default();
        let mut output = panels::output::Output::default();

        let mut frame = |events: Vec<Event>, state: &mut EditorState| {
            ctx.run_ui(input(events), |ui| {
                ui.columns(2, |cols| {
                    explorer.ui(&mut cols[0], state);
                    problems.ui(&mut cols[0], state);
                    output.ui(&mut cols[0], state);
                    code.ui(&mut cols[1], state);
                });
            })
            .textures_delta
            .clear();
        };
        for _ in 0..3 {
            frame(vec![], &mut state);
        }
        ide::with(|ide| {
            assert_eq!(ide.root.as_deref(), Some(dir.as_path()));
            assert!(ide.entries.iter().any(|e| e.rel == "src/main.rs"));
            assert!(!ide.entries.iter().any(|e| e.rel.starts_with("target")));
            ide.open_at(&dir.join("src/main.rs"), 2, 5, &mut state);
            ide.diagnostics.push(diagnostics::Diagnostic {
                severity: diagnostics::Severity::Error,
                path: dir.join("src/main.rs"),
                line: 2,
                column: 5,
                end_line: 2,
                end_column: 13,
                message: "test error".into(),
                label: None,
                code: None,
            });
        });
        for _ in 0..3 {
            frame(vec![], &mut state);
        }
        // Typing goes into the focused editor at the jump target.
        frame(
            vec![Event::Text("x".into()), Event::Text("(".into())],
            &mut state,
        );
        ide::with(|ide| {
            let doc = ide.active_doc().unwrap();
            // No pairing right before a word.
            assert!(doc.text.contains("    x(println!"), "{}", doc.text);
            assert!(doc.dirty);
            doc.goto = Some(doc.text.chars().count());
        });
        frame(vec![], &mut state);
        frame(vec![Event::Text("(".into())], &mut state);
        ide::with(|ide| {
            let doc = ide.active_doc().unwrap();
            assert!(doc.text.ends_with("}\n()"), "{:?}", doc.text);
        });
        // Ctrl+S while the code has focus saves the file, not the scene.
        let key = |key, modifiers| Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        frame(vec![key(Key::S, Modifiers::COMMAND)], &mut state);
        frame(vec![], &mut state);
        ide::with(|ide| assert!(!ide.active_doc().unwrap().dirty, "Ctrl+S saved"));
        assert!(
            std::fs::read_to_string(dir.join("src/main.rs"))
                .unwrap()
                .contains('x')
        );

        // Palette + quick open, then the shader file.
        frame(vec![key(Key::E, Modifiers::COMMAND)], &mut state);
        ide::with(|ide| assert!(ide.palette.is_some()));
        frame(vec![Event::Text("s.wgsl".into())], &mut state);
        frame(vec![key(Key::Enter, Modifiers::NONE)], &mut state);
        for _ in 0..2 {
            frame(vec![], &mut state);
        }
        ide::with(|ide| {
            assert!(ide.palette.is_none());
            let doc = ide.active_doc().unwrap();
            assert!(doc.path.ends_with("s.wgsl"));
            assert_eq!(doc.shader, Some(vec![]));
        });
        // Find bar opens with Ctrl+F.
        frame(vec![key(Key::F, Modifiers::COMMAND)], &mut state);
        for _ in 0..2 {
            frame(vec![], &mut state);
        }
        ide::with(|ide| {
            ide.docs.clear();
            ide.active = None;
        });
        frame(vec![], &mut state);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn registers_panels_and_template() {
        let mut registry = Registry::default();
        IdePlugin.build(&mut registry);
        for k in [EXPLORER, CODE, PROBLEMS, OUTPUT] {
            assert!(registry.panel(k).is_some(), "{k} missing");
        }
        assert!(registry.templates().iter().any(|t| t.name == "Code"));
    }
}
