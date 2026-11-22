use std::fs;
use std::path::Path;

use super::*;

fn manifest(id: &str, requires: &[&str]) -> String {
    let requires: Vec<String> = requires.iter().map(|r| format!("{r:?}")).collect();
    format!(
        r#"(id: "{id}", name: "Pack {id}", version: "1.0.0", category: Misc,
            description: "A test pack.", tags: ["alpha", "beta"], author: "t",
            license: "CC0-1.0", requires: [{}], provides: ["scripts/{id}/a.lua"],
            notes: "Use {id}.")"#,
        requires.join(", ")
    )
}

/// `<root>/<id>/pack.ron` with `scripts/<id>/a.lua`.
fn make_pack(root: &Path, id: &str, requires: &[&str], body: &str) {
    let dir = root.join(id);
    fs::create_dir_all(dir.join(format!("assets/scripts/{id}"))).unwrap();
    fs::write(dir.join("pack.ron"), manifest(id, requires)).unwrap();
    fs::write(dir.join(format!("assets/scripts/{id}/a.lua")), body).unwrap();
}

fn library(root: &Path) -> Library {
    Library::default().with_dirs(&[root.to_path_buf()])
}

#[test]
fn the_embedded_library_is_valid() {
    let lib = Library::embedded();
    assert!(lib.problems().is_empty(), "{:#?}", lib.problems());
    for pack in lib.list() {
        // Every pack must resolve (dependencies exist, no loops).
        lib.resolve(pack.id()).unwrap();
    }
}

#[test]
fn loads_lists_and_searches() {
    let dir = tempfile::tempdir().unwrap();
    make_pack(dir.path(), "one", &[], "return {}");
    make_pack(dir.path(), "two", &[], "return {}");
    let lib = library(dir.path());
    assert!(lib.problems().is_empty(), "{:?}", lib.problems());
    assert_eq!(lib.list().len(), 2);
    assert_eq!(lib.search("", None).len(), 2);
    assert_eq!(lib.search("pack one", None).len(), 1);
    assert_eq!(lib.search("ALPHA", Some(Category::Misc)).len(), 2);
    assert!(lib.search("alpha", Some(Category::Shaders)).is_empty());
    assert!(lib.search("gamma", None).is_empty());
    assert_eq!(lib.get("one").unwrap().headline_prefab(), None);
}

#[test]
fn invalid_packs_are_reported_and_skipped() {
    let dir = tempfile::tempdir().unwrap();
    make_pack(dir.path(), "good", &[], "return {}");
    // id differs from the folder.
    make_pack(dir.path(), "bad-folder", &[], "x");
    fs::write(
        dir.path().join("bad-folder/pack.ron"),
        manifest("other", &[]),
    )
    .unwrap();
    // A forbidden extension, a `..` path is covered in manifest tests.
    make_pack(dir.path(), "bad-ext", &[], "x");
    fs::write(
        dir.path().join("bad-ext/assets/scripts/bad-ext/run.exe"),
        "x",
    )
    .unwrap();
    // provides a file that is missing.
    make_pack(dir.path(), "bad-provides", &[], "x");
    fs::remove_file(
        dir.path()
            .join("bad-provides/assets/scripts/bad-provides/a.lua"),
    )
    .unwrap();
    fs::write(
        dir.path()
            .join("bad-provides/assets/scripts/bad-provides/b.lua"),
        "x",
    )
    .unwrap();
    // Unknown manifest field.
    make_pack(dir.path(), "bad-field", &[], "x");
    fs::write(
        dir.path().join("bad-field/pack.ron"),
        "(id: \"bad-field\", oops: 1)",
    )
    .unwrap();
    // Too big a preview.
    make_pack(dir.path(), "big-preview", &[], "x");
    fs::write(
        dir.path().join("big-preview/preview.png"),
        vec![0u8; MAX_PREVIEW_BYTES + 1],
    )
    .unwrap();

    let lib = library(dir.path());
    assert_eq!(lib.list().len(), 1);
    assert_eq!(lib.problems().len(), 5, "{:#?}", lib.problems());
    let all = lib.problems().join("\n");
    assert!(all.contains("differs from the folder"));
    assert!(all.contains("extension `.exe`"));
    assert!(all.contains("no such file"));
    assert!(all.contains("preview.png"));
}

#[test]
fn duplicate_ids_across_sources_are_refused() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    make_pack(a.path(), "same", &[], "1");
    make_pack(b.path(), "same", &[], "2");
    let lib = Library::default().with_dirs(&[a.path().into(), b.path().into()]);
    assert_eq!(lib.list().len(), 1);
    assert!(lib.problems()[0].contains("taken"));
}

#[test]
fn dependencies_come_first_and_cycles_are_found() {
    let dir = tempfile::tempdir().unwrap();
    make_pack(dir.path(), "base", &[], "b");
    make_pack(dir.path(), "mid", &["base"], "m");
    make_pack(dir.path(), "top", &["mid", "base"], "t");
    make_pack(dir.path(), "loop-a", &["loop-b"], "a");
    make_pack(dir.path(), "loop-b", &["loop-a"], "b");
    make_pack(dir.path(), "lost", &["missing"], "l");
    let lib = library(dir.path());
    let ids: Vec<_> = lib
        .resolve("top")
        .unwrap()
        .iter()
        .map(|p| p.id().to_owned())
        .collect();
    assert_eq!(ids, ["base", "mid", "top"]);
    assert_eq!(
        lib.resolve("loop-a").unwrap_err(),
        LibraryError::Cycle(vec!["loop-a".into(), "loop-b".into(), "loop-a".into()])
    );
    assert_eq!(
        lib.resolve("lost").unwrap_err(),
        LibraryError::MissingDependency {
            pack: "lost".into(),
            needed: "missing".into()
        }
    );
    assert_eq!(
        lib.resolve("zzz").unwrap_err(),
        LibraryError::NotFound("zzz".into())
    );
}

fn project() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn import_copies_files_makes_metas_and_writes_the_lock() {
    let src = tempfile::tempdir().unwrap();
    make_pack(src.path(), "base", &[], "base");
    make_pack(src.path(), "top", &["base"], "top");
    let lib = library(src.path());
    let proj = project();
    let report = lib
        .import("top", proj.path(), &ImportOptions::default())
        .unwrap();
    assert!(report.completed);
    let ids: Vec<_> = report.packs.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["base", "top"], "dependencies first");
    assert_eq!(report.written(), 2);
    assert_eq!(report.notes.len(), 2);
    assert!(report.summary().contains("2 file(s) written"));

    let file = proj.path().join("assets/scripts/top/a.lua");
    assert_eq!(fs::read_to_string(&file).unwrap(), "top");
    // The GUID sidecar exists and belongs to the asset.
    assert!(proj.path().join("assets/scripts/top/a.lua.meta").is_file());

    let lock = Lock::load(proj.path()).unwrap();
    assert_eq!(lock.packs.len(), 2);
    let top = lock.get("top").unwrap();
    assert!(top.explicit && top.version == "1.0.0");
    assert!(!lock.get("base").unwrap().explicit, "pulled in by `top`");
    assert_eq!(top.files[0].path, "scripts/top/a.lua");
    assert_eq!(top.files[0].hash, hash_hex(b"top"));
    assert_eq!(installed(proj.path()).unwrap().len(), 2);
    assert_eq!(
        lib.status("top", proj.path()),
        Status::Installed("1.0.0".into())
    );
    assert_eq!(lib.status("nope", proj.path()), Status::NotInstalled);
}

#[test]
fn importing_twice_is_a_no_op_that_keeps_guids() {
    let src = tempfile::tempdir().unwrap();
    make_pack(src.path(), "one", &[], "x");
    let lib = library(src.path());
    let proj = project();
    lib.import("one", proj.path(), &ImportOptions::default())
        .unwrap();
    let meta = proj.path().join("assets/scripts/one/a.lua.meta");
    let guid = fs::read_to_string(&meta).unwrap();
    let again = lib
        .import("one", proj.path(), &ImportOptions::default())
        .unwrap();
    assert!(again.completed);
    assert_eq!(again.written(), 0);
    assert_eq!(again.packs[0].identical.len(), 1);
    assert_eq!(fs::read_to_string(&meta).unwrap(), guid);
    assert_eq!(Lock::load(proj.path()).unwrap().packs.len(), 1);
}

#[test]
fn conflicts_are_never_overwritten_silently() {
    let src = tempfile::tempdir().unwrap();
    make_pack(src.path(), "one", &[], "pack");
    let lib = library(src.path());
    let proj = project();
    let mine = proj.path().join("assets/scripts/one/a.lua");
    fs::create_dir_all(mine.parent().unwrap()).unwrap();
    fs::write(&mine, "mine").unwrap();

    let plan = lib.plan("one", proj.path()).unwrap();
    assert_eq!(plan.conflicts(), [("one", "scripts/one/a.lua")]);

    // Default: nothing happens.
    let r = lib
        .import("one", proj.path(), &ImportOptions::default())
        .unwrap();
    assert!(!r.completed);
    assert_eq!(r.conflicts.len(), 1);
    assert_eq!(fs::read_to_string(&mine).unwrap(), "mine");
    assert!(
        !Lock::path(proj.path()).exists(),
        "no lock for a refused import"
    );

    // Skip keeps the file.
    let r = lib
        .import(
            "one",
            proj.path(),
            &ImportOptions {
                on_conflict: OnConflict::Skip,
            },
        )
        .unwrap();
    assert!(r.completed);
    assert_eq!(r.packs[0].skipped, ["scripts/one/a.lua"]);
    assert_eq!(fs::read_to_string(&mine).unwrap(), "mine");
    assert!(
        Lock::load(proj.path())
            .unwrap()
            .get("one")
            .unwrap()
            .files
            .is_empty()
    );

    // Keep both writes a suffixed copy.
    let r = lib
        .import(
            "one",
            proj.path(),
            &ImportOptions {
                on_conflict: OnConflict::KeepBoth,
            },
        )
        .unwrap();
    assert_eq!(
        r.packs[0].renamed,
        [(
            "scripts/one/a.lua".to_owned(),
            "scripts/one/a-2.lua".to_owned()
        )]
    );
    assert_eq!(fs::read_to_string(&mine).unwrap(), "mine");
    assert_eq!(
        fs::read_to_string(proj.path().join("assets/scripts/one/a-2.lua")).unwrap(),
        "pack"
    );
    assert!(
        proj.path()
            .join("assets/scripts/one/a-2.lua.meta")
            .is_file()
    );
    let lock = Lock::load(proj.path()).unwrap();
    let f = &lock.get("one").unwrap().files[0];
    assert_eq!(
        (f.path.as_str(), f.from.as_deref()),
        ("scripts/one/a-2.lua", Some("scripts/one/a.lua"))
    );

    // Replace overwrites.
    let r = lib
        .import(
            "one",
            proj.path(),
            &ImportOptions {
                on_conflict: OnConflict::Replace,
            },
        )
        .unwrap();
    assert_eq!(r.packs[0].replaced.len(), 1);
    assert_eq!(fs::read_to_string(&mine).unwrap(), "pack");
}

#[test]
fn suffix_goes_before_compound_extensions() {
    let dir = tempfile::tempdir().unwrap();
    let assets = dir.path();
    assert_eq!(
        free_name(assets, "prefabs/x/Player.prefab.ron"),
        "prefabs/x/Player-2.prefab.ron"
    );
    assert_eq!(free_name(assets, "a.png"), "a-2.png");
    fs::write(assets.join("a-2.png"), "").unwrap();
    assert_eq!(free_name(assets, "a.png"), "a-3.png");
}

#[test]
fn a_broken_lock_is_an_error_not_overwritten() {
    let src = tempfile::tempdir().unwrap();
    make_pack(src.path(), "one", &[], "x");
    let lib = library(src.path());
    let proj = project();
    fs::write(Lock::path(proj.path()), "(garbage").unwrap();
    assert!(matches!(
        lib.import("one", proj.path(), &ImportOptions::default()),
        Err(LibraryError::Lock(_))
    ));
    assert_eq!(
        fs::read_to_string(Lock::path(proj.path())).unwrap(),
        "(garbage"
    );
}

#[test]
fn lint_flags_files_outside_the_namespace() {
    let src = tempfile::tempdir().unwrap();
    make_pack(src.path(), "one", &[], "x");
    fs::write(src.path().join("one/assets/loose.lua"), "x").unwrap();
    let pack = Pack::from_dir(&src.path().join("one")).unwrap();
    assert_eq!(pack.lint().len(), 1);
}
