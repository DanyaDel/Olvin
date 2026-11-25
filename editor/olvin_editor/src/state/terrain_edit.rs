//! Terrain brush strokes as undo steps. A stroke keeps the terrain's data
//! from before it; at the end it becomes one step and the file is written,
//! so Play and the game read what the scene view shows. Undo swaps the data
//! back and writes the file again.

use olvin::terrain::TerrainData;

use super::{EditorState, Step, UNDO_LIMIT};

impl EditorState {
    /// A stroke starts changing the terrain data `rel`: remembers it as it
    /// is now. Further calls during the same stroke do nothing.
    pub fn terrain_stroke_begin(&mut self, rel: &str) {
        if self.terrain_stroke.is_some() {
            return;
        }
        let root = self.assets_dir();
        let before = self.terrains.entry(root.as_deref(), rel).data.clone();
        self.terrain_stroke = Some((rel.to_owned(), before));
    }

    /// The stroke is over: one undo step, and the file is written.
    pub fn terrain_stroke_end(&mut self) {
        let Some((rel, before)) = self.terrain_stroke.take() else {
            return;
        };
        self.commit();
        self.undo.push(Step::Terrain(rel.clone(), Box::new(before)));
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.save_terrain(&rel);
    }

    /// Whether a stroke is in progress.
    pub fn terrain_stroking(&self) -> bool {
        self.terrain_stroke.is_some()
    }

    /// Puts `data` in place of the terrain `rel` (undo/redo); returns what was there.
    pub(super) fn swap_terrain(&mut self, rel: &str, data: TerrainData) -> TerrainData {
        let root = self.assets_dir();
        let now = self.terrains.entry(root.as_deref(), rel).data.clone();
        self.terrains.replace(rel, data);
        self.save_terrain(rel);
        now
    }

    fn save_terrain(&mut self, rel: &str) {
        if rel.is_empty() {
            return;
        }
        if let Some(root) = self.assets_dir()
            && let Err(e) = self.terrains.save(&root, rel)
        {
            self.error(format!("terrain: {e}"));
        }
    }
}
