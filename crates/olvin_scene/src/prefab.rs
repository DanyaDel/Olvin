//! Prefabs: a small scene subtree saved as `*.prefab.ron` (a [`SceneFile`] with
//! its own ids, one root and its children) and placed in scenes as instances.
//!
//! On disk an instance is one entity, the *root*, with [`EntityDesc::prefab`]:
//! the asset, what the instance changes ([`PrefabInstance::overrides`],
//! [`PrefabInstance::removed`]) and nothing else of the subtree. Children added
//! to an instance are ordinary entities whose parent is a node of it. The root
//! keeps its own name, activity, parent and transform: placing and renaming an
//! instance is not an override.
//!
//! [`SceneFile::expand_prefabs`] turns instances into plain entities (what games
//! spawn and the editor edits): every node of the asset becomes an entity with
//! the id [`member_id`] and a [`PrefabNode`] mark. [`SceneFile::collapse_prefabs`]
//! is the way back: the marked entities are compared with the asset and what
//! differs becomes the instance's overrides. So edits to an expanded instance
//! need no bookkeeping, and editing the asset updates every instance.
//!
//! Nodes are addressed by a *path* of asset-local ids: `""` is the root, `"3"` a
//! node of the asset, `"3/2"` node 2 of a prefab instance that is node 3 of the
//! asset (nested prefabs).

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::file::{EntityDesc, SceneFile};

/// How deep prefabs may nest (a cycle is cut earlier, by asset path).
pub const MAX_DEPTH: usize = 16;

/// Marks the ids of prefab nodes, so scene ids allocated as "highest + 1"
/// never collide with them.
pub const MEMBER_BIT: u64 = 1 << 63;

/// The scene id of node `local` of the instance whose root has id `parent`.
/// A hash, so it is the same in every load and for nested instances (whose
/// root is itself a node), and needs no stored table.
pub fn member_id(parent: u64, local: u64) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in parent.to_le_bytes().into_iter().chain(local.to_le_bytes()) {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    h | MEMBER_BIT
}

/// The scene id of the node at `path` (see the module docs) of the instance
/// whose root has id `root`.
pub fn node_id(root: u64, path: &str) -> u64 {
    path.split('/')
        .filter_map(|l| l.parse().ok())
        .fold(root, member_id)
}

/// Whether `id` is the id of a prefab node (see [`member_id`]).
pub fn is_member_id(id: u64) -> bool {
    id & MEMBER_BIT != 0
}

/// A prefab instance, stored on its root entity.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PrefabInstance {
    /// `*.prefab.ron`, relative to the project's `assets/` (like materials).
    pub asset: String,
    /// Changed fields: node path → field → RON text of the new value. A field
    /// is a top-level field of [`EntityDesc`] by name (`"position"`, `"mesh"`,
    /// `"scripts"`...), or `"components.<Type>"` for one game component
    /// (empty text: the component is removed on this instance).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub overrides: BTreeMap<String, BTreeMap<String, String>>,
    /// Paths of nodes deleted from this instance (with their subtrees).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<String>,
}

impl PrefabInstance {
    pub fn new(asset: impl Into<String>) -> Self {
        Self {
            asset: asset.into(),
            ..Default::default()
        }
    }

    /// Nothing differs from the asset.
    pub fn is_clean(&self) -> bool {
        self.overrides.is_empty() && self.removed.is_empty()
    }
}

/// In an expanded scene: this entity is node `path` of the instance at `root`.
/// Not saved (collapsing drops these entities).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrefabNode {
    /// Id of the instance's root entity.
    pub root: u64,
    pub path: String,
}

/// Reads prefab assets: path relative to `assets/` → the parsed file.
pub type Loader<'a> = dyn FnMut(&str) -> Result<SceneFile, String> + 'a;

/// One entity of an expanded instance.
#[derive(Clone, Debug)]
pub struct Member {
    pub path: String,
    pub desc: EntityDesc,
}

/// The fields the root keeps for itself (an instance is placed and named freely).
const ROOT_OWN: [&str; 5] = ["name", "active", "position", "rotation", "scale"];

/// Every overridable field of `e` as (key, RON text).
fn field_texts(e: &EntityDesc) -> Vec<(String, String)> {
    // Destructured without `..` so a new field cannot be forgotten here.
    let EntityDesc {
        id: _,
        parent: _,
        name,
        active,
        tag,
        layer,
        position,
        rotation,
        scale,
        mesh,
        camera,
        poly,
        brush,
        material,
        model,
        particles,
        light,
        collider,
        rigidbody,
        character_controller,
        audio_source,
        audio_listener,
        scripts,
        components,
        prefab: _,
        prefab_node: _,
    } = e;
    let mut out = Vec::new();
    macro_rules! push {
        ($($f:ident),*) => {$(
            out.push((stringify!($f).to_owned(), ron::to_string($f).unwrap_or_default()));
        )*};
    }
    push!(
        name,
        active,
        tag,
        layer,
        position,
        rotation,
        scale,
        mesh,
        camera,
        poly,
        brush,
        material,
        model,
        particles,
        light,
        collider,
        rigidbody,
        character_controller,
        audio_source,
        audio_listener,
        scripts
    );
    for (k, v) in components {
        out.push((format!("components.{k}"), v.clone()));
    }
    out
}

/// Sets one field from its RON text; see [`field_texts`] for the keys.
fn set_field(e: &mut EntityDesc, key: &str, text: &str) -> Result<(), String> {
    macro_rules! set {
        ($($f:ident),*) => {
            match key {
                $(stringify!($f) => {
                    e.$f = ron::from_str(text).map_err(|err| format!("`{key}`: {err}"))?
                })*
                _ => return Err(format!("unknown field `{key}`")),
            }
        };
    }
    if let Some(component) = key.strip_prefix("components.") {
        if text.is_empty() {
            e.components.remove(component);
        } else {
            e.components.insert(component.to_owned(), text.to_owned());
        }
        return Ok(());
    }
    set!(
        name,
        active,
        tag,
        layer,
        position,
        rotation,
        scale,
        mesh,
        camera,
        poly,
        brush,
        material,
        model,
        particles,
        light,
        collider,
        rigidbody,
        character_controller,
        audio_source,
        audio_listener,
        scripts
    );
    Ok(())
}

/// What differs between `now` and `base` as overrides (`root`: the instance
/// root, which keeps its identity fields).
fn diff_fields(base: &EntityDesc, now: &EntityDesc, root: bool) -> BTreeMap<String, String> {
    let before: HashMap<String, String> = field_texts(base).into_iter().collect();
    let mut out = BTreeMap::new();
    for (key, text) in field_texts(now) {
        if root && ROOT_OWN.contains(&key.as_str()) {
            continue;
        }
        if before.get(&key) != Some(&text) {
            out.insert(key, text);
        }
    }
    for key in before.keys() {
        if key.starts_with("components.")
            && !now.components.contains_key(&key["components.".len()..])
        {
            out.insert(key.clone(), String::new());
        }
    }
    out
}

/// Nodes of `root`'s instance, root first, parents before children.
/// `None` (with a warning) when the asset is missing, broken or recursive.
fn expand_instance(
    root: &EntityDesc,
    load: &mut Loader,
    stack: &mut Vec<String>,
    warnings: &mut Vec<String>,
) -> Option<Vec<Member>> {
    let inst = root.prefab.as_ref()?;
    let who = format!("`{}`: prefab `{}`", root.name, inst.asset);
    if stack.len() >= MAX_DEPTH || stack.contains(&inst.asset) {
        warnings.push(format!("{who} contains itself, skipped"));
        return None;
    }
    let asset = match load(&inst.asset) {
        Ok(asset) => asset,
        Err(err) => {
            warnings.push(format!("{who}: {err}"));
            return None;
        }
    };
    let Some(root_local) = asset
        .entities
        .iter()
        .find(|e| e.parent.is_none_or(|p| asset.get(p).is_none()))
        .map(|e| e.id)
    else {
        warnings.push(format!("{who} has no objects"));
        return None;
    };
    stack.push(inst.asset.clone());

    let mut members: Vec<Member> = Vec::new();
    let mut ids: HashMap<u64, u64> = HashMap::new();
    for i in asset.parents_first() {
        let node = &asset.entities[i];
        let is_root = node.id == root_local;
        // Parentless extras hang under the root.
        let parent = if is_root {
            root.parent
        } else {
            let local = node
                .parent
                .filter(|p| asset.get(*p).is_some())
                .unwrap_or(root_local);
            // A parent cycle never resolves: those nodes are dropped.
            let Some(p) = ids.get(&local) else { continue };
            Some(*p)
        };
        let (id, path) = if is_root {
            (root.id, String::new())
        } else {
            (member_id(root.id, node.id), node.id.to_string())
        };
        ids.insert(node.id, id);
        let mut desc = node.clone();
        desc.id = id;
        desc.parent = parent;
        if is_root {
            desc.name = root.name.clone();
            desc.active = root.active;
            desc.position = root.position;
            desc.rotation = root.rotation;
            desc.scale = root.scale;
            // Expanded, the overrides live in the nodes: collapsing finds them again.
            desc.prefab = Some(PrefabInstance::new(inst.asset.clone()));
        } else if node.prefab.is_some() {
            // A prefab inside the prefab: its nodes follow, as `path/<its path>`.
            if let Some(inner) = expand_instance(&desc, load, stack, warnings) {
                members.extend(inner.into_iter().map(|m| Member {
                    path: if m.path.is_empty() {
                        path.clone()
                    } else {
                        format!("{path}/{}", m.path)
                    },
                    desc: m.desc,
                }));
                continue;
            }
        }
        members.push(Member { path, desc });
    }
    stack.pop();

    // What this instance removed (nodes and their subtrees) and changed.
    let mut gone: HashSet<u64> = HashSet::new();
    members.retain(|m| {
        let removed = inst
            .removed
            .iter()
            .any(|r| m.path == *r || m.path.starts_with(&format!("{r}/")));
        if removed || m.desc.parent.is_some_and(|p| gone.contains(&p)) {
            gone.insert(m.desc.id);
            return false;
        }
        true
    });
    for m in &mut members {
        let Some(fields) = inst.overrides.get(&m.path) else {
            continue;
        };
        for (key, text) in fields {
            if m.path.is_empty() && ROOT_OWN.contains(&key.as_str()) {
                continue;
            }
            if let Err(err) = set_field(&mut m.desc, key, text) {
                warnings.push(format!("{who}, node `{}`: {err}", m.path));
            }
        }
    }
    Some(members)
}

impl SceneFile {
    /// Replaces every prefab instance by its entities (see the module docs).
    /// Entities that are expanded already are left alone, so this is safe to
    /// repeat. An instance whose asset cannot be read stays a single entity;
    /// the returned warnings say why.
    pub fn expand_prefabs(&mut self, load: &mut Loader) -> Vec<String> {
        let mut warnings = Vec::new();
        if !self
            .entities
            .iter()
            .any(|e| e.prefab.is_some() && e.prefab_node.is_none())
        {
            return warnings;
        }
        let mut out = Vec::with_capacity(self.entities.len());
        for e in std::mem::take(&mut self.entities) {
            if e.prefab.is_none() || e.prefab_node.is_some() {
                out.push(e);
                continue;
            }
            match expand_instance(&e, load, &mut Vec::new(), &mut warnings) {
                Some(members) => out.extend(members.into_iter().map(|m| {
                    let mut desc = m.desc;
                    desc.prefab_node = Some(PrefabNode {
                        root: e.id,
                        path: m.path,
                    });
                    desc
                })),
                None => out.push(e),
            }
        }
        self.entities = out;
        warnings
    }

    /// The scene as it is saved: instance nodes dropped, each instance root
    /// carrying what differs from its asset. `load` must return the asset the
    /// instances were expanded from.
    pub fn collapse_prefabs(&self, load: &mut Loader) -> SceneFile {
        let mut out = self.clone();
        out.entities.clear();
        for e in &self.entities {
            match &e.prefab_node {
                None => out.entities.push(e.clone()),
                Some(node) if node.path.is_empty() && node.root == e.id => {
                    let mut lean = EntityDesc::new(e.id, e.name.clone());
                    lean.active = e.active;
                    lean.parent = e.parent;
                    lean.position = e.position;
                    lean.rotation = e.rotation;
                    lean.scale = e.scale;
                    // The asset cannot be read: keep what the instance said before.
                    lean.prefab = self
                        .prefab_diff(e.id, load)
                        .ok()
                        .or_else(|| e.prefab.clone());
                    out.entities.push(lean);
                }
                Some(_) => {}
            }
        }
        out
    }

    /// The instance as it is in the asset, unchanged (root and nodes, in order).
    pub fn prefab_pristine(&self, root: u64, load: &mut Loader) -> Result<Vec<Member>, String> {
        let e = self.get(root).ok_or("no such object")?;
        let asset = e
            .prefab
            .as_ref()
            .ok_or("not a prefab instance")?
            .asset
            .clone();
        let mut clean = e.clone();
        clean.prefab = Some(PrefabInstance::new(asset));
        let mut warnings = Vec::new();
        expand_instance(&clean, load, &mut Vec::new(), &mut warnings)
            .ok_or_else(|| warnings.join("; "))
    }

    /// What the expanded instance at `root` changes: the overrides and removed
    /// nodes that [`SceneFile::collapse_prefabs`] would save. Nodes added under
    /// it are not part of this (see [`SceneFile::prefab_added`]).
    pub fn prefab_diff(&self, root: u64, load: &mut Loader) -> Result<PrefabInstance, String> {
        let pristine = self.prefab_pristine(root, load)?;
        let asset = self
            .get(root)
            .and_then(|e| e.prefab.as_ref())
            .map(|p| p.asset.clone())
            .unwrap_or_default();
        let present: HashMap<&str, &EntityDesc> = self
            .entities
            .iter()
            .filter_map(|e| {
                let n = e.prefab_node.as_ref().filter(|n| n.root == root)?;
                Some((n.path.as_str(), e))
            })
            .collect();
        let mut out = PrefabInstance::new(asset);
        let by_id: HashMap<u64, &str> = pristine
            .iter()
            .map(|m| (m.desc.id, m.path.as_str()))
            .collect();
        for m in &pristine {
            match present.get(m.path.as_str()) {
                Some(now) => {
                    let fields = diff_fields(&m.desc, now, m.path.is_empty());
                    if !fields.is_empty() {
                        out.overrides.insert(m.path.clone(), fields);
                    }
                }
                // Only the top of a removed subtree is listed.
                None => {
                    let parent_gone = m
                        .desc
                        .parent
                        .and_then(|p| by_id.get(&p))
                        .is_some_and(|p| !present.contains_key(p));
                    if !parent_gone {
                        out.removed.push(m.path.clone());
                    }
                }
            }
        }
        Ok(out)
    }

    /// The instance root a prefab node belongs to.
    pub fn prefab_root_of(&self, id: u64) -> Option<u64> {
        Some(self.get(id)?.prefab_node.as_ref()?.root)
    }

    /// Ids of entities added under the instance at `root`: not nodes of the
    /// asset, but descendants of one.
    pub fn prefab_added(&self, root: u64) -> Vec<u64> {
        let mut inside: HashSet<u64> = self
            .entities
            .iter()
            .filter(|e| e.prefab_node.as_ref().is_some_and(|n| n.root == root))
            .map(|e| e.id)
            .collect();
        let mut added = Vec::new();
        // Parents come first, but a hand-edited file may not keep to it: repeat.
        loop {
            let before = added.len();
            for e in &self.entities {
                if e.prefab_node.is_none()
                    && !inside.contains(&e.id)
                    && e.parent.is_some_and(|p| inside.contains(&p))
                {
                    inside.insert(e.id);
                    added.push(e.id);
                }
            }
            if added.len() == before {
                return added;
            }
        }
    }

    /// Puts the instance back to what its asset says: overrides, removed
    /// nodes and added children are dropped.
    pub fn revert_prefab(&mut self, root: u64) {
        let added: HashSet<u64> = self.prefab_added(root).into_iter().collect();
        self.entities.retain(|e| {
            let member = e
                .prefab_node
                .as_ref()
                .is_some_and(|n| n.root == root && !(n.path.is_empty() && e.id == root));
            !member && !added.contains(&e.id)
        });
        if let Some(e) = self.get_mut(root) {
            e.prefab_node = None;
            if let Some(p) = &mut e.prefab {
                p.overrides.clear();
                p.removed.clear();
            }
        }
    }

    /// Puts one field of one node back to the asset's value.
    pub fn revert_prefab_field(
        &mut self,
        id: u64,
        key: &str,
        load: &mut Loader,
    ) -> Result<(), String> {
        let node = self
            .get(id)
            .and_then(|e| e.prefab_node.clone())
            .ok_or("not a prefab node")?;
        let pristine = self.prefab_pristine(node.root, load)?;
        let base = &pristine
            .iter()
            .find(|m| m.path == node.path)
            .ok_or("the node is not in the prefab any more")?
            .desc;
        let text = field_texts(base)
            .into_iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
            .unwrap_or_default();
        let e = self.get_mut(id).ok_or("no such object")?;
        set_field(e, key, &text)
    }

    /// Detaches every instance of `asset` from it (the asset is going away):
    /// expanded ones keep their nodes as ordinary entities, ones that never
    /// expanded become empty objects. Prefabs inside other prefabs stay.
    pub fn unpack_asset(&mut self, asset: &str) {
        let roots: Vec<u64> = self
            .entities
            .iter()
            .filter(|e| e.prefab.as_ref().is_some_and(|p| p.asset == asset))
            .filter(|e| e.prefab_node.as_ref().is_none_or(|n| n.path.is_empty()))
            .map(|e| e.id)
            .collect();
        for id in roots {
            if self.get(id).is_some_and(|e| e.prefab_node.is_some()) {
                self.unpack_prefab(id);
            } else if let Some(e) = self.get_mut(id) {
                e.prefab = None;
            }
        }
    }

    /// Detaches the instance at `root` from its asset: its nodes become
    /// ordinary entities.
    pub fn unpack_prefab(&mut self, root: u64) {
        for e in &mut self.entities {
            if e.prefab_node.as_ref().is_some_and(|n| n.root == root) {
                e.prefab_node = None;
                e.prefab = None;
            }
        }
    }

    /// The asset with the instance's changes applied to it (Apply All): the
    /// overrides become the asset's values, removed nodes leave it and added
    /// children become nodes. `None` when nothing differs. Callers then drop
    /// [`SceneFile::prefab_added`] from the scene and expand again.
    pub fn apply_prefab(&self, root: u64, load: &mut Loader) -> Result<SceneFile, String> {
        let diff = self.prefab_diff(root, load)?;
        let mut asset = load(&diff.asset)?;
        let root_local = asset
            .entities
            .iter()
            .find(|e| e.parent.is_none_or(|p| asset.get(p).is_none()))
            .map(|e| e.id)
            .ok_or("the prefab has no objects")?;
        let mut next = asset.entities.iter().map(|e| e.id).max().unwrap_or(0) + 1;

        for (path, fields) in &diff.overrides {
            let (head, rest) = split_path(path);
            let local = head.unwrap_or(root_local);
            let node = asset.get_mut(local).ok_or("node missing from the prefab")?;
            match rest {
                None => {
                    for (key, text) in fields {
                        set_field(node, key, text)?;
                    }
                }
                // A node of a prefab inside the prefab: the inner instance takes it.
                Some(rest) => {
                    let inner = node.prefab.as_mut().ok_or("node missing from the prefab")?;
                    inner
                        .overrides
                        .entry(rest.to_owned())
                        .or_default()
                        .extend(fields.clone());
                }
            }
        }
        let mut doomed: Vec<u64> = Vec::new();
        for path in &diff.removed {
            match split_path(path) {
                (Some(local), None) => doomed.push(local),
                (Some(local), Some(rest)) => {
                    if let Some(inner) = asset.get_mut(local).and_then(|n| n.prefab.as_mut()) {
                        inner.removed.push(rest.to_owned());
                    }
                }
                (None, _) => {}
            }
        }
        // Subtrees of removed nodes go too.
        let mut i = 0;
        while i < doomed.len() {
            let id = doomed[i];
            doomed.extend(
                asset
                    .entities
                    .iter()
                    .filter(|e| e.parent == Some(id))
                    .map(|e| e.id),
            );
            i += 1;
        }
        asset.entities.retain(|e| !doomed.contains(&e.id));

        // Added children become nodes, under the node their parent is.
        let nodes: HashMap<u64, u64> = self
            .entities
            .iter()
            .filter_map(|e| {
                let n = e.prefab_node.as_ref().filter(|n| n.root == root)?;
                match split_path(&n.path) {
                    (None, _) => Some((e.id, root_local)),
                    (Some(local), None) => Some((e.id, local)),
                    // A node of an inner prefab cannot take children.
                    (Some(_), Some(_)) => None,
                }
            })
            .collect();
        let mut new_ids: HashMap<u64, u64> = HashMap::new();
        for id in self.prefab_added(root) {
            let Some(e) = self.get(id) else { continue };
            // Under a node of an inner prefab there is no place for it.
            let Some(parent) = e
                .parent
                .and_then(|p| new_ids.get(&p).or_else(|| nodes.get(&p)).copied())
            else {
                continue;
            };
            let mut node = e.clone();
            node.id = next;
            node.parent = Some(parent);
            new_ids.insert(id, next);
            next += 1;
            asset.entities.push(node);
        }
        asset.version = crate::file::FORMAT_VERSION;
        Ok(asset)
    }
}

/// `"3/2"` → (3, "2"); `""` → (none, none).
fn split_path(path: &str) -> (Option<u64>, Option<&str>) {
    if path.is_empty() {
        return (None, None);
    }
    let (head, rest) = match path.split_once('/') {
        Some((h, r)) => (h, Some(r)),
        None => (path, None),
    };
    (head.parse().ok(), rest)
}

#[cfg(test)]
mod tests;
