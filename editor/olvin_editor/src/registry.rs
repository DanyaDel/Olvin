use egui_dock::DockState;

use crate::{EditorState, Tab};

/// One dockable window. A new instance is created per tab, so two viewports
/// keep two independent cameras.
pub trait Panel {
    fn ui(&mut self, ui: &mut egui::Ui, state: &mut EditorState);
}

/// An installable editor element: registers panels and workspace templates.
pub trait EditorPlugin {
    fn name(&self) -> &'static str;
    fn build(&self, registry: &mut Registry);
}

#[derive(Clone, Copy)]
pub struct PanelKind {
    /// Stable id stored in saved layouts, e.g. `"animator.timeline"`.
    pub id: &'static str,
    pub title: &'static str,
    pub factory: fn() -> Box<dyn Panel>,
}

/// Hands out unique tab ids while a workspace layout is being built.
pub struct TabAlloc(pub(crate) u32);

impl TabAlloc {
    pub fn tab(&mut self, kind: &str) -> Tab {
        self.0 += 1;
        Tab { kind: kind.to_owned(), id: self.0 }
    }
}

#[derive(Clone, Copy)]
pub struct WorkspaceTemplate {
    pub name: &'static str,
    pub build: fn(&mut TabAlloc) -> DockState<Tab>,
}

#[derive(Default)]
pub struct Registry {
    pub(crate) panels: Vec<PanelKind>,
    pub(crate) templates: Vec<WorkspaceTemplate>,
    pub(crate) plugins: Vec<&'static str>,
}

impl Registry {
    pub fn add_panel(&mut self, kind: PanelKind) -> &mut Self {
        self.panels.retain(|k| k.id != kind.id);
        self.panels.push(kind);
        self
    }

    pub fn add_workspace_template(&mut self, template: WorkspaceTemplate) -> &mut Self {
        self.templates.push(template);
        self
    }

    pub fn panel(&self, id: &str) -> Option<&PanelKind> {
        self.panels.iter().find(|k| k.id == id)
    }

    pub fn panels(&self) -> &[PanelKind] {
        &self.panels
    }

    pub fn templates(&self) -> &[WorkspaceTemplate] {
        &self.templates
    }

    pub fn plugins(&self) -> &[&'static str] {
        &self.plugins
    }
}
