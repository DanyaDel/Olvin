use std::collections::HashMap;

use glam::Vec3;

use super::*;
use crate::file::{MeshDesc, Shape};

type Assets = HashMap<String, SceneFile>;

fn loader(assets: &Assets) -> impl FnMut(&str) -> Result<SceneFile, String> + '_ {
    |path| {
        assets
            .get(path)
            .cloned()
            .ok_or_else(|| format!("{path}: not found"))
    }
}

fn mesh(id: u64, name: &str, parent: Option<u64>, pos: Vec3) -> EntityDesc {
    let mut e = EntityDesc::new(id, name);
    e.parent = parent;
    e.position = pos;
    e.mesh = Some(MeshDesc {
        shape: Shape::Cube,
        color: [0.5; 4],
    });
    e
}

/// A crate with a lid and a label (ids are the asset's own).
fn crate_prefab() -> SceneFile {
    SceneFile {
        entities: vec![
            mesh(1, "Crate", None, Vec3::ZERO),
            mesh(2, "Lid", Some(1), Vec3::Y),
            mesh(3, "Label", Some(2), Vec3::Z),
        ],
        ..Default::default()
    }
}

fn instance(id: u64, asset: &str, at: Vec3) -> EntityDesc {
    let mut e = EntityDesc::new(id, "Box");
    e.position = at;
    e.prefab = Some(PrefabInstance::new(asset));
    e
}

fn assets(list: &[(&str, SceneFile)]) -> Assets {
    list.iter()
        .map(|(p, s)| (p.to_string(), s.clone()))
        .collect()
}

fn scene(entities: Vec<EntityDesc>) -> SceneFile {
    SceneFile {
        entities,
        ..Default::default()
    }
}

#[test]
fn an_instance_expands_into_the_assets_nodes_with_stable_ids() {
    let assets = assets(&[("crate.prefab.ron", crate_prefab())]);
    let mut s = scene(vec![instance(
        10,
        "crate.prefab.ron",
        Vec3::new(5.0, 0.0, 0.0),
    )]);
    let warnings = s.expand_prefabs(&mut loader(&assets));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(s.entities.len(), 3);
    let root = s.get(10).unwrap();
    assert_eq!((root.name.as_str(), root.position.x), ("Box", 5.0));
    assert!(root.mesh.is_some(), "the rest comes from the asset");
    let lid = s.get(member_id(10, 2)).unwrap();
    assert_eq!(lid.parent, Some(10));
    assert_eq!(lid.name, "Lid");
    assert_eq!(s.get(member_id(10, 3)).unwrap().parent, Some(lid.id));
    assert!(is_member_id(lid.id) && !is_member_id(10));
    // Another instance gets other ids; expanding twice changes nothing.
    assert_ne!(member_id(10, 2), member_id(11, 2));
    let before = s.clone();
    s.expand_prefabs(&mut loader(&assets));
    assert_eq!(s, before);
    // The world position follows the root.
    let world = s.world_matrices()[&member_id(10, 2)].transform_point3(Vec3::ZERO);
    assert!(world.abs_diff_eq(Vec3::new(5.0, 1.0, 0.0), 1e-5));
}

#[test]
fn overrides_and_removed_nodes_apply_and_survive_a_save() {
    let assets = assets(&[("crate.prefab.ron", crate_prefab())]);
    let mut inst = instance(10, "crate.prefab.ron", Vec3::ZERO);
    let p = inst.prefab.as_mut().unwrap();
    p.overrides.insert(
        "2".into(),
        BTreeMap::from([("position".into(), "(0.0,2.5,0.0)".into())]),
    );
    p.overrides.insert(
        String::new(),
        BTreeMap::from([
            ("tag".into(), "\"Loot\"".into()),
            // The root's own fields are not overridable.
            ("name".into(), "\"Hacked\"".into()),
        ]),
    );
    p.removed.push("3".into());
    let mut s = scene(vec![instance(1, "crate.prefab.ron", Vec3::X), inst]);
    // Through the file text, as scenes are stored.
    let text = ron::to_string(&s).unwrap();
    assert!(!text.contains("prefab_node") && !text.contains("Lid"));
    s = SceneFile::parse(&text).unwrap();
    s.expand_prefabs(&mut loader(&assets));

    assert_eq!(s.get(member_id(10, 2)).unwrap().position.y, 2.5);
    assert!(s.get(member_id(10, 3)).is_none(), "removed");
    assert!(s.get(member_id(1, 3)).is_some(), "other instances keep it");
    assert_eq!(s.get(10).unwrap().tag, "Loot");
    assert_eq!(s.get(10).unwrap().name, "Box");

    // Collapsing finds the same overrides again.
    let saved = s.collapse_prefabs(&mut loader(&assets));
    assert_eq!(saved.entities.len(), 2);
    let back = saved.get(10).unwrap().prefab.as_ref().unwrap();
    assert_eq!(back.removed, ["3"]);
    assert_eq!(back.overrides["2"]["position"], "(0.0,2.5,0.0)");
    assert!(!back.overrides[""].contains_key("name"));
    assert!(saved.get(1).unwrap().prefab.as_ref().unwrap().is_clean());
    assert!(saved.get(1).unwrap().mesh.is_none(), "the root stays lean");
    let mut again = saved.clone();
    again.expand_prefabs(&mut loader(&assets));
    assert_eq!(again, s);
}

#[test]
fn edits_to_an_expanded_instance_become_overrides() {
    let assets = assets(&[("crate.prefab.ron", crate_prefab())]);
    let mut s = scene(vec![instance(10, "crate.prefab.ron", Vec3::ZERO)]);
    s.expand_prefabs(&mut loader(&assets));
    let lid = member_id(10, 2);
    assert!(s.prefab_diff(10, &mut loader(&assets)).unwrap().is_clean());

    s.get_mut(lid).unwrap().position.x = 4.0;
    s.get_mut(lid)
        .unwrap()
        .components
        .insert("Door".into(), "(open:true)".into());
    s.get_mut(10).unwrap().position = Vec3::splat(9.0); // placing is no override
    s.get_mut(10).unwrap().name = "Renamed".into();
    s.entities.retain(|e| e.id != member_id(10, 3)); // the label goes
    let diff = s.prefab_diff(10, &mut loader(&assets)).unwrap();
    assert_eq!(diff.overrides.len(), 1, "{diff:?}");
    assert!(diff.overrides["2"].contains_key("position"));
    assert_eq!(diff.overrides["2"]["components.Door"], "(open:true)");
    assert_eq!(diff.removed, ["3"]);

    // One field back to the asset's value.
    s.revert_prefab_field(lid, "position", &mut loader(&assets))
        .unwrap();
    assert_eq!(s.get(lid).unwrap().position, Vec3::Y);
    s.revert_prefab_field(lid, "components.Door", &mut loader(&assets))
        .unwrap();
    assert!(s.get(lid).unwrap().components.is_empty());
    // Everything back, children added by hand included.
    s.entities.push(mesh(50, "Sticker", Some(lid), Vec3::ZERO));
    assert_eq!(s.prefab_added(10), [50]);
    s.revert_prefab(10);
    s.expand_prefabs(&mut loader(&assets));
    assert_eq!(s.entities.len(), 3);
    assert!(s.prefab_diff(10, &mut loader(&assets)).unwrap().is_clean());
    assert_eq!(
        s.get(10).unwrap().name,
        "Renamed",
        "revert keeps the identity"
    );
}

#[test]
fn children_added_to_an_instance_stay_in_the_scene_file() {
    let assets = assets(&[("crate.prefab.ron", crate_prefab())]);
    let mut s = scene(vec![instance(10, "crate.prefab.ron", Vec3::ZERO)]);
    s.expand_prefabs(&mut loader(&assets));
    s.entities
        .push(mesh(11, "Ribbon", Some(member_id(10, 2)), Vec3::ZERO));
    let saved = s.collapse_prefabs(&mut loader(&assets));
    assert_eq!(saved.entities.len(), 2);
    let mut loaded = SceneFile::parse(&ron::to_string(&saved).unwrap()).unwrap();
    loaded.expand_prefabs(&mut loader(&assets));
    let ribbon = loaded.get(11).unwrap();
    assert_eq!(
        ribbon.parent,
        Some(member_id(10, 2)),
        "the parent id is stable"
    );
    assert!(loaded.world_matrices().contains_key(&11));
}

#[test]
fn apply_writes_changes_into_the_asset_and_other_instances_follow() {
    let mut assets = assets(&[("crate.prefab.ron", crate_prefab())]);
    let mut s = scene(vec![
        instance(10, "crate.prefab.ron", Vec3::ZERO),
        instance(20, "crate.prefab.ron", Vec3::X),
    ]);
    s.expand_prefabs(&mut loader(&assets));
    let lid = member_id(10, 2);
    s.get_mut(lid).unwrap().position.y = 3.0;
    s.get_mut(10).unwrap().layer = 4;
    s.entities.retain(|e| e.id != member_id(10, 3));
    s.entities.push(mesh(60, "Bolt", Some(lid), Vec3::ZERO));

    let new_asset = s.apply_prefab(10, &mut loader(&assets)).unwrap();
    assert_eq!(new_asset.get(2).unwrap().position.y, 3.0);
    assert_eq!(new_asset.get(1).unwrap().layer, 4);
    assert!(new_asset.get(3).is_none(), "the label left the prefab");
    let bolt = new_asset
        .entities
        .iter()
        .find(|e| e.name == "Bolt")
        .unwrap();
    assert_eq!(bolt.parent, Some(2), "added children became nodes");
    assert_eq!(
        new_asset.get(1).unwrap().name,
        "Crate",
        "identity is not applied"
    );

    // What the editor does next: drop the added ones, expand against the new asset.
    let saved = {
        let mut doomed = s.prefab_added(10);
        doomed.sort();
        assert_eq!(doomed, [60]);
        s.collapse_prefabs(&mut loader(&assets))
    };
    assets.insert("crate.prefab.ron".into(), new_asset);
    let mut s = saved;
    s.entities.retain(|e| e.id != 60);
    s.get_mut(10).unwrap().prefab = Some(PrefabInstance::new("crate.prefab.ron"));
    s.expand_prefabs(&mut loader(&assets));
    assert!(s.prefab_diff(10, &mut loader(&assets)).unwrap().is_clean());
    assert_eq!(s.get(member_id(20, 2)).unwrap().position.y, 3.0);
    assert!(s.get(member_id(20, 3)).is_none());
    assert_eq!(s.entities.iter().filter(|e| e.name == "Bolt").count(), 2);
}

#[test]
fn prefabs_nest_and_overrides_reach_inner_nodes() {
    let mut chest = SceneFile {
        entities: vec![mesh(1, "Chest", None, Vec3::ZERO)],
        ..Default::default()
    };
    let mut slot = instance(2, "crate.prefab.ron", Vec3::new(0.0, 0.0, 3.0));
    slot.name = "Slot".into();
    slot.parent = Some(1);
    chest.entities.push(slot);
    let assets = assets(&[
        ("crate.prefab.ron", crate_prefab()),
        ("chest.prefab.ron", chest),
    ]);
    let mut s = scene(vec![instance(10, "chest.prefab.ron", Vec3::X)]);
    let w = s.expand_prefabs(&mut loader(&assets));
    assert!(w.is_empty(), "{w:?}");
    // chest root, the inner crate's root, lid and label.
    assert_eq!(s.entities.len(), 4);
    let slot = member_id(10, 2);
    assert_eq!(s.get(slot).unwrap().parent, Some(10));
    assert_eq!(
        s.get(slot).unwrap().name,
        "Slot",
        "the outer asset names the inner root"
    );
    let lid = member_id(slot, 2);
    assert_eq!(s.get(lid).unwrap().parent, Some(slot));
    assert_eq!(
        s.get(lid).unwrap().prefab_node.as_ref().unwrap().path,
        "2/2"
    );
    let world = s.world_matrices()[&lid].transform_point3(Vec3::ZERO);
    assert!(world.abs_diff_eq(Vec3::new(1.0, 1.0, 3.0), 1e-5), "{world}");

    // An inner node edited on the outer instance is an override at "2/2" ...
    s.get_mut(lid).unwrap().position.y = 7.0;
    let diff = s.prefab_diff(10, &mut loader(&assets)).unwrap();
    assert!(diff.overrides["2/2"].contains_key("position"), "{diff:?}");
    // ... and applying it hands it to the inner instance inside the asset.
    let applied = s.apply_prefab(10, &mut loader(&assets)).unwrap();
    let inner = applied.get(2).unwrap().prefab.as_ref().unwrap();
    assert!(inner.overrides["2"].contains_key("position"), "{inner:?}");
    let mut assets2 = assets.clone();
    assets2.insert("chest.prefab.ron".into(), applied);
    let mut fresh = scene(vec![instance(30, "chest.prefab.ron", Vec3::ZERO)]);
    fresh.expand_prefabs(&mut loader(&assets2));
    assert_eq!(
        fresh
            .get(member_id(member_id(30, 2), 2))
            .unwrap()
            .position
            .y,
        7.0
    );

    // Removing an inner node.
    s.entities.retain(|e| e.id != member_id(slot, 3));
    let diff = s.prefab_diff(10, &mut loader(&assets)).unwrap();
    assert_eq!(diff.removed, ["2/3"]);
}

#[test]
fn cycles_and_missing_files_are_skipped_with_a_warning() {
    // a contains b, b contains a.
    let mut a = SceneFile {
        entities: vec![mesh(1, "A", None, Vec3::ZERO)],
        ..Default::default()
    };
    a.entities.push({
        let mut e = instance(2, "b.prefab.ron", Vec3::ZERO);
        e.parent = Some(1);
        e
    });
    let mut b = SceneFile {
        entities: vec![mesh(1, "B", None, Vec3::ZERO)],
        ..Default::default()
    };
    b.entities.push({
        let mut e = instance(2, "a.prefab.ron", Vec3::ZERO);
        e.parent = Some(1);
        e
    });
    let assets = assets(&[("a.prefab.ron", a), ("b.prefab.ron", b)]);
    let mut s = scene(vec![
        instance(10, "a.prefab.ron", Vec3::ZERO),
        instance(11, "nope.prefab.ron", Vec3::ZERO),
        EntityDesc::new(12, "Plain"),
    ]);
    let warnings = s.expand_prefabs(&mut loader(&assets));
    assert!(
        warnings.iter().any(|w| w.contains("nope.prefab.ron")),
        "{warnings:?}"
    );
    assert!(
        warnings.iter().any(|w| w.contains("contains itself")),
        "{warnings:?}"
    );
    // a, b (a's inner root, empty because it would loop), the missing one stays, Plain.
    assert!(s.get(10).is_some() && s.get(11).is_some() && s.get(12).is_some());
    assert!(s.get(11).unwrap().prefab_node.is_none());
    // Collapsing keeps an instance that never expanded as it was.
    let saved = s.collapse_prefabs(&mut loader(&assets));
    assert_eq!(
        saved.get(11).unwrap().prefab.as_ref().unwrap().asset,
        "nope.prefab.ron"
    );
    // An instance in a prefab that has no objects.
    let empty = assets_with("empty.prefab.ron", SceneFile::default());
    let mut s = scene(vec![instance(1, "empty.prefab.ron", Vec3::ZERO)]);
    assert_eq!(s.expand_prefabs(&mut loader(&empty)).len(), 1);
    assert_eq!(s.entities.len(), 1);
}

fn assets_with(path: &str, scene: SceneFile) -> Assets {
    assets(&[(path, scene)])
}

#[test]
fn unpacking_leaves_plain_objects() {
    let assets = assets(&[("crate.prefab.ron", crate_prefab())]);
    let mut s = scene(vec![instance(10, "crate.prefab.ron", Vec3::ZERO)]);
    s.expand_prefabs(&mut loader(&assets));
    s.unpack_prefab(10);
    assert!(
        s.entities
            .iter()
            .all(|e| e.prefab.is_none() && e.prefab_node.is_none())
    );
    let saved = s.collapse_prefabs(&mut loader(&assets));
    assert_eq!(saved.entities.len(), 3, "the nodes are saved as they are");
}

#[test]
fn every_entity_field_can_be_overridden() {
    use crate::file::*;
    let mut full = EntityDesc::new(1, "Full");
    full.active = false;
    full.tag = "T".into();
    full.layer = 3;
    full.position = Vec3::new(1.0, 2.0, 3.0);
    full.rotation = Vec3::splat(10.0);
    full.scale = Vec3::splat(2.0);
    full.mesh = Some(MeshDesc {
        shape: Shape::Sphere,
        color: [1.0, 0.5, 0.25, 1.0],
    });
    full.camera = Some(CameraDesc::default());
    full.poly = Some(PolyMesh::cube(1.0));
    full.brush = Some(BrushDesc::default());
    full.material = Some("m.mat.ron".into());
    full.model = Some(ModelDesc::new("a.glb"));
    full.light = Some(LightDesc::new(LightKind::Spot));
    full.collider = Some(ColliderDesc::new(ColliderShape::Mesh));
    full.rigidbody = Some(RigidbodyDesc::default());
    full.character_controller = Some(CharacterControllerDesc::default());
    full.audio_source = Some(AudioSourceDesc::default());
    full.audio_listener = true;
    full.scripts = vec![ScriptDesc {
        name: "Spin".into(),
        params: BTreeMap::from([("speed".into(), "2".into())]),
    }];
    full.components.insert("Door".into(), "(open:false)".into());
    let blank = EntityDesc::new(1, "Blank");
    let diff = diff_fields(&blank, &full, false);
    let mut rebuilt = blank.clone();
    for (key, text) in &diff {
        set_field(&mut rebuilt, key, text).unwrap();
    }
    assert_eq!(rebuilt, full);
    // And back: everything removed again.
    let undo = diff_fields(&full, &blank, false);
    for (key, text) in &undo {
        set_field(&mut rebuilt, key, text).unwrap();
    }
    assert_eq!(rebuilt, blank);
    assert!(set_field(&mut rebuilt, "nonsense", "1").is_err());
    assert!(set_field(&mut rebuilt, "layer", "(").is_err());
}
