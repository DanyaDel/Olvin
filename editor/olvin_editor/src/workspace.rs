use std::path::PathBuf;

use egui_dock::DockState;
use serde::{Deserialize, Serialize};

use crate::{Registry, TabAlloc};

/// A dock tab: which panel kind it shows plus a unique instance id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tab {
    pub kind: String,
    pub id: u32,
}

/// A browser-like top tab with its own window arrangement.
#[derive(Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub name: String,
    pub dock: DockState<Tab>,
}

#[derive(Serialize, Deserialize)]
pub struct Layout {
    pub workspaces: Vec<Workspace>,
    pub active: usize,
    pub next_tab: u32,
}

impl Layout {
    /// One workspace per registered template, in registration order.
    pub fn default_for(registry: &Registry) -> Self {
        let mut alloc = TabAlloc(0);
        let workspaces = registry
            .templates()
            .iter()
            .map(|t| Workspace { name: t.name.to_owned(), dock: (t.build)(&mut alloc) })
            .collect();
        Self { workspaces, active: 0, next_tab: alloc.0 }
    }

    pub fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("olvin-editor/layout.ron"))
    }

    /// The saved layout, or the default one if there is none or it's unreadable.
    pub fn load(registry: &Registry) -> Self {
        let saved = Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| ron::from_str::<Self>(&text).ok())
            .filter(|l| !l.workspaces.is_empty());
        match saved {
            Some(mut layout) => {
                layout.active = layout.active.min(layout.workspaces.len() - 1);
                layout
            }
            None => Self::default_for(registry),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or("нет папки конфигов")?;
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        let text = ron::ser::to_string_pretty(self, Default::default()).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| e.to_string())
    }

    pub fn alloc(&mut self) -> TabAlloc {
        TabAlloc(self.next_tab)
    }

    pub fn finish_alloc(&mut self, alloc: TabAlloc) {
        self.next_tab = self.next_tab.max(alloc.0);
    }

    pub fn add_workspace(&mut self, name: String, build: impl FnOnce(&mut TabAlloc) -> DockState<Tab>) {
        let mut alloc = self.alloc();
        let dock = build(&mut alloc);
        self.finish_alloc(alloc);
        self.workspaces.push(Workspace { name, dock });
        self.active = self.workspaces.len() - 1;
    }

    /// Copies a workspace with fresh tab ids, so panels don't share state.
    pub fn duplicate(&mut self, index: usize) {
        let mut alloc = self.alloc();
        let source = &self.workspaces[index];
        let dock = source.dock.filter_map_tabs(|tab| Some(alloc.tab(&tab.kind)));
        let name = format!("{} (копия)", source.name);
        self.finish_alloc(alloc);
        self.workspaces.insert(index + 1, Workspace { name, dock });
        self.active = index + 1;
    }

    pub fn close(&mut self, index: usize) {
        if self.workspaces.len() > 1 {
            self.workspaces.remove(index);
            if self.active >= index && self.active > 0 {
                self.active -= 1;
            }
        }
    }

    pub fn move_workspace(&mut self, from: usize, to: usize) {
        if from != to && to < self.workspaces.len() {
            let ws = self.workspaces.remove(from);
            self.workspaces.insert(to, ws);
            if self.active == from {
                self.active = to;
            }
        }
    }

    pub fn all_tab_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.workspaces.iter().flat_map(|w| w.dock.iter_all_tabs().map(|(_, t)| t.id))
    }
}
