//! Terrains in the editor: their data, shared by the scene view (which draws
//! it) and the brushes (which change it). The data lives in `*.terrain`
//! files next to the scene; edits are kept here and written at the end of
//! each stroke, so Play and the game read what the view shows.

use std::collections::HashMap;
use std::path::Path;

use olvin::terrain::{DEFAULT_RESOLUTION, TerrainData};

/// A rectangle of vertices, inclusive.
pub type VertexRect = ((u32, u32), (u32, u32));

fn union(a: Option<VertexRect>, b: VertexRect) -> VertexRect {
    match a {
        None => b,
        Some(((ax, az), (bx, bz))) => (
            (ax.min(b.0.0), az.min(b.0.1)),
            (bx.max(b.1.0), bz.max(b.1.1)),
        ),
    }
}

pub struct TerrainEntry {
    pub data: TerrainData,
    /// Changed heights the scene view has not rebuilt yet.
    heights_dirty: Option<VertexRect>,
    /// Changed weights the scene view has not uploaded yet.
    weights_dirty: bool,
    /// Bumped on every change (a replaced data set included).
    pub generation: u64,
}

/// Every terrain's data by its file (relative to `assets/`).
#[derive(Default)]
pub struct TerrainStore {
    entries: HashMap<String, TerrainEntry>,
}

impl TerrainStore {
    /// The data of `rel`, loaded on first use (flat when there is no file).
    pub fn entry(&mut self, root: Option<&Path>, rel: &str) -> &mut TerrainEntry {
        self.entries
            .entry(rel.to_owned())
            .or_insert_with(|| TerrainEntry {
                data: olvin::terrain::load_data(root, rel),
                heights_dirty: None,
                weights_dirty: false,
                generation: 0,
            })
    }

    /// The data of `rel` if it is loaded already.
    pub fn peek(&self, rel: &str) -> Option<&TerrainData> {
        self.entries.get(rel).map(|e| &e.data)
    }

    /// Marks heights in `rect` of `rel` as changed.
    pub fn heights_changed(&mut self, rel: &str, rect: VertexRect) {
        if let Some(e) = self.entries.get_mut(rel) {
            e.heights_dirty = Some(union(e.heights_dirty, rect));
            e.generation += 1;
        }
    }

    pub fn weights_changed(&mut self, rel: &str) {
        if let Some(e) = self.entries.get_mut(rel) {
            e.weights_dirty = true;
            e.generation += 1;
        }
    }

    /// Replaces the whole data set of `rel` (undo, a new resolution).
    pub fn replace(&mut self, rel: &str, data: TerrainData) {
        let last = data.resolution() - 1;
        let e = self
            .entries
            .entry(rel.to_owned())
            .or_insert_with(|| TerrainEntry {
                data: TerrainData::flat(2),
                heights_dirty: None,
                weights_dirty: false,
                generation: 0,
            });
        e.data = data;
        e.heights_dirty = Some(((0, 0), (last, last)));
        e.weights_dirty = true;
        e.generation += 1;
    }

    /// What the scene view must refresh for `rel`: changed heights, changed weights.
    pub(crate) fn take_dirty(&mut self, rel: &str) -> (Option<VertexRect>, bool) {
        match self.entries.get_mut(rel) {
            Some(e) => (e.heights_dirty.take(), std::mem::take(&mut e.weights_dirty)),
            None => (None, false),
        }
    }

    /// Writes `rel` to its file.
    pub fn save(&self, root: &Path, rel: &str) -> Result<(), String> {
        match self.entries.get(rel) {
            Some(e) => e.data.save(root.join(rel)),
            None => Ok(()),
        }
    }

    /// Drops what is cached for `rel` and under it (moved or deleted files):
    /// the next use reads the file again.
    pub fn forget(&mut self, prefix: &str) {
        self.entries
            .retain(|k, _| k != prefix && !k.starts_with(&format!("{prefix}/")));
    }
}

/// A new flat terrain file under `terrains/` with a free name; returns its path.
pub fn create_file(root: &Path, name: &str) -> Result<String, String> {
    let stem: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let rel = (1..10_000)
        .map(|n| match n {
            1 => format!("terrains/{stem}.terrain"),
            n => format!("terrains/{stem}-{n}.terrain"),
        })
        .find(|r| !root.join(r).exists())
        .ok_or("no free terrain file name under terrains/")?;
    TerrainData::flat(DEFAULT_RESOLUTION).save(root.join(&rel))?;
    Ok(rel)
}
